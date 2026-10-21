// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 2, activities (0.1): a routine runs as an activity with six
//! states, and its limits travel with it.

use std::collections::BTreeSet;

use chrono::Duration;
use serde_json::{Value, json};

use crate::fake::*;
use crate::ledger::{Entry, Source};
use crate::model::*;

impl Fake {
    pub fn open(&mut self, who: &Who, body: &Value) -> Res {
        let app = match (&who.app, opt_s(body, "app")) {
            (_, Some(named)) => named,
            (Some(own), None) => own.clone(),
            (None, None) => return Err(bad("missing `app`")),
        };
        let routine = s(body, "routine")?;
        let context = body.get("context").cloned().unwrap_or(Value::Null);
        let require: BTreeSet<String> = strings(body.get("approval_policy").unwrap_or(&Value::Null), "require").into_iter().collect();
        let audience = body.get("audience").map(|_| strings(body, "audience").into_iter().collect());

        // A routine opening an activity of its own app opens a child of its own.
        let parent = match who.kind {
            CallerKind::Routine if who.app.as_deref() == Some(app.as_str()) => who.activity.clone(),
            _ => opt_s(body, "parent"),
        };
        let principal = match (opt_s(body, "principal"), &who.member) {
            (Some(p), _) => p,
            (None, Some(m)) => m.clone(),
            (None, None) => return Err(bad("name the member the activity acts for in `principal`")),
        };

        let opener = if who.is_shell() { None } else { who.app.clone() };
        let id = self.open_activity(&app, &routine, &principal, audience, context, parent, require, opener, who.source())?;
        let act = &self.activities[&id];
        Ok(json!({
            "activity": id,
            "state": act.state.word(),
            "acts_for": act.acts_for,
            "audience": act.audience,
            "deadline": stamp(act.deadline),
        }))
    }

    /// Opens an activity in Created and draws its child grants from the app's
    /// install grants. Shared by the open call and scheduled runs.
    #[allow(clippy::too_many_arguments)]
    pub fn open_activity(
        &mut self,
        app_id: &str,
        routine: &str,
        principal: &str,
        audience: Option<BTreeSet<String>>,
        context: Value,
        parent: Option<String>,
        require: BTreeSet<String>,
        opener: Option<String>,
        source: Source,
    ) -> Result<String, CallError> {
        let app = self.running_app(app_id)?.clone();
        let name = app.manifest.display_name();
        let decl = app.manifest.routine(routine).cloned().ok_or_else(|| {
            CallError::from(refuse("routine_not_in_manifest", format!("{name} declares no routine called {routine}")).app(app_id))
        })?;

        // One app opens another's routine only when both manifests say so.
        if let Some(opener) = opener.filter(|o| o != app_id) {
            let declared = self.apps.get(&opener).is_some_and(|o| o.manifest.opens.iter().any(|r| r.app == app_id && r.routine == routine))
                && app.manifest.opened_by.contains(&opener);
            if !declared {
                return Err(refuse("open_not_declared", format!("{opener} may open {name}'s {routine} only when its manifest lists it under `opens` and {name}'s lists {opener} under `opened_by`"))
                    .app(app_id)
                    .into());
            }
        }

        // Software is never a principal; Daybook acts only for the owner.
        let Some(member) = self.household.member(principal).cloned() else {
            return Err(refuse("principal_may_not_use_app", format!("{principal} is not a member of the household")).app(app_id).into());
        };
        if app.manifest.kind == AppKind::Core && member.role != Role::Owner {
            return Err(refuse("principal_may_not_use_app", format!("{name} acts only for the owner")).app(app_id).member(principal).into());
        }

        // A default never widens who sees work: the principal alone, unless the app names more.
        let mut audience = audience.unwrap_or_default();
        if let Some(unknown) = audience.iter().find(|m| self.household.member(m).is_none()) {
            return Err(bad(format!("{unknown} in the audience is not a member of the household")));
        }
        audience.insert(principal.into());

        for op in &require {
            if app.manifest.operation(op).is_none() {
                return Err(refuse("operation_not_in_manifest", format!("the approval policy names {op}, which {name} does not declare"))
                    .app(app_id)
                    .operation(op)
                    .into());
            }
        }

        if let Some(key) = &decl.concurrency_key {
            let held = self
                .activities
                .values()
                .find(|a| a.app == app_id && !a.state.is_ending() && a.concurrency_key.as_ref() == Some(key));
            if let Some(holder) = held {
                return Err(refuse("concurrency_key_held", format!("activity {} already holds the concurrency key {key}", holder.id))
                    .app(app_id)
                    .detail(json!({ "key": key, "held_by": holder.id }))
                    .into());
            }
        }

        let mut deadline = self.now + Duration::minutes(decl.timeout_minutes);
        if let Some(pid) = &parent {
            let p = self.activity(pid)?;
            if p.app != app_id || p.state.is_ending() {
                return Err(refuse("activity_not_open", format!("parent activity {pid} is not an open activity of {name}")).app(app_id).into());
            }
            // A child closes no later than its parent.
            deadline = deadline.min(p.deadline);
        }

        let id = self.next_id("A");
        self.activities.insert(
            id.clone(),
            Activity {
                id: id.clone(),
                app: app_id.into(),
                routine: routine.into(),
                state: ActivityState::Created,
                acts_for: principal.into(),
                audience: audience.clone(),
                parent: parent.clone(),
                context: context.clone(),
                require_approval: require,
                opened_at: self.now,
                deadline,
                cost_cents: 0,
                cost_allowance_cents: decl.cost_allowance_cents,
                concurrency_key: decl.concurrency_key.clone(),
                used_grants: BTreeSet::new(),
                loosened: BTreeSet::new(),
            },
        );

        let plain = format!("{name} opened its {routine} activity ({id}) acting for {}.", self.member_name(principal));
        self.journal(
            Entry::new("activity", source, plain)
                .app(app_id)
                .activity(&id)
                .member(principal)
                .detail(json!({ "to": "created", "routine": routine, "parent": parent, "context": context, "audience": audience, "deadline": stamp(deadline) })),
        );

        // Authority never outlives the work that uses it: each activity draws a
        // child grant from every live install grant, expiring with it.
        let install: Vec<Grant> = self
            .grants
            .values()
            .filter(|g| g.state == GrantState::Live && g.holder == Holder::App(app_id.into()))
            .cloned()
            .collect();
        for g in install {
            self.make_grant(Holder::Activity(id.clone()), app_id, &g.operation, &g.object, &g.principal, GrantKind::Standing, None, Some(deadline), Some(g.id.clone()), true);
        }

        Ok(id)
    }

    pub fn read_state(&mut self, who: &Who, body: &Value) -> Res {
        let id = s(body, "activity")?;
        let act = self.activity(&id)?.clone();
        if !self.in_activity_audience(who, &act) {
            return Err(refuse("outside_audience", format!("activity {id} is outside the caller's audience")).activity(&id).into());
        }

        let proposals: Vec<Value> = self
            .proposals
            .values()
            .filter(|p| p.activity == id && p.state == ProposalState::Pending)
            .map(|p| json!({ "proposal": p.id, "operation": p.operation, "object": p.object, "payload_hash": p.payload_hash }))
            .collect();
        let grants: Vec<Value> = self
            .grants
            .values()
            .filter(|g| g.state == GrantState::Live)
            .filter(|g| g.holder == Holder::Activity(id.clone()) || g.holder == Holder::App(act.app.clone()))
            .map(grant_view)
            .collect();
        let unknown: Vec<&str> = self
            .receipts
            .values()
            .filter(|r| r.activity == id && !r.is_definitive())
            .map(|r| r.request_id.as_str())
            .collect();

        Ok(json!({
            "activity": id,
            "app": act.app,
            "routine": act.routine,
            "state": act.state.word(),
            "acts_for": act.acts_for,
            "parent": act.parent,
            "deadline": stamp(act.deadline),
            "open_proposals": proposals,
            "live_grants": grants,
            "unknown_outcomes": unknown,
            "cost_cents": act.cost_cents,
            "cost_allowance_cents": act.cost_allowance_cents,
            "loosened": act.loosened,
        }))
    }

    pub fn cancel(&mut self, who: &Who, body: &Value) -> Res {
        let id = s(body, "activity")?;
        let act = self.activity(&id)?.clone();
        if !self.in_activity_audience(who, &act) {
            return Err(refuse("outside_audience", format!("activity {id} is outside the caller's audience")).activity(&id).into());
        }
        if act.state.is_ending() {
            return Err(refuse("already_closing", format!("activity {id} is already {}", act.state.word())).activity(&id).into());
        }

        let why = format!("cancelled by {}", self.describe(who));
        self.enter_closing(&id, &why, who.source());
        Ok(json!({ "activity": id, "state": "closing" }))
    }

    pub fn close(&mut self, who: &Who, body: &Value) -> Res {
        let id = s(body, "activity")?;
        let act = self.activity(&id)?.clone();
        if !self.in_activity_audience(who, &act) {
            return Err(refuse("outside_audience", format!("activity {id} is outside the caller's audience")).activity(&id).into());
        }
        if act.state == ActivityState::Closed {
            return Err(refuse("already_closed", format!("activity {id} is already closed")).activity(&id).into());
        }

        // Temporary grants must have expired or been used, here and in every open child.
        let mut family = vec![id.clone()];
        family.extend(self.activities.values().filter(|a| a.parent.as_deref() == Some(id.as_str()) && a.state != ActivityState::Closed).map(|a| a.id.clone()));
        let live_temporary: Vec<String> = self
            .grants
            .values()
            .filter(|g| g.state == GrantState::Live && g.kind == GrantKind::OneTime)
            .filter(|g| matches!(&g.holder, Holder::Activity(a) if family.contains(a)))
            .map(|g| g.id.clone())
            .collect();
        if !live_temporary.is_empty() && !act.state.is_ending() {
            return Err(refuse("temporary_grants_live", format!("temporary grants are still live: {}", live_temporary.join(", ")))
                .activity(&id)
                .detail(json!({ "grants": live_temporary }))
                .into());
        }

        let source = who.source();
        self.enter_closing(&id, "it finished", source.clone());
        for child in family.iter().skip(1) {
            self.set_state(child, ActivityState::Closed, "its parent closed", source.clone());
        }
        self.set_state(&id, ActivityState::Closed, "its outputs are kept and its grants have ended", source);

        let unknown: Vec<String> = self
            .receipts
            .values()
            .filter(|r| r.activity == id && !r.is_definitive())
            .map(|r| r.request_id.clone())
            .collect();
        Ok(json!({ "activity": id, "state": "closed", "unknown_outcomes_still_tracked": unknown }))
    }
    /// Watch (G-13): the state changes of an app's activities, each the
    /// journal entry that caused it. Over HTTP the fake answers at once with
    /// what happened after the cursor; pass the returned cursor next time.
    pub fn watch(&mut self, who: &Who, body: &Value) -> Res {
        let app = opt_s(body, "app").or_else(|| who.app.clone());
        let only = strings(body, "activities");
        let mut states = strings(body, "states");
        if states.is_empty() {
            states.push("closed".into());
        }
        let after = body.get("after").and_then(Value::as_u64).unwrap_or(0);

        let events: Vec<Value> = self
            .ledger
            .iter()
            .filter(|e| e.seq > after && e.kind == "activity")
            .filter(|e| e.detail.as_ref().and_then(|d| d["to"].as_str()).is_some_and(|to| states.iter().any(|s| s == to)))
            .filter(|e| {
                let Some(act) = e.activity.as_ref().and_then(|a| self.activities.get(a)) else { return false };
                let linked = who.app.as_ref().is_some_and(|w| self.apps.get(w).is_some_and(|x| x.manifest.read_scopes.iter().any(|s| s.app == act.app)));
                (self.in_activity_audience(who, act) || linked)
                    && app.as_ref().is_none_or(|a| *a == act.app)
                    && (only.is_empty() || only.contains(&act.id))
            })
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
        let cursor = self.ledger.last().map(|e| e.seq).unwrap_or(0);
        Ok(json!({ "events": events, "cursor": cursor }))
    }
}

pub fn grant_view(g: &Grant) -> Value {
    json!({
        "grant": g.id,
        "holder": g.holder.label(),
        "app": g.app,
        "operation": g.operation,
        "object": g.object,
        "principal": g.principal,
        "kind": g.kind,
        "bound_hash": g.bound_hash,
        "expires_at": g.expires_at.map(stamp),
        "state": g.state,
        "derived_from": g.derived_from,
    })
}
