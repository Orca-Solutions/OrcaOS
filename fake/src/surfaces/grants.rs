// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 3, grants and connections (0.1). A grant names the object, the
//! operation, the recipient, the duration and the principal. Only a member
//! grants: software is never a principal.

use serde_json::{Value, json};

use crate::fake::*;
use crate::ledger::{Entry, Source};
use crate::model::*;
use crate::surfaces::activities::grant_view;

impl Fake {
    pub fn add_connection(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Setting up a connection")?;
        if self.household.member(&member).is_some_and(|m| m.role == Role::Child) {
            return Err(refuse("not_adult", "only an adult member sets up a connection").into());
        }
        let handle = s(body, "handle")?;
        let kind = s(body, "kind")?;
        let account = opt_s(body, "account").unwrap_or_default();
        if !OPERATIONS.iter().any(|(_, adapter, _)| *adapter == kind) {
            return Err(refuse("no_adapter_contract", format!("there is no adapter contract for {kind}")).into());
        }

        self.connections.insert(handle.clone(), Connection { handle: handle.clone(), kind: kind.clone(), account: account.clone() });
        self.mock.add_connection(&kind, &handle, &account);

        let plain = format!("{} set up the {kind} connection {handle}; apps refer to it by handle and never see its credential.", self.member_name(&member));
        self.journal(Entry::new("connection", Source::principal(&member), plain).member(&member).object(format!("{kind}:{handle}")));
        Ok(json!({ "handle": handle, "kind": kind }))
    }

    pub fn grant(&mut self, who: &Who, body: &Value) -> Res {
        let operation = s(body, "operation")?;
        let object = s(body, "object")?;
        let holder_v = body.get("holder").cloned().unwrap_or_default();
        let holder = match (opt_s(&holder_v, "activity"), opt_s(&holder_v, "app")) {
            (Some(a), _) => Holder::Activity(a),
            (None, Some(app)) => Holder::App(app),
            _ => return Err(bad("`holder` is {\"activity\": id} or {\"app\": id}")),
        };
        let r = |code, text: String| refuse(code, text).holder(holder.label()).operation(&operation).object(&object);

        if !who.is_shell() {
            return Err(r("caller_not_principal", "software is never a principal; only a member grants, through the shell".into()).into());
        }
        let member = who.member.clone().unwrap_or_default();

        let (app_id, act) = match &holder {
            Holder::Activity(aid) => {
                let act = self.activity(aid)?.clone();
                (act.app.clone(), Some(act))
            }
            Holder::App(app) => (app.clone(), None),
            Holder::Support(_) => unreachable!(),
        };
        let app = self.running_app(&app_id).map_err(|e| crate::surfaces::install::with_app(e, &app_id))?.clone();
        let name = app.manifest.display_name();

        let Some(decl) = app.manifest.operation(&operation).cloned() else {
            return Err(r("operation_not_in_manifest", format!("{operation} is not in {name}'s manifest")).app(&app_id).into());
        };
        if !object_matches(&decl.object, &object) {
            return Err(r("object_outside_manifest", format!("{object} is outside the manifest's {}", decl.object)).app(&app_id).into());
        }

        match self.household.member(&member).map(|m| m.role) {
            Some(Role::Child) => return Err(r("principal_may_not_grant", "a child member does not grant".into()).app(&app_id).into()),
            Some(Role::Adult) if app.manifest.kind == AppKind::Core => {
                return Err(r("not_owner", format!("only the owner grants for {name}")).app(&app_id).into());
            }
            _ => {}
        }

        let kind = match opt_s(body, "kind").as_deref() {
            Some("one_time") => GrantKind::OneTime,
            Some("standing") => GrantKind::Standing,
            Some(other) => return Err(bad(format!("unknown grant kind {other}"))),
            None => decl.grant,
        };
        // An install grant (G-6) is held by the app and lasts until revoked at the app.
        if matches!(holder, Holder::App(_)) && (kind == GrantKind::OneTime || decl.grant != GrantKind::Standing) {
            return Err(r("operation_not_standing", format!("{operation} is not declared standing in {name}'s manifest, so it cannot be an install grant")).app(&app_id).into());
        }

        // A one-time grant is bound to the work state it was given for.
        let bound_hash = match (kind, opt_s(body, "bound_hash"), body.get("bound_to")) {
            (GrantKind::OneTime, Some(h), _) => Some(h),
            (GrantKind::OneTime, None, Some(state)) => Some(hash(state)),
            (GrantKind::OneTime, None, None) => {
                return Err(r("work_state_missing", "a one-time grant needs the work state it is bound to (`bound_to` or `bound_hash`)".into())
                    .app(&app_id)
                    .into());
            }
            _ => None,
        };

        let mut expires_at = time(body, "expires_at")?;
        let mut derived_from = None;
        if let Some(act) = &act {
            if act.state.is_ending() {
                return Err(r("activity_not_open", format!("activity {} is {}", act.id, act.state.word())).activity(&act.id).into());
            }
            if expires_at.is_some_and(|t| t > act.deadline) {
                return Err(r("expiry_beyond_activity", format!("the grant would outlive activity {}", act.id)).activity(&act.id).into());
            }
            expires_at = Some(expires_at.unwrap_or(act.deadline));

            // A child activity holds a subset of its parent's authority.
            if let Some(pid) = &act.parent {
                let parent_app = act.app.clone();
                let covering = self.grants.values().find(|g| {
                    g.state == GrantState::Live
                        && (g.holder == Holder::Activity(pid.clone()) || g.holder == Holder::App(parent_app.clone()))
                        && g.covers(&operation, &object)
                });
                match covering {
                    Some(g) => derived_from = Some(g.id.clone()),
                    None => {
                        return Err(r("beyond_parent_authority", format!("parent activity {pid} holds no live grant covering this")).activity(&act.id).into());
                    }
                }
            }
        }

        let gid = self.make_grant(holder.clone(), &app_id, &operation, &object, &member, kind, bound_hash.clone(), expires_at, derived_from, false);

        if let Some(act) = &act
            && act.state == ActivityState::Suspended
        {
            self.set_state(&act.id, ActivityState::Running, &format!("grant {gid} restored its authority"), Source::principal(&member));
        }

        Ok(json!({ "grant": gid, "bound_hash": bound_hash, "expires_at": expires_at.map(stamp) }))
    }

    pub fn revoke(&mut self, who: &Who, body: &Value) -> Res {
        let gid = s(body, "grant")?;
        let Some(g) = self.grants.get(&gid).cloned() else {
            return Err(bad(format!("there is no grant {gid}")));
        };
        // A member revokes through the shell; an app may give up its own grants.
        if !who.is_shell() && who.app.as_deref() != Some(g.app.as_str()) {
            return Err(refuse("outside_audience", format!("grant {gid} belongs to another app")).grant(&gid).into());
        }
        let actor = self.describe(who);

        if g.state != GrantState::Live {
            let plain = format!("{actor} revoked grant {gid}, which had already stopped conferring authority.");
            self.journal(Entry::new("revocation", who.source(), plain).app(&g.app).grant(&gid).operation(&g.operation).object(&g.object));
            return Ok(json!({ "grant": gid, "state": g.state }));
        }

        // Everything delegated from it stops too.
        let mut ids = vec![gid.clone()];
        let mut i = 0;
        while i < ids.len() {
            let parent = ids[i].clone();
            ids.extend(
                self.grants
                    .values()
                    .filter(|c| c.state == GrantState::Live && c.derived_from.as_deref() == Some(parent.as_str()))
                    .map(|c| c.id.clone()),
            );
            i += 1;
        }

        for id in &ids {
            let g = self.grants.get_mut(id).unwrap();
            g.state = GrantState::Revoked;
            let g = g.clone();
            self.invalidate_authorizations(|a| a.grant.as_deref() == Some(id.as_str()), "its grant was revoked");

            let plain = if *id == gid {
                format!("{actor} revoked grant {id} ({} on {}); it confers no authority for future actions, and reads already made stay.", g.operation, g.object)
            } else {
                format!("Grant {id} stopped with grant {gid}, which it was delegated from.")
            };
            let mut entry = Entry::new("revocation", who.source(), plain)
                .app(&g.app)
                .member(&g.principal)
                .holder(g.holder.label())
                .grant(id)
                .operation(&g.operation)
                .object(&g.object);
            if let Holder::Activity(a) = &g.holder {
                entry = entry.activity(a);
            }
            self.journal(entry);
            self.suspend_dependents(id);
        }

        Ok(json!({ "grant": gid, "state": "revoked", "also_stopped": &ids[1..] }))
    }

    pub fn list_grants(&mut self, who: &Who, body: &Value) -> Res {
        let app = opt_s(body, "app");
        let activity = opt_s(body, "activity");
        let object = opt_s(body, "object");
        let me = who.member.clone().unwrap_or_default();
        let owner = who.is_shell() && self.is_owner(&me);

        let grants: Vec<Value> = self
            .grants
            .values()
            .filter(|g| g.state == GrantState::Live)
            .filter(|g| match who.kind {
                CallerKind::Shell => owner || g.principal == me,
                _ => who.app.as_deref() == Some(g.app.as_str()),
            })
            .filter(|g| app.as_ref().is_none_or(|a| *a == g.app))
            .filter(|g| activity.as_ref().is_none_or(|a| g.holder == Holder::Activity(a.clone()) || g.holder == Holder::App(self.activities.get(a).map(|x| x.app.clone()).unwrap_or_default())))
            .filter(|g| object.as_ref().is_none_or(|o| object_matches(&g.object, o)))
            .map(grant_view)
            .collect();
        Ok(json!({ "grants": grants }))
    }

    /// Use (read): the read proceeds on the grant alone.
    pub fn use_grant(&mut self, who: &Who, body: &Value) -> Res {
        let aid = self.acting_activity(who, body)?;
        let operation = s(body, "operation")?;
        let object = s(body, "object")?;

        if adapter_of(&operation).is_some_and(|(_, consequential)| consequential) {
            return Err(bad(format!("{operation} is a consequential action: use ledger.act, or the egress calls")));
        }
        let g = self.check_grant(&aid, &operation, &object, opt_s(body, "grant"))?;
        let items = self.read_through_adapter(&operation, &object);

        self.touch(&aid);
        self.activities.get_mut(&aid).unwrap().used_grants.insert(g.id.clone());
        if g.kind == GrantKind::OneTime {
            self.grants.get_mut(&g.id).unwrap().state = GrantState::Used;
        }

        let act = self.activities[&aid].clone();
        let count = match items.len() {
            1 => "1 item".to_string(),
            n => format!("{n} items"),
        };
        let plain = format!("{} read {object} under grant {} ({count}).", self.app_name(&act.app), g.id);
        self.journal(
            Entry::new("read", Source::service("enforcement check"), plain)
                .app(&act.app)
                .activity(&aid)
                .member(&act.acts_for)
                .grant(&g.id)
                .operation(&operation)
                .object(&object)
                .evidence("observed")
                .authority(json!({ "grant": g.id, "principal": g.principal })),
        );
        Ok(json!({ "items": items }))
    }

    /// The checks every use of a grant shares: the app is on, the operation is
    /// in the manifest, the grant is live and held by this activity or its
    /// app, it covers the object, and the activity is open.
    pub fn check_grant(&self, aid: &str, operation: &str, object: &str, explicit: Option<String>) -> Result<Grant, CallError> {
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

        // An activity uses grants it holds; install grants reach it as child grants.
        let held = |g: &Grant| g.holder == Holder::Activity(aid.into());
        let grant = match explicit {
            Some(gid) => match self.grants.get(&gid) {
                Some(g) if held(g) => g.clone(),
                _ => return Err(r("no_grant", format!("grant {gid} is not held by this activity or its app")).grant(gid).into()),
            },
            None => {
                let mut candidates: Vec<&Grant> = self.grants.values().filter(|g| held(g) && g.covers(operation, object)).collect();
                candidates.sort_by_key(|g| g.state != GrantState::Live);
                match candidates.first() {
                    Some(g) => (*g).clone(),
                    None => return Err(r("no_grant", format!("no grant covers {operation} on {object}")).into()),
                }
            }
        };

        let gid = grant.id.clone();
        match grant.state {
            GrantState::Live => {}
            GrantState::Revoked => return Err(r("grant_revoked", format!("grant {gid} was revoked")).grant(gid).into()),
            GrantState::Expired => return Err(r("grant_expired", format!("grant {gid} expired")).grant(gid).into()),
            GrantState::Used => return Err(r("grant_used", format!("one-time grant {gid} was already used")).grant(gid).into()),
            GrantState::Ended | GrantState::Held => return Err(r("grant_ended", format!("grant {gid} has ended")).grant(gid).into()),
        }

        // A child grant is good only while the grant it was drawn from is.
        let mut from = grant.derived_from.clone();
        while let Some(pid) = from {
            let parent = &self.grants[&pid];
            if parent.state != GrantState::Live {
                return Err(r("grant_revoked", format!("grant {gid} was drawn from grant {pid}, which no longer confers authority")).grant(gid).into());
            }
            from = parent.derived_from.clone();
        }
        if grant.expires_at.is_some_and(|t| t <= self.now) {
            return Err(r("grant_expired", format!("grant {gid} expired")).grant(gid).into());
        }
        if !grant.covers(operation, object) {
            return Err(r("object_outside_grant", format!("grant {gid} covers {} on {}", grant.operation, grant.object)).grant(gid).into());
        }

        match act.state {
            ActivityState::Suspended => Err(r("activity_suspended", format!("activity {aid} is suspended until a grant restores its authority")).into()),
            s if s.is_ending() => Err(r("activity_not_open", format!("activity {aid} is {}", s.word())).into()),
            _ => Ok(grant),
        }
    }

    fn read_through_adapter(&mut self, operation: &str, object: &str) -> Vec<Value> {
        let handle = object.split_once(':').map(|(_, h)| h).unwrap_or(object);
        match operation {
            "mail.read" => self
                .mock
                .mail
                .get(handle)
                .map(|m| {
                    m.inbox
                        .iter()
                        .map(|msg| {
                            json!({
                                "message": msg,
                                "provenance": {
                                    "source": "external",
                                    "system": format!("mail:{}", m.account),
                                    "relayed_by": format!("mail adapter ({handle})"),
                                    "trust": "untrusted",
                                },
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            "calendar.read" => self.mock.calendar.get(handle).cloned().unwrap_or_default(),
            _ => Vec::new(),
        }
    }
}
