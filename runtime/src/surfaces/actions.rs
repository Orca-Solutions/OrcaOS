// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 5, actions and receipts (0.1). An app never performs a
//! consequential action itself: the enforcement check mints a single-use
//! authorization from a grant or an approval, and the core consumes it,
//! consume-if-live, committing the request identifier before dispatch. The
//! receipt records what the system saw.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::core::authority::{Basis, Consumed, NotLive};
use crate::ledger::{Entry, Source};
use crate::mock::{Status, Submitted};
use crate::model::*;
use crate::runtime::*;

impl Runtime {
    pub fn act(&mut self, who: &Who, body: &Value) -> Res {
        let aid = self.acting_activity(who, body)?;
        let operation = s(body, "operation")?;
        let object = s(body, "object")?;
        let payload = body.get("payload").cloned().unwrap_or(Value::Null);

        match operation.as_str() {
            "mail.send" | "calendar.write" | "lan.print" => {}
            "provider.disclose" => return Err(bad("disclosure to a provider goes through egress.disclose")),
            _ if adapter_of(&operation).is_none_or(|(_, consequential)| !consequential) => {
                // Still checked and journaled, so an undeclared operation is refused, never ignored.
                self.action_base(&aid, &operation, &object)?;
                return Err(bad(format!("{operation} is not a consequential action the runtime dispatches")));
            }
            _ => {}
        }

        let cleared = self.enforce(&aid, &operation, &object, &payload, opt_s(body, "proposal"), opt_s(body, "grant"))?;
        let receipt = self.dispatch(&aid, &operation, &object, &payload, cleared, json!({}));
        Ok(receipt)
    }

    /// The checks before any authority is looked at: the app is on, the
    /// operation is in its manifest, the object is inside the manifest's
    /// pattern, and the activity is open.
    pub fn action_base(&self, aid: &str, operation: &str, object: &str) -> Result<Activity, CallError> {
        let act = self.activity(aid)?.clone();
        let holder = format!("activity:{aid}");
        let r = |code, text: String| refuse(code, text).app(&act.app).activity(aid).holder(&holder).operation(operation).object(object);

        let app = self.running_app(&act.app).map_err(|e| crate::surfaces::install::with_app(e, &act.app))?;
        let name = app.manifest.display_name();
        let Some(decl) = app.manifest.operation(operation) else {
            return Err(r("operation_not_in_manifest", format!("{operation} is not in {name}'s manifest")).into());
        };
        if !object_matches(&decl.object, object) {
            return Err(r("object_outside_manifest", format!("{object} is outside the manifest's {}", decl.object)).into());
        }
        match act.state() {
            ActivityState::Suspended => Err(r("activity_suspended", format!("activity {aid} is suspended until a grant restores its authority")).into()),
            s if s.is_ending() => Err(r("activity_not_open", format!("activity {aid} is {}", s.word())).into()),
            _ => Ok(act),
        }
    }

    /// The enforcement check. Finds the authority for the action: an
    /// approval's authorization where the policy marks the operation, or a
    /// grant, from which it mints one. Then the core consumes it,
    /// consume-if-live, and only a committed consumption comes back.
    pub fn enforce(&mut self, aid: &str, operation: &str, object: &str, payload: &Value, proposal: Option<String>, grant: Option<String>) -> Result<Consumed, CallError> {
        let act = self.action_base(aid, operation, object)?;
        let holder = format!("activity:{aid}");
        let r = |code, text: String| refuse(code, text).app(&act.app).activity(aid).holder(&holder).operation(operation).object(object);
        let payload_hash = hash(payload);

        let auth_id = if self.needs_approval(&act, operation) {
            let Some(pid) = proposal else {
                return Err(r("approval_needed", format!("{operation} needs a member's approval in this activity; propose it first")).into());
            };
            let p = match self.proposals.get(&pid) {
                Some(p) if p.activity == aid && p.operation == operation && p.object == object => p.clone(),
                _ => return Err(r("approval_needed", format!("proposal {pid} is not a proposal for this action")).proposal(pid).into()),
            };
            let auth = p.authorization.as_ref().and_then(|a| self.core.authorization(a));
            let Some(auth) = auth.filter(|a| a.unconsumed && p.state == ProposalState::Approved) else {
                return Err(r("approval_needed", format!("proposal {pid} has no unused approval")).proposal(pid).into());
            };
            if auth.expires_at <= self.now() {
                return Err(r("approval_expired", format!("the approval of {pid} expired")).proposal(pid).into());
            }

            // What the member saw is what gets bound: a changed draft voids the approval.
            if auth.payload_hash != payload_hash {
                let plain = format!("The approval of {pid} was voided: the payload changed after the member approved it. A new proposal is needed.");
                self.journal(
                    Entry::new("approval_voided", Source::service("enforcement check"), plain)
                        .app(&act.app)
                        .activity(aid)
                        .proposal(&pid)
                        .operation(operation)
                        .object(object)
                        .detail(json!({ "approved_hash": auth.payload_hash, "presented_hash": payload_hash })),
                );
                self.core.invalidate(&auth.id, "the payload changed after approval");
                self.proposals.get_mut(&pid).unwrap().state = ProposalState::Voided;
                return Err(r("payload_changed", "the payload changed after approval; a new proposal is needed".into()).proposal(pid).into());
            }
            auth.id
        } else {
            let g = self.check_grant(aid, operation, object, grant)?;
            if let Some(bound) = &g.bound_hash
                && *bound != payload_hash
            {
                return Err(r("work_state_changed", format!("one-time grant {} is bound to a different draft, record version or recipient; propose again", g.id))
                    .grant(&g.id)
                    .detail(json!({ "bound_hash": bound, "presented_hash": payload_hash }))
                    .into());
            }

            let adapter = adapter_of(operation).map(|(a, _)| a).unwrap_or_default();
            let basis = Basis::Grant { grant: g.id.clone(), principal: g.principal.clone() };
            self.core.mint(basis, aid, operation, object, adapter, &payload_hash, None, None)
        };

        // Consume-if-live: the authorization, its grant and the activity are
        // checked live and the consumption committed, in one step, before
        // dispatch.
        self.touch(aid);
        let request_id = format!("{aid}:{operation}:{auth_id}");
        let plain = format!("The enforcement check consumed a single-use authorization for {operation} on {object} as {request_id}, before dispatch.");
        let entry = Entry::new("consumption", Source::service("enforcement check"), plain)
            .app(&act.app)
            .activity(aid)
            .member(&act.acts_for)
            .operation(operation)
            .object(object);
        self.core.consume_if_live(&auth_id, entry).map_err(|e| {
            let refusal = r(e.code(), e.why().to_string());
            match e {
                NotLive::Grant { grant, .. } => refusal.grant(grant).into(),
                _ => refusal.into(),
            }
        })
    }

    /// The adapter dispatches once and writes the receipt.
    pub fn dispatch(&mut self, aid: &str, operation: &str, object: &str, payload: &Value, cleared: Consumed, detail: Value) -> Value {
        let act = self.core.activity(aid).unwrap().clone();
        let handle = object.split_once(':').map(|(_, h)| h.to_string()).unwrap_or_else(|| object.into());
        let rid = cleared.request_id.clone();

        let (outcome, evidence, source) = match operation {
            "mail.send" => {
                let account = self.mock.mail.get(&handle).map(|m| m.account.clone()).unwrap_or_default();
                let source = Source::external(&format!("mail:{account}"), &format!("mail adapter ({handle})"));
                match self.mock.submit_mail(&handle, &rid, payload) {
                    Submitted::Accepted { message_id } => (json!({ "state": "acknowledged", "message_id": message_id }), "acknowledged", source),
                    Submitted::Refused { reason } => (json!({ "state": "failed", "why": "refused", "reason": reason }), "acknowledged", source),
                    Submitted::NoReply => (json!({ "state": "unknown" }), "observed", Source::service(&format!("mail adapter ({handle})"))),
                }
            }
            "calendar.write" => {
                let events = self.mock.calendar.entry(handle.clone()).or_default();
                events.push(json!({ "request_id": rid, "event": payload }));
                (json!({ "state": "acknowledged", "event": format!("evt-{}", events.len()) }), "acknowledged", Source::external(&format!("calendar:{handle}"), "calendar adapter"))
            }
            "lan.print" => {
                let jobs = self.mock.lan.entry(handle.clone()).or_default();
                jobs.push(json!({ "request_id": rid, "job": payload }));
                (json!({ "state": "acknowledged", "job": format!("job-{}", jobs.len()) }), "acknowledged", Source::external(&format!("lan:{handle}"), "LAN adapter"))
            }
            _ => (json!({ "state": "failed", "why": "certified_not_started", "reason": "no adapter" }), "observed", Source::service("enforcement check")),
        };

        let receipt = Receipt {
            request_id: rid.clone(),
            activity: aid.into(),
            app: act.app.clone(),
            member: act.acts_for.clone(),
            operation: operation.into(),
            object: object.into(),
            authority: cleared.authority.clone(),
            outcome: outcome.clone(),
            evidence: evidence.into(),
            detail: detail.clone(),
        };
        self.receipts.insert(rid.clone(), receipt);

        let said = match outcome["state"].as_str() {
            Some("acknowledged") => "the other side acknowledged it".to_string(),
            Some("unknown") => "no answer came back, so the outcome is unknown; it may or may not have left".to_string(),
            _ => format!("it failed: {}", outcome["reason"].as_str().unwrap_or("refused")),
        };
        let authority = match (cleared.authority.get("approval"), cleared.authority.get("grant")) {
            (Some(p), _) => format!("approved by {} ({})", self.member_name(cleared.authority["member"].as_str().unwrap_or_default()), p.as_str().unwrap_or_default()),
            (None, Some(g)) => format!("under grant {}", g.as_str().unwrap_or_default()),
            _ => String::new(),
        };
        let plain = format!(
            "{}'s {operation} on {object} for {}, {authority}: {said}.",
            self.app_name(&act.app),
            self.member_name(&act.acts_for)
        );
        self.journal(
            Entry::new("receipt", source, plain)
                .app(&act.app)
                .activity(aid)
                .member(&act.acts_for)
                .operation(operation)
                .object(object)
                .request_id(&rid)
                .outcome(outcome.clone())
                .evidence(evidence)
                .authority(cleared.authority.clone())
                .maybe(Entry::grant, cleared.grant.clone())
                .detail(detail.clone()),
        );

        json!({ "request_id": rid, "outcome": outcome, "evidence": evidence, "authority": cleared.authority, "detail": detail })
    }

    /// A status check by request identifier. It never replays the action:
    /// a new attempt is a new proposal.
    pub fn reconcile(&mut self, who: &Who, body: &Value) -> Res {
        let rid = s(body, "request_id")?;
        let Some(receipt) = self.receipts.get(&rid).cloned() else {
            return Err(bad(format!("there is no receipt {rid}")));
        };
        let allowed = match who.kind {
            CallerKind::Shell => {
                let m = who.member.as_deref().unwrap_or_default();
                m == receipt.member || self.is_owner(m)
            }
            _ => who.app.as_deref() == Some(receipt.app.as_str()),
        };
        if !allowed {
            return Err(refuse("outside_audience", format!("receipt {rid} is outside the caller's audience")).into());
        }
        if receipt.is_definitive() {
            return Ok(receipt_view(&receipt));
        }

        let handle = receipt.object.split_once(':').map(|(_, h)| h.to_string()).unwrap_or_default();
        let account = self.mock.mail.get(&handle).map(|m| m.account.clone()).unwrap_or_default();
        let status = match receipt.operation.as_str() {
            "mail.send" => self.mock.mail_status(&handle, &rid),
            _ => Status::Unavailable,
        };

        let resolution = match status {
            Status::Accepted { message_id } => Some((json!({ "state": "acknowledged", "message_id": message_id }), "acknowledged")),
            Status::Refused { reason } => Some((json!({ "state": "failed", "why": "refused", "reason": reason }), "acknowledged")),
            Status::NotReceived => Some((json!({ "state": "failed", "why": "certified_not_started" }), "acknowledged")),
            Status::Unavailable => None,
        };
        let source = Source::external(&format!("mail:{account}"), &format!("mail adapter ({handle})"));
        let base = |kind: &str, plain: String| {
            Entry::new(kind, source.clone(), plain)
                .app(&receipt.app)
                .activity(&receipt.activity)
                .member(&receipt.member)
                .operation(&receipt.operation)
                .object(&receipt.object)
                .request_id(&rid)
        };

        let Some((to, evidence)) = resolution else {
            let plain = format!("A status check on {rid} got no answer; the outcome stays unknown. Nothing was re-sent.");
            self.journal(base("status_check", plain).outcome(json!({ "state": "unknown" })).evidence("observed"));
            return Ok(receipt_view(&receipt));
        };

        let outcome = json!({ "state": "reconciled", "to": to, "evidence": evidence });
        let said = match to["state"].as_str() {
            Some("acknowledged") => "the other side had received it",
            _ => "it did not go",
        };
        let plain = format!("A status check reconciled {rid}: {said}. Nothing was re-sent.");
        self.journal(base("reconciliation", plain).outcome(outcome.clone()).evidence(evidence));

        let r = self.receipts.get_mut(&rid).unwrap();
        r.outcome = outcome;
        r.evidence = evidence.into();
        Ok(receipt_view(&r.clone()))
    }

    pub fn read_ledger(&mut self, who: &Who, body: &Value) -> Res {
        let activity = opt_s(body, "activity");
        let app = opt_s(body, "app");
        let member = opt_s(body, "member");
        let object = opt_s(body, "object");
        let since = time(body, "since")?;
        let until = time(body, "until")?;
        let limit = body.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;

        let parse = |at: &str| DateTime::parse_from_rfc3339(at).map(|t| t.with_timezone(&Utc)).ok();
        let entries: Vec<Value> = self
            .ledger()
            .iter()
            .rev()
            .filter(|e| self.ledger_visible(who, e))
            .filter(|e| activity.as_ref().is_none_or(|a| e.activity.as_ref() == Some(a)))
            .filter(|e| app.as_ref().is_none_or(|a| e.app.as_ref() == Some(a)))
            .filter(|e| member.as_ref().is_none_or(|m| e.member.as_ref() == Some(m)))
            .filter(|e| object.as_ref().is_none_or(|o| e.object.as_ref() == Some(o)))
            .filter(|e| since.is_none_or(|s| parse(&e.at).is_some_and(|t| t >= s)))
            .filter(|e| until.is_none_or(|u| parse(&e.at).is_some_and(|t| t <= u)))
            .take(limit)
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        Ok(json!({ "entries": entries }))
    }

    fn ledger_visible(&self, who: &Who, e: &crate::ledger::Entry) -> bool {
        match who.kind {
            CallerKind::Shell => {
                let m = who.member.as_deref().unwrap_or_default();
                self.is_owner(m)
                    || e.member.as_deref() == Some(m)
                    || e.visible_to.as_ref().is_some_and(|v| v.iter().any(|x| x == m))
                    || e.activity.as_ref().and_then(|a| self.core.activity(a)).is_some_and(|a| a.audience.contains(m))
            }
            _ => {
                let app = who.app.as_deref();
                e.app.as_deref() == app || e.detail.as_ref().and_then(|d| d.get("matter_app")).and_then(Value::as_str) == app
            }
        }
    }

    pub fn claim(&mut self, who: &Who, body: &Value) -> Res {
        let aid = self.acting_activity(who, body)?;
        let statement = s(body, "statement")?;
        let act = self.activity(&aid)?.clone();
        if act.state().is_ending() {
            return Err(refuse("activity_not_open", format!("activity {aid} is {}", act.state().word())).activity(&aid).into());
        }

        let plain = format!("{} says: {statement} (a claim, not an observation).", self.describe(who));
        let seq = self.journal(
            Entry::new("claim", who.source(), plain)
                .app(&act.app)
                .activity(&aid)
                .member(&act.acts_for)
                .evidence("claimed")
                .detail(json!({ "statement": statement })),
        );
        Ok(json!({ "seq": seq }))
    }

    pub fn attest(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.paired_member(who, "An attestation")?;
        let aid = s(body, "activity")?;
        let statement = s(body, "statement")?;
        let act = self.activity(&aid)?.clone();

        let plain = format!("{} attests: {statement}", self.member_name(&member));
        let seq = self.journal(
            Entry::new("attestation", Source::principal(&member), plain)
                .app(&act.app)
                .activity(&aid)
                .member(&member)
                .evidence("attested")
                .detail(json!({ "statement": statement })),
        );
        Ok(json!({ "seq": seq }))
    }
}

pub fn receipt_view(r: &Receipt) -> Value {
    json!({
        "request_id": r.request_id,
        "activity": r.activity,
        "app": r.app,
        "member": r.member,
        "operation": r.operation,
        "object": r.object,
        "outcome": r.outcome,
        "evidence": r.evidence,
        "authority": r.authority,
        "detail": r.detail,
    })
}
