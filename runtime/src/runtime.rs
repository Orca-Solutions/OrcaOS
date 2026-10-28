// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The runtime's state outside the enforcement core, and the call path every
//! surface shares: who is calling, which surface version they asked for, and
//! the rule that a refusal is journaled, never silent.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

pub use crate::core::hash;
use crate::core::Core;
use crate::core::journal::Journal;
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
    /// A request the runtime could not read. Not a refusal; not journaled.
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

// ---- The runtime ----

pub struct Runtime {
    /// The journal, activities, grants and authorizations.
    pub core: Core,
    pub household: Household,
    pub connections: BTreeMap<String, Connection>,
    pub apps: BTreeMap<String, App>,
    pub proposals: BTreeMap<String, Proposal>,
    pub receipts: BTreeMap<String, Receipt>,
    pub records: BTreeMap<String, Record>,
    pub schedules: BTreeMap<String, Schedule>,
    pub notifications: Vec<Notification>,
    pub contributions: Vec<Contribution>,
    /// (app, member): the member let that app's runs read records in their audience.
    pub app_read_allows: BTreeSet<(String, String)>,
    /// The outside systems. Every adapter talks to an in-process mock for now.
    pub mock: Mock,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    /// A runtime whose journal is kept in memory, for fixtures and tests.
    pub fn new() -> Self {
        let start = DateTime::parse_from_rfc3339("2026-11-02T12:00:00Z").unwrap().with_timezone(&Utc);
        Self::with_journal(Journal::in_memory(), start)
    }

    pub fn with_journal(journal: Journal, now: DateTime<Utc>) -> Self {
        Self {
            core: Core::new(journal, now),
            household: Household {
                zone: "UTC".into(),
                ..Default::default()
            },
            connections: BTreeMap::new(),
            apps: BTreeMap::new(),
            proposals: BTreeMap::new(),
            receipts: BTreeMap::new(),
            records: BTreeMap::new(),
            schedules: BTreeMap::new(),
            notifications: Vec::new(),
            contributions: Vec::new(),
            app_read_allows: BTreeSet::new(),
            mock: Mock::default(),
        }
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.core.clock.now
    }

    pub fn next_id(&mut self, prefix: &str) -> String {
        self.core.next_id(prefix)
    }

    pub fn journal(&mut self, entry: Entry) -> u64 {
        self.core.journal(entry)
    }

    pub fn ledger(&self) -> &[Entry] {
        self.core.entries()
    }

    /// Sets the household, and the zone ledger lines are stamped in.
    pub fn set_household(&mut self, household: Household) {
        self.core.clock.zone = household.zone.clone();
        self.household = household;
    }

    /// Receipts are kept in the journal; after a restart they are read back
    /// from it, so an outcome left unknown can still be reconciled.
    pub fn restore_receipts(&mut self) -> usize {
        let mut restored: BTreeMap<String, Receipt> = BTreeMap::new();
        for e in self.core.entries() {
            let Some(rid) = e.request_id.clone() else { continue };
            match e.kind.as_str() {
                "receipt" | "disclosure" => {
                    restored.insert(
                        rid.clone(),
                        Receipt {
                            request_id: rid,
                            activity: e.activity.clone().unwrap_or_default(),
                            app: e.app.clone().unwrap_or_default(),
                            member: e.member.clone().unwrap_or_default(),
                            operation: e.operation.clone().unwrap_or_default(),
                            object: e.object.clone().unwrap_or_default(),
                            authority: e.authority.clone().unwrap_or(Value::Null),
                            outcome: e.outcome.clone().unwrap_or(Value::Null),
                            evidence: e.evidence.clone().unwrap_or_default(),
                            detail: e.detail.clone().unwrap_or(json!({})),
                        },
                    );
                }
                "reconciliation" => {
                    if let Some(r) = restored.get_mut(&rid) {
                        r.outcome = e.outcome.clone().unwrap_or(Value::Null);
                        r.evidence = e.evidence.clone().unwrap_or_default();
                    }
                }
                _ => {}
            }
        }
        let n = restored.len();
        self.receipts.extend(restored);
        n
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
                .core
                .activity(&activity)
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
        app_name_in(&self.apps, id)
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
                let act = self.core.activity(who.activity.as_deref().unwrap_or_default()).expect("a routine caller names a known activity");
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
        self.core
            .activity(id)
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

    /// Moves an activity along the lifecycle through the core. A move the
    /// contract does not name is a bug here, never a caller's fault: it is
    /// not made, and debug builds stop on it.
    pub fn set_state(&mut self, id: &str, state: ActivityState, why: &str, source: Source) {
        let apps = &self.apps;
        let names = |app: &str| app_name_in(apps, app);
        if let Err(e) = self.core.transition(id, state, why, source, &names) {
            debug_assert!(false, "illegal lifecycle move: {e}");
            eprintln!("refused an illegal lifecycle move: {e}");
        }
    }

    /// Created becomes Running on the activity's first call.
    pub fn touch(&mut self, id: &str) {
        if self.core.activity(id).is_some_and(|a| a.state() == ActivityState::Created) {
            self.set_state(id, ActivityState::Running, "it started work", Source::service("supervisor"));
        }
    }

    /// Any open state to Closing: participants are asked to stop, the
    /// activity's grants stop conferring authority, pending proposals are
    /// withdrawn and child activities close first. Unknown outcomes are not
    /// waited on.
    pub fn enter_closing(&mut self, id: &str, why: &str, source: Source) {
        if self.core.activity(id).is_none_or(|a| a.state().is_ending()) {
            return;
        }

        let children: Vec<String> = self
            .core
            .activities()
            .filter(|a| a.parent.as_deref() == Some(id) && !a.state().is_ending())
            .map(|a| a.id.clone())
            .collect();
        for child in children {
            self.enter_closing(&child, "its parent activity is closing", source.clone());
        }

        self.set_state(id, ActivityState::Closing, why, source);
        self.core.end_grants(|g| g.holder == Holder::Activity(id.into()), "its activity is closing");
        self.withdraw_proposals(|p| p.activity == id, "its activity is closing");
    }

    pub fn withdraw_proposals(&mut self, which: impl Fn(&Proposal) -> bool, why: &str) {
        let ids: Vec<String> = self
            .proposals
            .values()
            .filter(|p| matches!(p.state, ProposalState::Pending | ProposalState::Approved) && which(p))
            .filter(|p| {
                // An approved proposal whose authorization was consumed has acted already.
                p.authorization.as_ref().is_none_or(|a| self.core.authorization(a).is_some_and(|a| a.unconsumed))
            })
            .map(|p| p.id.clone())
            .collect();

        for pid in ids {
            let p = self.proposals[&pid].clone();
            let plain = format!("{}'s proposal {pid} ({} on {}) was withdrawn: {why}.", self.app_name(&p.app), p.operation, p.object);
            self.journal(
                Entry::new("withdrawal", Source::service("approval path"), plain)
                    .app(&p.app)
                    .activity(&p.activity)
                    .proposal(&pid)
                    .operation(&p.operation)
                    .object(&p.object),
            );
            self.proposals.get_mut(&pid).unwrap().state = ProposalState::Withdrawn;
            self.core.invalidate_for_proposal(&pid, why);
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
            .core
            .activities()
            .filter(|a| a.app == app && !a.state().is_ending() && a.parent.is_none())
            .map(|a| a.id.clone())
            .collect();
        for id in open {
            self.enter_closing(&id, "its app was switched off", source.clone());
            self.set_state(&id, ActivityState::Closed, "its app was switched off; nothing of it runs", source.clone());
        }

        self.withdraw_proposals(|p| p.app == app, "its app was switched off");

        // Uninstalling revokes held install grants too, so a later install
        // cannot confirm one back to life.
        let revoke = revoke || self.apps[app].manifest.off_revokes;
        let ids: Vec<String> = self
            .core
            .grants()
            .filter(|g| g.app == app && matches!(g.holder, Holder::App(_)))
            .filter(|g| g.state() == GrantState::Live || (revoke && g.state() == GrantState::Held))
            .map(|g| g.id.clone())
            .collect();
        for gid in ids {
            let g = self.core.grant(&gid).unwrap().clone();
            let (kind, to, plain) = if revoke {
                ("revocation", GrantState::Revoked, format!("Install grant {gid} ({} on {}) was revoked with its app.", g.operation, g.object))
            } else {
                (
                    "grant_held",
                    GrantState::Held,
                    format!("Install grant {gid} ({} on {}) is held, not live, while its app is off; switching it on asks for it again.", g.operation, g.object),
                )
            };
            let entry = Entry::new(kind, source.clone(), plain)
                .app(app)
                .member(&g.principal)
                .holder(g.holder.label())
                .grant(&gid)
                .operation(&g.operation)
                .object(&g.object);
            self.core.change_grant(&gid, to, entry).expect("a live install grant can be held or revoked");
        }
    }

    // ---- The clock ----

    /// Moves the clock. Awake, every tick, expiry and timeout in between
    /// happens at its own time, in order. Asleep (the runtime was not running),
    /// ticks in between were missed: the latest one inside its catch-up
    /// window runs once, late, the rest are journaled as skipped, and
    /// expiries are noticed on waking.
    pub fn advance_to(&mut self, until: DateTime<Utc>, asleep: bool) {
        if asleep {
            let mut ticks: Vec<(DateTime<Utc>, String, Option<&'static str>)> = Vec::new();
            for s in self.schedules.values() {
                for t in schedule::ticks_between(s, self.now(), until) {
                    ticks.push((t.at, s.id.clone(), t.adjustment));
                }
            }
            ticks.sort();

            self.core.clock.now = until;
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
                .filter_map(|s| schedule::ticks_between(s, self.now(), until).into_iter().next().map(|t| (t.at, s.id.clone(), t.adjustment)))
                .min();
            let deadline = self.next_deadline().filter(|t| *t <= until);

            match (tick, deadline) {
                (Some((at, _, _)), Some(d)) if d <= at => {
                    self.core.clock.now = d.max(self.now());
                    self.expire();
                }
                (None, Some(d)) => {
                    self.core.clock.now = d.max(self.now());
                    self.expire();
                }
                (Some((at, sid, adjustment)), _) => {
                    self.core.clock.now = at;
                    self.expire();
                    self.fire(&sid, at, false, adjustment);
                }
                (None, None) => break,
            }
        }
        self.core.clock.now = until;
        self.expire();
    }

    /// The next time a live grant expires or an open activity times out.
    fn next_deadline(&self) -> Option<DateTime<Utc>> {
        let now = self.now();
        let activities = self.core.activities().filter(|a| !a.state().is_ending()).map(|a| a.deadline).filter(|t| *t > now);
        activities.chain(self.core.next_grant_expiry()).min()
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
        self.core.expire();

        let now = self.now();
        let late: Vec<String> = self
            .core
            .activities()
            .filter(|a| !a.state().is_ending() && a.deadline <= now)
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

/// An app's display name, or its id when it is not installed.
pub fn app_name_in(apps: &BTreeMap<String, App>, id: &str) -> String {
    apps.get(id).map(|a| a.manifest.display_name()).unwrap_or_else(|| id.into())
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
