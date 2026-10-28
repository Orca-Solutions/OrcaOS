// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The enforcement core: the journal writer, activities and their lifecycle,
//! grants and their revocation, and single-use authorizations.
//!
//! What it holds is private to it. The surfaces read activities and grants
//! through it and change them only through its calls, each of which writes
//! its journal line first and changes state only once that line has
//! committed. That is how the Decision log's ordering rule is kept: the
//! journal writer is the single point of order, a revocation takes effect
//! when its record commits, and consumption is consume-if-live, one step
//! that checks the authorization, the grant and the activity are all live
//! and commits the consumption record before any dispatch.
//!
//! Requests are answered one at a time, and the journal writer and the
//! adapters run in one process on one host, as the Decision log's first
//! implementation assumes. Revocation across hosts is an open question.

pub mod authority;
pub mod journal;
pub mod lifecycle;
pub mod recovery;
#[cfg(test)]
pub mod tests;

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ledger::{Entry, Source};
use crate::model::GrantKind;
use authority::{Authorization, AuthorizationState, AuthorizationView, Basis, Consumed, Grant, GrantState, Holder, NotLive};
use journal::Journal;
use lifecycle::{Activity, ActivityState, IllegalTransition};

/// How long a grant-minted authorization stays good between minting and
/// consumption. Both happen in one call, so this only bounds a stray one.
const GRANT_AUTHORIZATION_LIFETIME_MINUTES: i64 = 5;

/// Names apps in plain-language lines; the core knows apps only by id.
pub type Names<'a> = &'a dyn Fn(&str) -> String;

/// The runtime's clock and the household's time zone, for ledger stamps.
#[derive(Clone, Debug)]
pub struct Clock {
    pub now: DateTime<Utc>,
    pub zone: String,
}

/// A grant change the core does not allow: used, revoked, expired and ended
/// are final, and only a held grant can be confirmed live again.
#[derive(Debug)]
pub struct IllegalGrantChange {
    pub grant: String,
    pub from: GrantState,
    pub to: GrantState,
}

impl std::fmt::Display for IllegalGrantChange {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "grant {} cannot go from {} to {}", self.grant, grant_word(self.from), grant_word(self.to))
    }
}

pub struct Core {
    pub clock: Clock,
    journal: Journal,
    activities: BTreeMap<String, Activity>,
    grants: BTreeMap<String, Grant>,
    authorizations: BTreeMap<String, Authorization>,
    counters: BTreeMap<String, u64>,
}

impl Core {
    pub fn new(journal: Journal, now: DateTime<Utc>) -> Self {
        Self {
            clock: Clock { now, zone: "UTC".into() },
            journal,
            activities: BTreeMap::new(),
            grants: BTreeMap::new(),
            authorizations: BTreeMap::new(),
            counters: BTreeMap::new(),
        }
    }

    pub fn next_id(&mut self, prefix: &str) -> String {
        let n = self.counters.entry(prefix.into()).or_insert(0);
        *n = n.checked_add(1).unwrap_or_else(|| panic!("fail-stop: the {prefix} ids are used up"));
        format!("{prefix}{n}")
    }

    // ---- The journal ----

    /// Stamps a line with the clock and commits it.
    pub fn journal(&mut self, mut entry: Entry) -> u64 {
        entry.at = crate::runtime::stamp(self.clock.now);
        entry.local = crate::schedule::local(&self.clock.zone, self.clock.now);
        self.journal.append(entry)
    }

    pub fn entries(&self) -> &[Entry] {
        self.journal.entries()
    }

    pub fn is_durable(&self) -> bool {
        self.journal.is_durable()
    }

    // ---- Activities ----

    pub fn activity(&self, id: &str) -> Option<&Activity> {
        self.activities.get(id)
    }

    /// Everything about an activity but its state, which only `transition` sets.
    pub fn activity_mut(&mut self, id: &str) -> Option<&mut Activity> {
        self.activities.get_mut(id)
    }

    pub fn activities(&self) -> impl Iterator<Item = &Activity> {
        self.activities.values()
    }

    /// Journals the opening line, then holds the activity, in Created.
    pub fn open_activity(&mut self, activity: Activity, entry: Entry) {
        debug_assert_eq!(activity.state, ActivityState::Created);
        self.journal(entry);
        self.activities.insert(activity.id.clone(), activity);
    }

    /// Moves an activity along the lifecycle and journals the move. Moving to
    /// the state it is already in does nothing; a move the contract does not
    /// name is refused and leaves no line.
    pub fn transition(&mut self, id: &str, to: ActivityState, why: &str, source: Source, names: Names) -> Result<(), IllegalTransition> {
        let act = self.activities.get(id).expect("transition of an activity the core holds");
        let from = act.state;
        if from == to {
            return Ok(());
        }
        if !from.may_become(to) {
            return Err(IllegalTransition { activity: id.into(), from, to });
        }

        let (app, routine, member) = (act.app.clone(), act.routine.clone(), act.acts_for.clone());
        let plain = format!("{}'s {routine} activity ({id}) is now {}: {why}.", names(&app), to.word());
        self.journal(
            Entry::new("activity", source, plain)
                .app(app)
                .activity(id)
                .member(member)
                .detail(json!({ "from": from.word(), "to": to.word(), "why": why })),
        );
        self.activities.get_mut(id).unwrap().state = to;
        Ok(())
    }

    // ---- Grants ----

    pub fn grant(&self, id: &str) -> Option<&Grant> {
        self.grants.get(id)
    }

    pub fn grants(&self) -> impl Iterator<Item = &Grant> {
        self.grants.values()
    }

    /// Journals a new grant, then holds it live.
    pub fn issue_grant(&mut self, grant: Grant, entry: Entry) {
        debug_assert_eq!(grant.state, GrantState::Live);
        self.journal(entry);
        self.grants.insert(grant.id.clone(), grant);
    }

    /// Changes a grant's state after journaling the line that says so.
    pub fn change_grant(&mut self, id: &str, to: GrantState, entry: Entry) -> Result<(), IllegalGrantChange> {
        let from = self.grants[id].state;
        let allowed = matches!(
            (from, to),
            (GrantState::Live, GrantState::Used | GrantState::Revoked | GrantState::Expired | GrantState::Ended | GrantState::Held)
                | (GrantState::Held, GrantState::Live | GrantState::Revoked | GrantState::Ended)
        );
        if !allowed {
            return Err(IllegalGrantChange { grant: id.into(), from, to });
        }

        self.journal(entry);
        self.grants.get_mut(id).unwrap().state = to;
        if to != GrantState::Live {
            self.invalidate_for_grant(id, &format!("its grant is {}", grant_word(to)));
        }
        Ok(())
    }

    /// Whether a grant can confer authority now: it is live, unexpired, and
    /// so is every grant it was drawn from.
    pub fn grant_is_live(&self, id: &str) -> Result<&Grant, NotLive> {
        let Some(grant) = self.grants.get(id) else {
            return Err(NotLive::Grant { code: "no_grant", grant: id.into(), why: format!("there is no grant {id}") });
        };
        let dead = |code, why: String| Err(NotLive::Grant { code, grant: id.into(), why });

        match grant.state {
            GrantState::Live => {}
            GrantState::Revoked => return dead("grant_revoked", format!("grant {id} was revoked")),
            GrantState::Expired => return dead("grant_expired", format!("grant {id} expired")),
            GrantState::Used => return dead("grant_used", format!("one-time grant {id} was already used")),
            GrantState::Ended | GrantState::Held => return dead("grant_ended", format!("grant {id} has ended")),
        }
        if grant.expires_at.is_some_and(|t| t <= self.clock.now) {
            return dead("grant_expired", format!("grant {id} expired"));
        }

        let mut from = grant.derived_from.clone();
        while let Some(pid) = from {
            let parent = &self.grants[&pid];
            if parent.state != GrantState::Live || parent.expires_at.is_some_and(|t| t <= self.clock.now) {
                return dead("grant_revoked", format!("grant {id} was drawn from grant {pid}, which no longer confers authority"));
            }
            from = parent.derived_from.clone();
        }
        Ok(grant)
    }

    /// Records a read made on a grant, then spends the grant if it was one-time.
    pub fn use_for_read(&mut self, id: &str, entry: Entry) -> u64 {
        let seq = self.journal(entry);
        let grant = self.grants.get_mut(id).unwrap();
        if grant.kind == GrantKind::OneTime {
            grant.state = GrantState::Used;
        }
        seq
    }

    /// Revokes a grant and everything drawn from it. Each revocation takes
    /// effect when its line commits; an activity that held one goes
    /// Suspended if it was running. Never refused for the grant's state.
    /// Returns the grants stopped, the named one first.
    pub fn revoke(&mut self, id: &str, source: Source, actor: &str, names: Names) -> Vec<String> {
        let g = self.grants[id].clone();
        if g.state != GrantState::Live && g.state != GrantState::Held {
            let plain = format!("{actor} revoked grant {id}, which had already stopped conferring authority.");
            self.journal(Entry::new("revocation", source, plain).app(&g.app).grant(id).operation(&g.operation).object(&g.object));
            return Vec::new();
        }

        let mut ids = vec![id.to_string()];
        let mut i = 0;
        while i < ids.len() {
            let parent = ids[i].clone();
            ids.extend(
                self.grants
                    .values()
                    .filter(|c| matches!(c.state, GrantState::Live | GrantState::Held) && c.derived_from.as_deref() == Some(parent.as_str()))
                    .map(|c| c.id.clone()),
            );
            i += 1;
        }

        for gid in &ids {
            let g = self.grants[gid].clone();
            let plain = if gid == id {
                format!("{actor} revoked grant {gid} ({} on {}); it confers no authority for future actions, and reads already made stay.", g.operation, g.object)
            } else {
                format!("Grant {gid} stopped with grant {id}, which it was delegated from.")
            };
            let mut entry = Entry::new("revocation", source.clone(), plain)
                .app(&g.app)
                .member(&g.principal)
                .holder(g.holder.label())
                .grant(gid)
                .operation(&g.operation)
                .object(&g.object);
            if let Holder::Activity(a) = &g.holder {
                entry = entry.activity(a);
            }
            self.change_grant(gid, GrantState::Revoked, entry).expect("a live or held grant can be revoked");

            if let Holder::Activity(aid) = &g.holder
                && self.activities.get(aid).is_some_and(|a| a.state == ActivityState::Running)
            {
                let why = format!("grant {gid} it needs was revoked");
                self.transition(aid, ActivityState::Suspended, &why, Source::service("supervisor"), names).expect("running to suspended");
            }
        }
        ids
    }

    /// Ends every live grant the filter picks, each with its own line.
    pub fn end_grants(&mut self, which: impl Fn(&Grant) -> bool, why: &str) {
        let ids: Vec<String> = self.grants.values().filter(|g| g.state == GrantState::Live && which(g)).map(|g| g.id.clone()).collect();
        for gid in ids {
            let g = self.grants[&gid].clone();
            let plain = format!("Grant {gid} ({} on {}) ended: {why}.", g.operation, g.object);
            let entry = Entry::new("grant_ended", Source::service("supervisor"), plain)
                .app(&g.app)
                .member(&g.principal)
                .holder(g.holder.label())
                .grant(&gid)
                .operation(&g.operation)
                .object(&g.object);
            self.change_grant(&gid, GrantState::Ended, entry).expect("a live grant can end");
        }
    }

    /// Grants and authorizations whose time has run out at the clock's now.
    pub fn expire(&mut self) {
        let now = self.clock.now;
        let ids: Vec<String> = self
            .grants
            .values()
            .filter(|g| g.state == GrantState::Live && g.expires_at.is_some_and(|t| t <= now))
            .map(|g| g.id.clone())
            .collect();
        for gid in ids {
            let g = self.grants[&gid].clone();
            let plain = format!("Grant {gid} ({} on {}) expired.", g.operation, g.object);
            let entry = Entry::new("grant_ended", Source::service("supervisor"), plain)
                .app(&g.app)
                .member(&g.principal)
                .holder(g.holder.label())
                .grant(&gid)
                .operation(&g.operation)
                .object(&g.object);
            self.change_grant(&gid, GrantState::Expired, entry).expect("a live grant can expire");
        }

        for a in self.authorizations.values_mut() {
            if a.state == AuthorizationState::Unconsumed && a.expires_at <= now {
                a.state = AuthorizationState::Invalidated { reason: "it expired".into() };
            }
        }
    }

    /// The next time a live grant expires.
    pub fn next_grant_expiry(&self) -> Option<DateTime<Utc>> {
        self.grants.values().filter(|g| g.state == GrantState::Live).filter_map(|g| g.expires_at).filter(|t| *t > self.clock.now).min()
    }

    // ---- Authorizations ----

    /// Mints a single-use authorization for one consequential action. When a
    /// line is given (the approval path's), it commits first.
    #[allow(clippy::too_many_arguments)]
    pub fn mint(&mut self, basis: Basis, activity: &str, operation: &str, object: &str, adapter: &str, payload_hash: &str, expires_at: Option<DateTime<Utc>>, entry: Option<Entry>) -> String {
        let id = self.next_id("auth-");
        let mut expires_at = expires_at.unwrap_or(self.clock.now + Duration::minutes(GRANT_AUTHORIZATION_LIFETIME_MINUTES));
        if let Some(t) = basis.grant().and_then(|g| self.grants.get(g)).and_then(|g| g.expires_at) {
            expires_at = expires_at.min(t);
        }
        if let Some(mut entry) = entry {
            let mut detail = entry.detail.take().unwrap_or_else(|| json!({}));
            detail["authorization"] = json!(id);
            entry.detail = Some(detail);
            self.journal(entry);
        }

        self.authorizations.insert(
            id.clone(),
            Authorization {
                id: id.clone(),
                activity: activity.into(),
                operation: operation.into(),
                object: object.into(),
                adapter: adapter.into(),
                payload_hash: payload_hash.into(),
                basis,
                expires_at,
                state: AuthorizationState::Unconsumed,
            },
        );
        id
    }

    pub fn authorization(&self, id: &str) -> Option<AuthorizationView> {
        self.authorizations.get(id).map(Authorization::view)
    }

    pub fn invalidate(&mut self, id: &str, why: &str) {
        self.invalidate_where(|a| a.id == id, why);
    }

    pub fn invalidate_for_grant(&mut self, grant: &str, why: &str) {
        self.invalidate_where(|a| a.basis.grant() == Some(grant), why);
    }

    pub fn invalidate_for_proposal(&mut self, proposal: &str, why: &str) {
        self.invalidate_where(|a| a.basis.proposal() == Some(proposal), why);
    }

    fn invalidate_where(&mut self, which: impl Fn(&Authorization) -> bool, why: &str) {
        for a in self.authorizations.values_mut() {
            if a.state == AuthorizationState::Unconsumed && which(a) {
                a.state = AuthorizationState::Invalidated { reason: why.into() };
            }
        }
    }

    /// Consume-if-live: in one step, checks that the authorization is unused
    /// and unexpired, that the grant it was minted from (and every grant that
    /// one was drawn from) is live, and that its activity is open, then
    /// commits the consumption record with a fresh request identifier. Only
    /// after the record commits is the authorization marked consumed, and
    /// only then can the adapter dispatch, with the `Consumed` returned here.
    ///
    /// `entry` is the consumption line as the caller words it; the core sets
    /// its request identifier, authority and grant.
    pub fn consume_if_live(&mut self, id: &str, entry: Entry) -> Result<Consumed, NotLive> {
        let now = self.clock.now;
        let Some(auth) = self.authorizations.get(id) else {
            return Err(NotLive::Authorization { code: "approval_needed", why: format!("there is no authorization {id}") });
        };

        let approved = auth.basis.proposal().is_some();
        match &auth.state {
            AuthorizationState::Unconsumed => {}
            AuthorizationState::Consumed { request_id } => {
                return Err(NotLive::Authorization { code: "grant_used", why: format!("authorization {id} was already consumed as {request_id}") });
            }
            AuthorizationState::Invalidated { reason } => {
                let code = if approved { "approval_needed" } else { "grant_ended" };
                return Err(NotLive::Authorization { code, why: format!("authorization {id} no longer holds: {reason}") });
            }
        }
        if auth.expires_at <= now {
            let code = if approved { "approval_expired" } else { "grant_expired" };
            return Err(NotLive::Authorization { code, why: format!("authorization {id} expired") });
        }

        if let Some(grant) = auth.basis.grant() {
            self.grant_is_live(grant)?;
        }

        let Some(act) = self.activities.get(&auth.activity) else {
            return Err(NotLive::Activity { code: "unknown_activity", why: format!("there is no activity {}", auth.activity) });
        };
        match act.state {
            s if s.is_live() => {}
            ActivityState::Suspended => {
                return Err(NotLive::Activity { code: "activity_suspended", why: format!("activity {} is suspended until a grant restores its authority", act.id) });
            }
            s => return Err(NotLive::Activity { code: "activity_not_open", why: format!("activity {} is {}", act.id, s.word()) }),
        }

        let request_id = format!("{}:{}:{id}", auth.activity, auth.operation);
        let authority = auth.basis.authority();
        let grant = auth.basis.grant().map(str::to_string);
        let (activity, adapter, payload_hash) = (auth.activity.clone(), auth.adapter.clone(), auth.payload_hash.clone());
        let (operation, object) = (auth.operation.clone(), auth.object.clone());

        // The line names what the authorization covers, whatever the caller wrote.
        let mut entry = entry
            .activity(&auth.activity)
            .operation(&auth.operation)
            .object(&auth.object)
            .request_id(&request_id)
            .authority(authority.clone())
            .maybe(Entry::grant, grant.clone());
        let mut detail = entry.detail.take().unwrap_or_else(|| json!({}));
        detail["payload_hash"] = json!(payload_hash);
        detail["adapter"] = json!(adapter);
        detail["authorization"] = json!(id);
        entry.detail = Some(detail);
        self.journal(entry);

        // Committed: now, and only now, the authorization is spent.
        self.authorizations.get_mut(id).unwrap().state = AuthorizationState::Consumed { request_id: request_id.clone() };
        if let Some(gid) = &grant {
            let g = self.grants.get_mut(gid).unwrap();
            if g.kind == GrantKind::OneTime {
                g.state = GrantState::Used;
            }
            self.activities.get_mut(&activity).unwrap().used_grants.insert(gid.clone());
        }

        Ok(Consumed::new(request_id, authority, grant, operation, object, payload_hash))
    }

    // ---- The factory reset ----

    /// Wipe (G-9): every activity, grant, authorization and journal line goes.
    pub fn wipe(&mut self) {
        self.activities.clear();
        self.grants.clear();
        self.authorizations.clear();
        self.journal.wipe(&self.counters);
    }
}

fn grant_word(state: GrantState) -> &'static str {
    match state {
        GrantState::Live => "live",
        GrantState::Used => "used",
        GrantState::Revoked => "revoked",
        GrantState::Expired => "expired",
        GrantState::Ended => "ended",
        GrantState::Held => "held",
    }
}

/// The hash a payload, a draft, a work state or a journal line is bound by:
/// SHA-256 over its JSON with object keys in sorted order, so the order a
/// client sends keys in never changes it.
pub fn hash(v: &Value) -> String {
    let canonical = serde_json::to_string(&sorted(v)).unwrap_or_default();
    format!("sha256:{}", hex::encode(Sha256::digest(canonical.as_bytes())))
}

fn sorted(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&map[k]))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}
