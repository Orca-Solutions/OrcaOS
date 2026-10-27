// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 4, proposals and approvals (0.3). A proposal has no power. Only a
//! member's yes, given through the shell on a paired device, turns it into a
//! single-use authorization, and that authorization goes to the adapter,
//! never to the app.

use chrono::Duration;
use serde_json::{Value, json};

use crate::fake::*;
use crate::ledger::{Entry, Source};
use crate::model::*;

impl Fake {
    pub fn propose(&mut self, who: &Who, body: &Value) -> Res {
        let aid = self.acting_activity(who, body)?;
        let operation = s(body, "operation")?;
        let object = s(body, "object")?;
        let payload = body.get("payload").cloned().unwrap_or(Value::Null);
        let target = body.get("target").cloned().unwrap_or(Value::Null);

        let act = self.activity(&aid)?.clone();
        let app = self.running_app(&act.app)?.clone();
        let name = app.manifest.display_name();
        let r = |code, text: String| refuse(code, text).app(&act.app).activity(&aid).holder(format!("activity:{aid}")).operation(&operation).object(&object);

        if act.state.is_ending() {
            return Err(r("activity_not_open", format!("activity {aid} is {}", act.state.word())).into());
        }
        let Some(decl) = app.manifest.operation(&operation) else {
            return Err(r("operation_not_in_manifest", format!("{operation} is not in {name}'s manifest")).into());
        };
        if !object_matches(&decl.object, &object) {
            return Err(r("no_grant_could_cover", format!("no grant could ever cover {object}: the manifest declares {}", decl.object)).into());
        }

        // Proposing on a matter another app opened: the matter must be in the
        // proposer's audience, and the proposal names both apps.
        let (matter, matter_app) = match opt_s(body, "matter") {
            None => (None, None),
            Some(mid) => {
                let m = self.activity(&mid)?.clone();
                if m.app != act.app && !app.manifest.read_scopes.iter().any(|sc| sc.app == m.app) {
                    return Err(r("outside_audience", format!("matter {mid} belongs to {}, which is outside {name}'s read scope", self.app_name(&m.app))).into());
                }
                (Some(mid), Some(m.app))
            }
        };

        let pid = self.next_id("P");
        let payload_hash = hash(&payload);
        let needs = self.needs_approval(&act, &operation);
        self.proposals.insert(
            pid.clone(),
            Proposal {
                id: pid.clone(),
                activity: aid.clone(),
                app: act.app.clone(),
                matter_app: matter_app.clone(),
                matter: matter.clone(),
                operation: operation.clone(),
                object: object.clone(),
                target: target.clone(),
                payload,
                payload_hash: payload_hash.clone(),
                state: ProposalState::Pending,
                created_at: self.now,
                decided_by: None,
                authorization: None,
            },
        );

        self.touch(&aid);
        let on_matter = match &matter_app {
            Some(m) if *m != act.app => format!(" on {}'s matter {}", self.app_name(m), matter.clone().unwrap_or_default()),
            _ => String::new(),
        };
        let plain = format!("{name} proposes {operation} on {object}{on_matter}; it has no power until a member approves it.");
        self.journal(
            Entry::new("proposal", who.source(), plain)
                .app(&act.app)
                .activity(&aid)
                .member(&act.acts_for)
                .proposal(&pid)
                .operation(&operation)
                .object(&object)
                .evidence("claimed")
                .detail(json!({ "payload_hash": payload_hash, "target": target, "matter": matter, "matter_app": matter_app, "needs_approval": needs })),
        );
        if needs {
            self.set_state(&aid, ActivityState::Waiting, &format!("proposal {pid} waits on a member"), Source::service("approval path"));
        }

        let approvers = self.approvers_for(&self.activities[&aid].clone(), &operation);
        Ok(json!({ "proposal": pid, "payload_hash": payload_hash, "needs_approval": needs, "approvers": approvers }))
    }

    /// What the shell shows on the paired device, and binds an approval to.
    pub fn show(&mut self, who: &Who, body: &Value) -> Res {
        self.shell_member(who, "Showing a proposal")?;
        let pid = s(body, "proposal")?;
        let p = self.proposals.get(&pid).ok_or_else(|| bad(format!("there is no proposal {pid}")))?;
        Ok(json!({
            "proposal": p.id,
            "app": p.app,
            "matter_app": p.matter_app,
            "proposer_name": self.app_name(&p.app),
            "operation": p.operation,
            "object": p.object,
            "target": p.target,
            "payload": p.payload,
            "payload_hash": p.payload_hash,
            "state": p.state,
        }))
    }

    pub fn approve(&mut self, who: &Who, body: &Value) -> Res {
        let pid = s(body, "proposal")?;
        let r = |code, text: String| refuse(code, text).proposal(&pid);

        let member = match self.paired_member(who, "An approval") {
            Ok(m) => m,
            Err(CallError::Refused(e)) => return Err((*e).proposal(&pid).into()),
            Err(e) => return Err(e),
        };
        let Some(p) = self.proposals.get(&pid).cloned() else {
            return Err(bad(format!("there is no proposal {pid}")));
        };
        let r = |code, text: String| r(code, text).app(&p.app).activity(&p.activity).operation(&p.operation).object(&p.object);

        if p.state != ProposalState::Pending {
            return Err(r("proposal_not_pending", format!("proposal {pid} is {:?}", p.state).to_lowercase()).into());
        }
        let act = self.activities[&p.activity].clone();
        if !self.approvers_for(&act, &p.operation).contains(&member) {
            return Err(r("not_allowed_to_approve", format!("{} may not approve {}", self.member_name(&member), p.operation)).member(&member).into());
        }
        let shown = s(body, "payload_hash")?;
        if shown != p.payload_hash {
            return Err(r("payload_changed", "what was shown is not what was proposed; the payload changed since it was shown".into()).member(&member).into());
        }

        let auth_id = self.next_id("auth-");
        let expires_at = (self.now + Duration::hours(24)).min(act.deadline);
        self.authorizations.insert(
            auth_id.clone(),
            Authorization {
                id: auth_id.clone(),
                activity: p.activity.clone(),
                operation: p.operation.clone(),
                object: p.object.clone(),
                payload_hash: p.payload_hash.clone(),
                grant: None,
                proposal: Some(pid.clone()),
                expires_at,
                state: AuthorizationState::Unconsumed,
            },
        );
        {
            let p = self.proposals.get_mut(&pid).unwrap();
            p.state = ProposalState::Approved;
            p.decided_by = Some(member.clone());
            p.authorization = Some(auth_id.clone());
        }

        let plain = format!(
            "{} approved {}'s proposal {pid}: {} on {}, exactly as shown.",
            self.member_name(&member),
            self.app_name(&p.app),
            p.operation,
            p.object
        );
        self.journal(
            Entry::new("approval", Source::principal(&member), plain)
                .app(&p.app)
                .activity(&p.activity)
                .member(&member)
                .proposal(&pid)
                .operation(&p.operation)
                .object(&p.object)
                .detail(json!({ "payload_hash": p.payload_hash, "device": who.device })),
        );
        let plain = format!("A single-use authorization for {pid}, bound to its payload, went to the adapter for {}, not to the app.", p.object);
        self.journal(
            Entry::new("authorization", Source::service("enforcement core"), plain)
                .app(&p.app)
                .activity(&p.activity)
                .member(&member)
                .proposal(&pid)
                .operation(&p.operation)
                .object(&p.object)
                .authority(json!({ "approval": pid, "member": member }))
                .detail(json!({ "payload_hash": p.payload_hash, "expires_at": stamp(expires_at) })),
        );

        self.resume_if_answered(&p.activity);
        Ok(json!({ "proposal": pid, "state": "approved" }))
    }

    pub fn decline(&mut self, who: &Who, body: &Value) -> Res {
        let pid = s(body, "proposal")?;
        let member = self.paired_member(who, "Declining")?;
        let Some(p) = self.proposals.get(&pid).cloned() else {
            return Err(bad(format!("there is no proposal {pid}")));
        };
        if p.state != ProposalState::Pending {
            return Err(refuse("proposal_not_pending", format!("proposal {pid} is not pending")).proposal(&pid).into());
        }

        let p = {
            let p = self.proposals.get_mut(&pid).unwrap();
            p.state = ProposalState::Declined;
            p.decided_by = Some(member.clone());
            p.clone()
        };
        let plain = format!("{} declined {}'s proposal {pid} ({} on {}).", self.member_name(&member), self.app_name(&p.app), p.operation, p.object);
        self.journal(
            Entry::new("decline", Source::principal(&member), plain)
                .app(&p.app)
                .activity(&p.activity)
                .member(&member)
                .proposal(&pid)
                .operation(&p.operation)
                .object(&p.object),
        );
        self.resume_if_answered(&p.activity);
        Ok(json!({ "proposal": pid, "state": "declined" }))
    }

    pub fn read_policy(&mut self, who: &Who, body: &Value) -> Res {
        let aid = match &who.activity {
            Some(a) => a.clone(),
            None => s(body, "activity")?,
        };
        let act = self.activity(&aid)?.clone();
        let manifest = &self.apps[&act.app].manifest;
        let rules: Vec<Value> = manifest
            .operations
            .iter()
            .map(|o| {
                json!({
                    "operation": o.operation,
                    "needs_approval": self.needs_approval(&act, &o.operation),
                    "approvers": self.approvers_for(&act, &o.operation),
                })
            })
            .collect();
        Ok(json!({ "activity": aid, "operations": rules, "household_rules": self.household.approval_rules }))
    }

    pub fn list_pending(&mut self, who: &Who, body: &Value) -> Res {
        let member = s(body, "member")?;
        let allowed = match who.kind {
            CallerKind::Shell | CallerKind::App => who.member.as_deref() == Some(member.as_str()),
            // A routine whose activity acts for that member has them in its read scope.
            CallerKind::Routine => who.member.as_deref() == Some(member.as_str()),
        };
        if !allowed {
            return Err(refuse("outside_audience", format!("{member}'s pending approvals are outside the caller's audience")).member(&member).into());
        }

        let mut pending: Vec<&Proposal> = self.proposals.values().filter(|p| p.state == ProposalState::Pending).collect();
        pending.sort_by_key(|p| p.created_at);
        let items: Vec<Value> = pending
            .into_iter()
            .filter(|p| self.approvers_for(&self.activities[&p.activity], &p.operation).contains(&member))
            .map(|p| {
                json!({
                    "proposal": p.id,
                    "app": p.app,
                    "matter_app": p.matter_app,
                    "activity": p.activity,
                    "operation": p.operation,
                    "object": p.object,
                    "age_minutes": (self.now - p.created_at).num_minutes(),
                })
            })
            .collect();
        Ok(json!({ "member": member, "count": items.len(), "pending": items }))
    }

    /// Sets a household approval rule (G-15). Owner only, through the shell.
    pub fn set_rule(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Setting an approval rule")?;
        if !self.is_owner(&member) {
            return Err(refuse("not_owner", "only the owner sets approval rules").into());
        }
        let rule: ApprovalRule = serde_json::from_value(body.get("rule").cloned().unwrap_or_default())
            .map_err(|e| bad(format!("the rule could not be read: {e}")))?;

        self.household.approval_rules.retain(|r| !(r.operation == rule.operation && r.when_audience == rule.when_audience));
        self.household.approval_rules.push(rule.clone());

        let plain = format!("{} set who may approve {}.", self.member_name(&member), rule.operation);
        self.journal(Entry::new("approval_rule", Source::principal(&member), plain).member(&member).operation(&rule.operation).detail(json!(rule)));
        Ok(json!({ "rules": self.household.approval_rules }))
    }

    /// Who may approve an operation in an activity: a household rule that
    /// matches the record in context wins; otherwise each member's own list.
    pub fn approvers_for(&self, act: &Activity, operation: &str) -> Vec<String> {
        let record_audience = act
            .context
            .get("record")
            .and_then(Value::as_str)
            .and_then(|id| self.records.get(id))
            .map(|r| r.audience.clone());

        let rule = self
            .household
            .approval_rules
            .iter()
            .filter(|r| r.operation == operation)
            .find(|r| r.when_audience.is_none() || r.when_audience == record_audience);

        let Some(rule) = rule else {
            return self.approvers(operation);
        };
        let members: Vec<String> = match &rule.approvers {
            Approvers::RecordAudience(_) => record_audience.map(|a| self.audience_members(&a).into_iter().collect()).unwrap_or_default(),
            Approvers::Members { members } => members.clone(),
            Approvers::Role { role } => self.household.members.iter().filter(|m| m.role == *role).map(|m| m.id.clone()).collect(),
        };
        members.into_iter().filter(|m| self.household.member(m).is_some_and(|x| x.role != Role::Child)).collect()
    }

    fn resume_if_answered(&mut self, aid: &str) {
        let act = self.activities[aid].clone();
        let waiting = self
            .proposals
            .values()
            .any(|p| p.activity == aid && p.state == ProposalState::Pending && self.needs_approval(&act, &p.operation));
        if act.state == ActivityState::Waiting && !waiting {
            self.set_state(aid, ActivityState::Running, "its proposals were answered", Source::service("approval path"));
        }
    }
}
