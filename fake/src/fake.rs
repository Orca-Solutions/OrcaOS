// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The fake's state and the call path every surface shares: who is calling,
//! which surface version they asked for, and the rule that a refusal is
//! journaled, never silent.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ledger::{Entry, Source};
use crate::mock::Mock;
use crate::model::*;
use crate::schedule;

// ---- Callers ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallerKind {
    /// An app outside the runtime, in a member's session.
    App,
    /// A routine running inside an activity.
    Routine,
    /// A member, through the shell on a paired device.
    Shell,
}

#[derive(Clone, Debug)]
pub struct Who {
    pub kind: CallerKind,
    pub app: Option<String>,
    pub member: Option<String>,
    pub activity: Option<String>,
    pub device: Option<String>,
    pub label: String,
}

impl Who {
    pub fn source(&self) -> Source {
        match self.kind {
            CallerKind::Shell => Source::principal(self.member.as_deref().unwrap_or_default()),
            _ => Source::participant(&self.label),
        }
    }

    pub fn is_shell(&self) -> bool {
        self.kind == CallerKind::Shell
    }
}

// ---- Refusals ----

#[derive(Clone, Debug, Default)]
pub struct Refusal {
    pub code: &'static str,
    pub text: String,
    pub app: Option<String>,
    pub activity: Option<String>,
    pub member: Option<String>,
    pub holder: Option<String>,
    pub operation: Option<String>,
    pub object: Option<String>,
    pub grant: Option<String>,
    pub proposal: Option<String>,
    pub detail: Option<Value>,
    pub seq: Option<u64>,
}

macro_rules! setter {
    ($($name:ident),*) => {
        $(pub fn $name(mut self, v: impl Into<String>) -> Self {
            self.$name = Some(v.into());
            self
        })*
    };
}

impl Refusal {
    setter!(app, activity, member, holder, operation, object, grant, proposal);

    pub fn detail(mut self, v: Value) -> Self {
        self.detail = Some(v);
        self
    }
}

pub fn refuse(code: &'static str, text: impl Into<String>) -> Refusal {
    Refusal {
        code,
        text: text.into(),
        ..Default::default()
    }
}

#[derive(Debug)]
pub enum CallError {
    /// A refusal: journaled, with a stable code.
    Refused(Box<Refusal>),
    /// A request the fake could not read. Not a refusal; not journaled.
    Bad(String),
}

impl From<Refusal> for CallError {
    fn from(r: Refusal) -> Self {
        Self::Refused(Box::new(r))
    }
}

pub type Res = Result<Value, CallError>;

pub fn bad(text: impl Into<String>) -> CallError {
    CallError::Bad(text.into())
}

// ---- Body helpers ----

pub fn s(body: &Value, key: &str) -> Result<String, CallError> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| bad(format!("missing text field `{key}`")))
}

pub fn opt_s(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_string)
}

pub fn strings(body: &Value, key: &str) -> Vec<String> {
    body.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

pub fn time(body: &Value, key: &str) -> Result<Option<DateTime<Utc>>, CallError> {
    match opt_s(body, key) {
        None => Ok(None),
        Some(t) => DateTime::parse_from_rfc3339(&t)
            .map(|t| Some(t.with_timezone(&Utc)))
            .map_err(|_| bad(format!("`{key}` is not an RFC 3339 time"))),
    }
}

/// The hash a payload, a draft or a work state is bound by: SHA-256 over its
/// JSON with object keys in sorted order, so the order a client sends keys
/// in never changes it.
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

fn lowercase_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---- The fake ----

pub struct Fake {
    pub now: DateTime<Utc>,
    pub household: Household,
    pub connections: BTreeMap<String, Connection>,
    pub apps: BTreeMap<String, App>,
    pub activities: BTreeMap<String, Activity>,
    pub grants: BTreeMap<String, Grant>,
    pub proposals: BTreeMap<String, Proposal>,
    pub authorizations: BTreeMap<String, Authorization>,
    pub receipts: BTreeMap<String, Receipt>,
    pub records: BTreeMap<String, Record>,
    pub schedules: BTreeMap<String, Schedule>,
    pub notifications: Vec<Notification>,
    pub contributions: Vec<Contribution>,
    /// (app, member): the member let that app's runs read records in their audience.
    pub app_read_allows: BTreeSet<(String, String)>,
    pub ledger: Vec<Entry>,
    pub mock: Mock,
    counters: BTreeMap<String, u64>,
}

impl Default for Fake {
    fn default() -> Self {
        Self::new()
    }
}

impl Fake {
    pub fn new() -> Self {
        Self {
            now: DateTime::parse_from_rfc3339("2026-11-02T12:00:00Z").unwrap().with_timezone(&Utc),
            household: Household {
                zone: "UTC".into(),
                ..Default::default()
            },
            connections: BTreeMap::new(),
            apps: BTreeMap::new(),
            activities: BTreeMap::new(),
            grants: BTreeMap::new(),
            proposals: BTreeMap::new(),
            authorizations: BTreeMap::new(),
            receipts: BTreeMap::new(),
            records: BTreeMap::new(),
            schedules: BTreeMap::new(),
            notifications: Vec::new(),
            contributions: Vec::new(),
            app_read_allows: BTreeSet::new(),
            ledger: Vec::new(),
            mock: Mock::default(),
            counters: BTreeMap::new(),
        }
    }

    pub fn next_id(&mut self, prefix: &str) -> String {
        let n = self.counters.entry(prefix.into()).or_insert(0);
        *n += 1;
        format!("{prefix}{n}")
    }

    pub fn journal(&mut self, mut entry: Entry) -> u64 {
        entry.seq = self.ledger.len() as u64 + 1;
        entry.at = stamp(self.now);
        entry.local = schedule::local(&self.household.zone, self.now);
        let seq = entry.seq;
        self.ledger.push(entry);
        seq
    }

    // ---- The call path ----

    /// Answers one call on one surface. Every refusal is journaled before it
    /// is returned.
    pub fn call(&mut self, surface: &str, version: &str, name: &str, body: &Value) -> Res {
        let who = match self.resolve(body.get("caller")) {
            Ok(who) => who,
            Err(CallError::Refused(r)) => return Err(self.refused(None, surface, name, *r)),
            Err(e) => return Err(e),
        };

        let result = self
            .check_version(surface, version)
            .and_then(|_| self.route(&who, surface, name, body));

        match result {
            Err(CallError::Refused(r)) => Err(self.refused(Some(&who), surface, name, *r)),
            other => other,
        }
    }

    fn route(&mut self, who: &Who, surface: &str, name: &str, body: &Value) -> Res {
        match (surface, name) {
            ("install", "install") => self.install(who, body),
            ("install", "switch_on") => self.switch_on(who, body),
            ("install", "switch_off") => self.switch_off(who, body),
            ("install", "uninstall") => self.uninstall(who, body),
            ("install", "read_app") => self.read_app(who, body),

            ("activities", "open") => self.open(who, body),
            ("activities", "read_state") => self.read_state(who, body),
            ("activities", "cancel") => self.cancel(who, body),
            ("activities", "close") => self.close(who, body),
            ("activities", "watch") => self.watch(who, body),

            ("grants", "add_connection") => self.add_connection(who, body),
            ("grants", "grant") => self.grant(who, body),
            ("grants", "revoke") => self.revoke(who, body),
            ("grants", "list") => self.list_grants(who, body),
            ("grants", "use") => self.use_grant(who, body),

            ("proposals", "propose") => self.propose(who, body),
            ("proposals", "show") => self.show(who, body),
            ("proposals", "approve") => self.approve(who, body),
            ("proposals", "decline") => self.decline(who, body),
            ("proposals", "read_policy") => self.read_policy(who, body),
            ("proposals", "list_pending") => self.list_pending(who, body),
            ("proposals", "set_rule") => self.set_rule(who, body),

            ("ledger", "act") => self.act(who, body),
            ("ledger", "reconcile") => self.reconcile(who, body),
            ("ledger", "read") => self.read_ledger(who, body),
            ("ledger", "claim") => self.claim(who, body),
            ("ledger", "attest") => self.attest(who, body),

            ("records", "write") => self.write_record(who, body),
            ("records", "read") => self.read_records(who, body),
            ("records", "promote") => self.promote(who, body),
            ("records", "delete") => self.delete_record(who, body),
            ("records", "allow_app_read") => self.allow_app_read(who, body),

            ("egress", "fetch") => self.fetch(who, body),
            ("egress", "disclose") => self.disclose(who, body),
            ("egress", "loosen") => self.loosen(who, body),
            ("egress", "lan") => self.lan(who, body),
            ("egress", "lock_category") => self.lock_category(who, body),

            ("members", "list") => self.list_members(who, body),
            ("members", "pair_device") => self.pair_device(who, body),
            ("members", "set_guardians") => self.set_guardians(who, body),
            ("members", "list_guardians") => self.list_guardians(who, body),

            ("schedules", "schedule") => self.schedule(who, body),
            ("schedules", "list") => self.list_schedules(who, body),
            ("schedules", "unschedule") => self.unschedule(who, body),

            ("household", "notify") => self.notify(who, body),
            ("household", "list_notifications") => self.list_notifications(who, body),
            ("household", "pause_app") => self.pause_app(who, body),
            ("household", "set_spend_allowance") => self.set_spend_allowance(who, body),
            ("household", "grant_support") => self.grant_support(who, body),
            ("household", "contribute") => self.contribute(who, body),
            ("household", "list_contributions") => self.list_contributions(who, body),
            ("household", "answer") => self.answer(who, body),
            ("household", "wipe") => self.wipe(who, body),

            _ => Err(bad(format!("surface `{surface}` has no call `{name}`"))),
        }
    }

    fn check_version(&self, surface: &str, version: &str) -> Result<(), CallError> {
        let Some((_, current)) = SURFACES.iter().find(|(name, _)| *name == surface) else {
            return Err(bad(format!("there is no surface `{surface}`")));
        };
        if version_answers(current, version) {
            return Ok(());
        }
        Err(refuse(
            "surface_version_unsupported",
            format!("the {surface} surface answers versions up to {current} of the same major; {version} was asked for"),
        )
        .detail(json!({ "surface": surface, "asked": version, "current": current }))
        .into())
    }

    fn resolve(&self, caller: Option<&Value>) -> Result<Who, CallError> {
        let Some(caller) = caller else {
            return Err(bad("missing `caller`: one of {\"shell\":member,\"device\":id}, {\"app\":id,\"member\":id} or {\"routine\":activity}"));
        };

        if let Some(member) = opt_s(caller, "shell") {
            self.household
                .member(&member)
                .ok_or_else(|| refuse("unknown_member", format!("{member} is not a member of the household")))?;
            return Ok(Who {
                kind: CallerKind::Shell,
                app: None,
                device: opt_s(caller, "device"),
                label: format!("member:{member}"),
                member: Some(member),
                activity: None,
            });
        }

        if let Some(activity) = opt_s(caller, "routine") {
            let act = self
                .activities
                .get(&activity)
                .ok_or_else(|| refuse("unknown_activity", format!("there is no activity {activity}")))?;
            return Ok(Who {
                kind: CallerKind::Routine,
                app: Some(act.app.clone()),
                member: Some(act.acts_for.clone()),
                label: format!("routine:{}/{}@{}", act.app, act.routine, act.id),
                activity: Some(activity),
                device: None,
            });
        }

        if let Some(app) = opt_s(caller, "app") {
            if !self.apps.get(&app).is_some_and(|a| a.installed) {
                return Err(refuse("app_not_installed", format!("{app} is not installed")).app(app).into());
            }
            let member = opt_s(caller, "member");
            if let Some(m) = &member
                && self.household.member(m).is_none()
            {
                return Err(refuse("unknown_member", format!("{m} is not a member of the household")).into());
            }
            return Ok(Who {
                kind: CallerKind::App,
                label: format!("app:{app}"),
                app: Some(app),
                member,
                activity: None,
                device: None,
            });
        }

        Err(bad("`caller` names no shell member, app or routine"))
    }

    /// Journals a refusal and returns it with its ledger line number.
    fn refused(&mut self, who: Option<&Who>, surface: &str, name: &str, mut r: Refusal) -> CallError {
        let actor = match who {
            Some(w) => self.describe(w),
            None => "An unknown caller".into(),
        };
        let what = match (&r.operation, &r.object) {
            (Some(op), Some(obj)) => format!("{op} on {obj}"),
            (Some(op), None) => op.clone(),
            _ => format!("{surface}.{name}"),
        };
        let plain = format!("{actor} asked for {what}; refused: {}.", r.text);

        let source = who.map(Who::source).unwrap_or_else(|| Source::service("call gate"));
        let mut entry = Entry::new("refusal", source, plain)
            .reason(r.code, &r.text)
            .detail(json!({ "call": format!("{surface}.{name}") }));
        entry = entry
            .maybe(Entry::app, r.app.clone().or_else(|| who.and_then(|w| w.app.clone())))
            .maybe(Entry::activity, r.activity.clone().or_else(|| who.and_then(|w| w.activity.clone())))
            .maybe(Entry::member, r.member.clone().or_else(|| who.and_then(|w| w.member.clone())))
            .maybe(Entry::holder, r.holder.clone())
            .maybe(Entry::operation, r.operation.clone())
            .maybe(Entry::object, r.object.clone())
            .maybe(Entry::grant, r.grant.clone())
            .maybe(Entry::proposal, r.proposal.clone());
        if let Some(d) = &r.detail {
            entry.detail = Some(json!({ "call": format!("{surface}.{name}"), "about": d }));
        }

        r.seq = Some(self.journal(entry));
        CallError::Refused(Box::new(r))
    }

    // ---- Names for plain-language lines ----

    pub fn app_name(&self, id: &str) -> String {
        self.apps.get(id).map(|a| a.manifest.display_name()).unwrap_or_else(|| id.into())
    }

    pub fn member_name(&self, id: &str) -> String {
        match self.household.member(id) {
            Some(m) if !m.name.is_empty() => m.name.clone(),
            _ => id.into(),
        }
    }

    pub fn describe(&self, who: &Who) -> String {
        match who.kind {
            CallerKind::Shell => self.member_name(who.member.as_deref().unwrap_or_default()),
            CallerKind::App => self.app_name(who.app.as_deref().unwrap_or_default()),
            CallerKind::Routine => {
                let act = &self.activities[who.activity.as_deref().unwrap_or_default()];
                format!("{}'s {} routine ({})", self.app_name(&act.app), act.routine, act.id)
            }
        }
    }

    // ---- Lookups ----

    pub fn installed(&self, app: &str) -> Result<&App, CallError> {
        self.apps
            .get(app)
            .filter(|a| a.installed)
            .ok_or_else(|| refuse("app_not_installed", format!("{app} is not installed")).app(app).into())
    }

    pub fn running_app(&self, app: &str) -> Result<&App, CallError> {
        let a = self.installed(app)?;
        if !a.on {
            let word = if a.paused { "paused" } else { "switched off" };
            return Err(refuse("app_off", format!("{} is {word}", a.manifest.display_name())).app(app).into());
        }
        Ok(a)
    }

    pub fn activity(&self, id: &str) -> Result<&Activity, CallError> {
        self.activities
            .get(id)
            .ok_or_else(|| refuse("unknown_activity", format!("there is no activity {id}")).activity(id).into())
    }

    /// The activity a call acts in: the calling routine's own, or the one an
    /// app names in the body. An app may only act in its own activities.
    pub fn acting_activity(&self, who: &Who, body: &Value) -> Result<String, CallError> {
        let id = match (&who.activity, opt_s(body, "activity")) {
            (Some(own), _) => own.clone(),
            (None, Some(named)) => named,
            (None, None) => return Err(bad("missing `activity`")),
        };
        let act = self.activity(&id)?;
        if who.kind != CallerKind::Shell && who.app.as_deref() != Some(act.app.as_str()) {
            return Err(refuse("outside_audience", format!("activity {id} belongs to another app")).activity(&id).into());
        }
        Ok(id)
    }

    /// Who may see an activity: its app, the member it acts for, the members
    /// in its audience, and the owner.
    pub fn in_activity_audience(&self, who: &Who, act: &Activity) -> bool {
        match who.kind {
            CallerKind::Shell => {
                let m = who.member.as_deref().unwrap_or_default();
                m == act.acts_for || act.audience.contains(m) || self.is_owner(m)
            }
            _ => who.app.as_deref() == Some(act.app.as_str()),
        }
    }

    pub fn is_owner(&self, member: &str) -> bool {
        self.household.member(member).is_some_and(|m| m.role == Role::Owner)
    }

    pub fn shell_member(&self, who: &Who, what: &str) -> Result<String, CallError> {
        match (who.kind, &who.member) {
            (CallerKind::Shell, Some(m)) => Ok(m.clone()),
            _ => Err(refuse("not_through_shell", format!("{} happens only through the shell, by a member", lowercase_first(what))).into()),
        }
    }

    /// A member on a device the approval path trusts.
    pub fn paired_member(&self, who: &Who, what: &str) -> Result<String, CallError> {
        let member = self.shell_member(who, what)?;
        let device = who.device.clone().unwrap_or_default();
        let paired = self.household.member(&member).is_some_and(|m| m.devices.contains(&device));
        if !paired {
            return Err(refuse(
                "device_not_paired",
                format!("{} is given only on a paired device; {} is not paired to {member}", lowercase_first(what), if device.is_empty() { "no device" } else { &device }),
            )
            .member(member)
            .into());
        }
        Ok(member)
    }

    /// Who may switch an app on or off, or pause it: any adult member for an
    /// add-on; the owner alone for Daybook.
    pub fn may_switch(&self, member: &str, app: &App) -> Result<(), CallError> {
        let role = self.household.member(member).map(|m| m.role);
        match (app.manifest.kind, role) {
            (AppKind::Core, Some(Role::Owner)) => Ok(()),
            (AppKind::Core, _) => Err(refuse("not_owner", format!("only the owner switches {} on or off", app.manifest.display_name())).into()),
            (AppKind::AddOn, Some(Role::Owner | Role::Adult)) => Ok(()),
            (AppKind::AddOn, _) => Err(refuse("not_adult", "only an adult member switches an add-on on or off").into()),
        }
    }

    pub fn audience_members(&self, audience: &Audience) -> BTreeSet<String> {
        match audience {
            Audience::Everyone(_) => self.household.members.iter().map(|m| m.id.clone()).collect(),
            Audience::Members { members } => members.iter().cloned().collect(),
            Audience::Guardians { guardians_of } => {
                self.household.guardians.get(guardians_of).cloned().unwrap_or_default().into_iter().collect()
            }
        }
    }

    pub fn needs_approval(&self, act: &Activity, operation: &str) -> bool {
        act.require_approval.contains(operation)
            || self.apps[&act.app]
                .manifest
                .operation(operation)
                .is_some_and(|o| o.approval == ApprovalDefault::Required)
    }

    pub fn approvers(&self, operation: &str) -> Vec<String> {
        self.household
            .members
            .iter()
            .filter(|m| m.role != Role::Child && m.may_approve(operation))
            .map(|m| m.id.clone())
            .collect()
    }

    // ---- Activity lifecycle ----

    pub fn set_state(&mut self, id: &str, state: ActivityState, why: &str, source: Source) {
        let act = self.activities.get_mut(id).unwrap();
        if act.state == state {
            return;
        }
        let from = act.state;
        act.state = state;
        let (app, routine, member) = (act.app.clone(), act.routine.clone(), act.acts_for.clone());

        let plain = format!("{}'s {routine} activity ({id}) is now {}: {why}.", self.app_name(&app), state.word());
        self.journal(
            Entry::new("activity", source, plain)
                .app(app)
                .activity(id)
                .member(member)
                .detail(json!({ "from": from.word(), "to": state.word(), "why": why })),
        );
    }

    /// Created becomes Running on the activity's first call.
    pub fn touch(&mut self, id: &str) {
        if self.activities[id].state == ActivityState::Created {
            self.set_state(id, ActivityState::Running, "it started work", Source::service("supervisor"));
        }
    }

    /// Any state to Closing: participants are asked to stop, the activity's
    /// grants stop conferring authority, pending proposals are withdrawn and
    /// child activities close first. Unknown outcomes are not waited on.
    pub fn enter_closing(&mut self, id: &str, why: &str, source: Source) {
        if self.activities[id].state.is_ending() {
            return;
        }

        let children: Vec<String> = self
            .activities
            .values()
            .filter(|a| a.parent.as_deref() == Some(id) && !a.state.is_ending())
            .map(|a| a.id.clone())
            .collect();
        for child in children {
            self.enter_closing(&child, "its parent activity is closing", source.clone());
        }

        self.set_state(id, ActivityState::Closing, why, source);
        self.end_grants(|g| g.holder == Holder::Activity(id.into()), "its activity is closing");
        self.withdraw_proposals(|p| p.activity == id, "its activity is closing");
    }

    pub fn end_grants(&mut self, which: impl Fn(&Grant) -> bool, why: &str) {
        let ids: Vec<String> = self
            .grants
            .values()
            .filter(|g| g.state == GrantState::Live && which(g))
            .map(|g| g.id.clone())
            .collect();

        for gid in ids {
            let g = self.grants.get_mut(&gid).unwrap();
            g.state = GrantState::Ended;
            let g = g.clone();
            self.invalidate_authorizations(|a| a.grant.as_deref() == Some(gid.as_str()), why);

            let plain = format!("Grant {gid} ({} on {}) ended: {why}.", g.operation, g.object);
            self.journal(
                Entry::new("grant_ended", Source::service("supervisor"), plain)
                    .app(g.app.clone())
                    .member(g.principal.clone())
                    .holder(g.holder.label())
                    .grant(gid)
                    .operation(g.operation)
                    .object(g.object),
            );
        }
    }

    pub fn withdraw_proposals(&mut self, which: impl Fn(&Proposal) -> bool, why: &str) {
        let ids: Vec<String> = self
            .proposals
            .values()
            .filter(|p| matches!(p.state, ProposalState::Pending | ProposalState::Approved) && which(p))
            .filter(|p| {
                // An approved proposal whose authorization was consumed has acted already.
                p.authorization
                    .as_ref()
                    .is_none_or(|a| self.authorizations[a].state == AuthorizationState::Unconsumed)
            })
            .map(|p| p.id.clone())
            .collect();

        for pid in ids {
            let p = self.proposals.get_mut(&pid).unwrap();
            p.state = ProposalState::Withdrawn;
            let p = p.clone();
            self.invalidate_authorizations(|a| a.proposal.as_deref() == Some(pid.as_str()), why);

            let plain = format!("{}'s proposal {pid} ({} on {}) was withdrawn: {why}.", self.app_name(&p.app), p.operation, p.object);
            self.journal(
                Entry::new("withdrawal", Source::service("approval path"), plain)
                    .app(p.app)
                    .activity(p.activity)
                    .proposal(pid)
                    .operation(p.operation)
                    .object(p.object),
            );
        }
    }

    pub fn invalidate_authorizations(&mut self, which: impl Fn(&Authorization) -> bool, why: &str) {
        for a in self.authorizations.values_mut() {
            if a.state == AuthorizationState::Unconsumed && which(a) {
                a.state = AuthorizationState::Invalidated { reason: why.into() };
            }
        }
    }

    /// After a revocation: an open activity that held the grant goes
    /// Suspended. Install grants reach activities through the child grants
    /// drawn from them, which stop with them.
    pub fn suspend_dependents(&mut self, gid: &str) {
        let Holder::Activity(id) = self.grants[gid].holder.clone() else { return };
        if self.activities.get(&id).is_some_and(|a| a.state.is_live()) {
            let why = format!("grant {gid} it needs was revoked");
            self.set_state(&id, ActivityState::Suspended, &why, Source::service("supervisor"));
        }
    }

    /// Switch off, pause and uninstall share this: no process, no live
    /// grants, no egress. Pending proposals are withdrawn with a ledger line;
    /// install grants are held, not live (revoked instead when the manifest
    /// sets `off_revokes`, or `revoke` is true).
    pub fn turn_off(&mut self, app: &str, paused: bool, member: &str, revoke: bool) {
        {
            let a = self.apps.get_mut(app).unwrap();
            a.on = false;
            a.paused = paused;
        }

        let source = Source::principal(member);
        let open: Vec<String> = self
            .activities
            .values()
            .filter(|a| a.app == app && !a.state.is_ending() && a.parent.is_none())
            .map(|a| a.id.clone())
            .collect();
        for id in open {
            self.enter_closing(&id, "its app was switched off", source.clone());
            self.set_state(&id, ActivityState::Closed, "its app was switched off; nothing of it runs", source.clone());
        }

        self.withdraw_proposals(|p| p.app == app, "its app was switched off");

        let revoke = revoke || self.apps[app].manifest.off_revokes;
        let ids: Vec<String> = self
            .grants
            .values()
            .filter(|g| g.app == app && g.state == GrantState::Live && matches!(g.holder, Holder::App(_)))
            .map(|g| g.id.clone())
            .collect();
        for gid in ids {
            let g = self.grants.get_mut(&gid).unwrap();
            g.state = if revoke { GrantState::Revoked } else { GrantState::Held };
            let g = g.clone();
            let plain = if revoke {
                format!("Install grant {gid} ({} on {}) was revoked with its app.", g.operation, g.object)
            } else {
                format!("Install grant {gid} ({} on {}) is held, not live, while its app is off; switching it on asks for it again.", g.operation, g.object)
            };
            let kind = if revoke { "revocation" } else { "grant_held" };
            self.journal(
                Entry::new(kind, source.clone(), plain)
                    .app(app)
                    .member(g.principal)
                    .holder(g.holder.label())
                    .grant(gid)
                    .operation(g.operation)
                    .object(g.object),
            );
        }
    }

    // ---- The clock ----

    /// Moves the clock. Awake, every tick, expiry and timeout in between
    /// happens at its own time, in order. Asleep (the fake was not running),
    /// ticks in between were missed: the latest one inside its catch-up
    /// window runs once, late, the rest are journaled as skipped, and
    /// expiries are noticed on waking.
    pub fn advance_to(&mut self, until: DateTime<Utc>, asleep: bool) {
        if asleep {
            let mut ticks: Vec<(DateTime<Utc>, String, Option<&'static str>)> = Vec::new();
            for s in self.schedules.values() {
                for t in schedule::ticks_between(s, self.now, until) {
                    ticks.push((t.at, s.id.clone(), t.adjustment));
                }
            }
            ticks.sort();

            self.now = until;
            self.expire();
            let mut latest: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();
            for (at, sid, _) in &ticks {
                latest.insert(sid.clone(), *at);
            }
            for (at, sid, adjustment) in ticks {
                let window = Duration::hours(self.schedules[&sid].catch_up_hours);
                if latest[&sid] == at && until - at <= window {
                    self.fire(&sid, at, true, adjustment);
                } else {
                    self.skip_tick(&sid, at, "Diamond Dog was not running, and the catch-up window has passed or a later tick replaced it");
                }
            }
            return;
        }

        loop {
            let tick = self
                .schedules
                .values()
                .filter_map(|s| schedule::ticks_between(s, self.now, until).into_iter().next().map(|t| (t.at, s.id.clone(), t.adjustment)))
                .min();
            let deadline = self.next_deadline().filter(|t| *t <= until);

            match (tick, deadline) {
                (Some((at, _, _)), Some(d)) if d <= at => {
                    self.now = d.max(self.now);
                    self.expire();
                }
                (None, Some(d)) => {
                    self.now = d.max(self.now);
                    self.expire();
                }
                (Some((at, sid, adjustment)), _) => {
                    self.now = at;
                    self.expire();
                    self.fire(&sid, at, false, adjustment);
                }
                (None, None) => break,
            }
        }
        self.now = until;
        self.expire();
    }

    /// The next time a live grant expires or an open activity times out.
    fn next_deadline(&self) -> Option<DateTime<Utc>> {
        let grants = self.grants.values().filter(|g| g.state == GrantState::Live).filter_map(|g| g.expires_at);
        let activities = self.activities.values().filter(|a| !a.state.is_ending()).map(|a| a.deadline);
        grants.chain(activities).filter(|t| *t > self.now).min()
    }

    fn skip_tick(&mut self, sid: &str, at: DateTime<Utc>, why: &str) {
        let s = self.schedules[sid].clone();
        let plain = format!(
            "{}'s {} run due {} was skipped: {why}.",
            self.app_name(&s.app),
            s.routine,
            schedule::local(&self.household.zone, at)
        );
        self.journal(
            Entry::new("tick", Source::service("scheduler"), plain)
                .app(s.app)
                .member(s.set_by)
                .detail(json!({ "schedule": sid, "due": stamp(at), "ran": false, "why": why })),
        );
    }

    fn fire(&mut self, sid: &str, at: DateTime<Utc>, late: bool, adjustment: Option<&'static str>) {
        let s = self.schedules[sid].clone();
        let Some(app) = self.apps.get(&s.app).cloned() else { return };
        if !app.installed {
            return;
        }
        if !app.on {
            let word = if app.paused { "paused" } else { "switched off" };
            self.skip_tick(sid, at, &format!("{} is {word}, so nothing ran and nothing left", app.manifest.display_name()));
            return;
        }

        // Software is never a principal: the run acts for the member who switched the app on.
        let member = app.switched_on_by.clone().unwrap_or_default();
        match self.open_activity(&s.app, &s.routine, &member, None, json!({ "schedule": sid, "due": stamp(at) }), None, BTreeSet::new(), None, Source::service("scheduler")) {
            Ok(aid) => {
                let mut plain = format!(
                    "{}'s {} run due {} started as {aid}, acting for {}",
                    app.manifest.display_name(),
                    s.routine,
                    schedule::local(&self.household.zone, at),
                    self.member_name(&member)
                );
                if late {
                    plain.push_str(", late");
                }
                if let Some(a) = adjustment {
                    plain.push_str(&format!("; {a}"));
                }
                plain.push('.');
                self.journal(
                    Entry::new("tick", Source::service("scheduler"), plain)
                        .app(s.app)
                        .activity(aid)
                        .member(member)
                        .detail(json!({ "schedule": sid, "due": stamp(at), "ran": true, "late": late, "adjustment": adjustment })),
                );
            }
            Err(CallError::Refused(r)) => self.skip_tick(sid, at, &r.text),
            Err(CallError::Bad(text)) => self.skip_tick(sid, at, &text),
        }
    }

    /// Grants, authorizations and activities whose time has run out.
    pub fn expire(&mut self) {
        let now = self.now;
        let ids: Vec<String> = self
            .grants
            .values()
            .filter(|g| g.state == GrantState::Live && g.expires_at.is_some_and(|t| t <= now))
            .map(|g| g.id.clone())
            .collect();
        for gid in ids {
            let g = self.grants.get_mut(&gid).unwrap();
            g.state = GrantState::Expired;
            let g = g.clone();
            let plain = format!("Grant {gid} ({} on {}) expired.", g.operation, g.object);
            self.journal(
                Entry::new("grant_ended", Source::service("supervisor"), plain)
                    .app(g.app)
                    .member(g.principal)
                    .holder(g.holder.label())
                    .grant(gid)
                    .operation(g.operation)
                    .object(g.object),
            );
        }

        self.invalidate_authorizations(|a| a.expires_at <= now, "it expired");

        let late: Vec<String> = self
            .activities
            .values()
            .filter(|a| !a.state.is_ending() && a.deadline <= now)
            .map(|a| a.id.clone())
            .collect();
        // A timed-out activity closes on its own: entering Closing ends its
        // temporary grants, so it goes on to Closed with no call from the app.
        for id in late {
            self.enter_closing(&id, "it reached its timeout", Source::service("supervisor"));
            self.set_state(&id, ActivityState::Closed, "it timed out and its temporary grants have ended", Source::service("supervisor"));
        }
    }
}

/// A surface answers its own version and any older minor of the same major.
pub fn version_answers(current: &str, asked: &str) -> bool {
    let parse = |v: &str| -> Option<(u32, u32)> {
        let (major, minor) = v.split_once('.')?;
        Some((major.parse().ok()?, minor.parse().ok()?))
    };
    match (parse(current), parse(asked)) {
        (Some((cm, cn)), Some((am, an))) => cm == am && an <= cn,
        _ => false,
    }
}
