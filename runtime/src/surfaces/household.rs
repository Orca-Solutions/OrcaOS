// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 10, notifications and household controls (0.3).

use std::collections::BTreeSet;

use chrono::Duration;
use serde_json::{Value, json};

use crate::runtime::*;
use crate::ledger::{Entry, Source};
use crate::model::*;
use crate::schedule;

const CONTRIBUTION_KINDS: [&str; 6] = ["matter", "question", "review", "notice", "offer", "settings_panel"];

impl Runtime {
    pub fn notify(&mut self, who: &Who, body: &Value) -> Res {
        let aid = self.acting_activity(who, body)?;
        let member = s(body, "member")?;
        let kind = s(body, "kind")?;
        let act = self.activity(&aid)?.clone();
        let app = self.running_app(&act.app)?.clone();
        let r = |code, text: String| refuse(code, text).app(&act.app).activity(&aid).member(&member);

        if !act.audience.contains(&member) {
            return Err(r("outside_audience", format!("{} is outside activity {aid}'s audience", self.member_name(&member))).into());
        }

        // A live notice the member started is always allowed; the rest draw on the budget.
        let live = kind == "live_notice";
        if !live && !app.manifest.notifications.kinds.contains(&kind) {
            return Err(r("notification_kind_undeclared", format!("{} declares no notification kind {kind}", app.manifest.display_name())).into());
        }
        if !live {
            let today = schedule::local_date(&self.household.zone, self.now());
            let used = self
                .notifications
                .iter()
                .filter(|n| n.app == act.app && n.kind != "live_notice" && schedule::local_date(&self.household.zone, n.at) == today)
                .count() as u32;
            if used >= app.manifest.notifications.per_day {
                return Err(r("notification_budget_spent", format!("{} has used its {} notifications for today", app.manifest.display_name(), app.manifest.notifications.per_day)).into());
            }
        }

        let nid = self.next_id("N");
        self.notifications.push(Notification { id: nid.clone(), member: member.clone(), app: act.app.clone(), activity: aid.clone(), kind: kind.clone(), at: self.now() });
        let plain = format!("{} sent {} a {kind} push; it carries no content, which is read in the shell.", app.manifest.display_name(), self.member_name(&member));
        self.journal(Entry::new("notification", who.source(), plain).app(&act.app).activity(&aid).member(&member).detail(json!({ "kind": kind, "notification": nid })));
        Ok(json!({ "notification": nid }))
    }

    pub fn list_notifications(&mut self, who: &Who, _body: &Value) -> Res {
        let member = self.shell_member(who, "Reading notifications")?;
        let items: Vec<&Notification> = self.notifications.iter().filter(|n| n.member == member).collect();
        Ok(json!({ "notifications": items }))
    }

    pub fn pause_app(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Pausing an app")?;
        let app_id = s(body, "app")?;
        let app = self.installed(&app_id)?.clone();
        self.may_switch(&member, &app).map_err(|e| crate::surfaces::install::with_app(e, &app_id))?;

        let plain = format!("{} paused {}: the same as switching it off, by the member's decision.", self.member_name(&member), app.manifest.display_name());
        self.journal(Entry::new("pause", Source::principal(&member), plain).app(&app_id).member(&member));
        self.turn_off(&app_id, true, &member, false);
        Ok(json!({ "app": app_id, "paused": true }))
    }

    pub fn set_spend_allowance(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Setting a spend allowance")?;
        let app_id = s(body, "app")?;
        self.installed(&app_id)?;
        if !self.is_owner(&member) {
            return Err(refuse("not_owner", "only the owner sets spend allowances").app(&app_id).into());
        }
        let ceiling = body.get("ceiling_cents").and_then(Value::as_u64).ok_or_else(|| bad("missing `ceiling_cents`"))?;
        let period = opt_s(body, "period").unwrap_or_else(|| "month".into());
        let paused = body.get("paused").and_then(Value::as_bool).unwrap_or(false);

        let now = self.now();
        let app = self.apps.get_mut(&app_id).unwrap();
        let spent = app.allowance.as_ref().map(|a| a.spent_cents).unwrap_or(0);
        let start = app.allowance.as_ref().map(|a| a.period_start).unwrap_or(now);
        app.allowance = Some(Allowance { ceiling_cents: ceiling, period: period.clone(), paused, spent_cents: spent, period_start: start });

        let plain = format!(
            "{} set {}'s model spend to at most {ceiling} cents a {period}{}.",
            self.member_name(&member),
            self.app_name(&app_id),
            if paused { ", paused" } else { "" }
        );
        self.journal(Entry::new("allowance", Source::principal(&member), plain).app(&app_id).member(&member).detail(json!({ "ceiling_cents": ceiling, "period": period, "paused": paused })));
        Ok(json!({ "app": app_id, "ceiling_cents": ceiling, "period": period, "paused": paused, "spent_cents": spent }))
    }

    /// A time-boxed grant to a support person, in the ledger like any other.
    /// It is recorded; no support session is run.
    pub fn grant_support(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Granting a support session")?;
        let support = s(body, "support")?;
        let minutes = body.get("minutes").and_then(Value::as_i64).unwrap_or(60);
        let operations = strings(body, "operations");

        // A member may share what they can read themselves, nothing more.
        let within = ["ledger.read", "records.read"];
        if let Some(op) = operations.iter().find(|op| !within.contains(&op.as_str())) {
            return Err(refuse("beyond_member_authority", format!("{op} is beyond what a member can hand to support")).operation(op).into());
        }

        let expires = self.now() + Duration::minutes(minutes);
        let ids: Vec<String> = operations
            .iter()
            .map(|op| self.make_grant(Holder::Support(support.clone()), "household", op, &format!("member:{member}"), &member, GrantKind::Standing, None, Some(expires), None, false))
            .collect();
        Ok(json!({ "grants": ids, "expires_at": stamp(expires) }))
    }

    /// Contribute (G-8): an item in the shell that carries no authority.
    pub fn contribute(&mut self, who: &Who, body: &Value) -> Res {
        let app_id = who.app.clone().ok_or_else(|| bad("only an app or a routine contributes"))?;
        let app = self.running_app(&app_id)?.clone();
        let kind = s(body, "kind")?;
        let content = body.get("content").cloned().unwrap_or(Value::Null);
        let activity = who.activity.clone().or_else(|| opt_s(body, "activity"));
        let r = |code, text: String| refuse(code, text).app(&app_id);

        if !CONTRIBUTION_KINDS.contains(&kind.as_str()) {
            return Err(bad(format!("`kind` is one of {CONTRIBUTION_KINDS:?}")));
        }

        // The app's audience: Daybook acts only for the owner; an add-on for the household.
        let app_audience: BTreeSet<String> = match app.manifest.kind {
            AppKind::Core => self.household.owner().map(|o| o.id.clone()).into_iter().collect(),
            AppKind::AddOn => self.household.members.iter().map(|m| m.id.clone()).collect(),
        };
        let audience: BTreeSet<String> = match body.get("audience") {
            Some(_) => strings(body, "audience").into_iter().collect(),
            None => match &activity {
                Some(a) => self.activity(a)?.audience.clone(),
                None => app_audience.clone(),
            },
        };
        if !audience.is_subset(&app_audience) {
            return Err(r("outside_audience", "the item's audience reaches beyond the app's".into()).into());
        }
        if kind == "settings_panel" && self.contributions.iter().any(|c| c.app == app_id && c.kind == "settings_panel") {
            return Err(r("second_settings_panel", "an app has one settings panel".into()).into());
        }
        if content.get("operation").is_some() || content.get("grant").is_some() {
            return Err(r("needs_proposal", "content that would need a grant is a proposal, not a contribution".into()).into());
        }

        let cid = self.next_id("C");
        self.contributions.push(Contribution { id: cid.clone(), app: app_id.clone(), kind: kind.clone(), content: content.clone(), audience: audience.clone(), activity: activity.clone(), at: self.now(), answered_by: Vec::new() });

        let plain = format!("{} added a {kind} to the shell; it carries no authority.", app.manifest.display_name());
        let mut entry = Entry::new("contribution", who.source(), plain).app(&app_id).detail(json!({ "item": cid, "kind": kind, "audience": audience }));
        if let Some(a) = &activity {
            entry = entry.activity(a);
        }
        if let Some(m) = &who.member {
            entry = entry.member(m);
        }
        self.journal(entry);
        Ok(json!({ "item": cid }))
    }

    pub fn list_contributions(&mut self, who: &Who, _body: &Value) -> Res {
        let items: Vec<&Contribution> = match who.kind {
            CallerKind::Shell => {
                let m = who.member.clone().unwrap_or_default();
                self.contributions.iter().filter(|c| c.audience.contains(&m)).collect()
            }
            _ => self.contributions.iter().filter(|c| who.app.as_ref() == Some(&c.app)).collect(),
        };
        Ok(json!({ "items": items }))
    }

    /// A member answers a question or confirms a review. The answer comes
    /// back as a record with source principal, never as a grant.
    pub fn answer(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Answering")?;
        let cid = s(body, "item")?;
        let response = body.get("response").cloned().unwrap_or(Value::Null);
        let Some(item) = self.contributions.iter().find(|c| c.id == cid).cloned() else {
            return Err(bad(format!("there is no item {cid}")));
        };
        if !matches!(item.kind.as_str(), "question" | "review") {
            return Err(bad(format!("item {cid} is a {}, which takes no answer", item.kind)));
        }
        if !item.audience.contains(&member) {
            return Err(refuse("outside_audience", format!("item {cid} is outside {member}'s audience")).app(&item.app).into());
        }

        let rid = self.next_id("R");
        self.records.insert(
            rid.clone(),
            Record {
                id: rid.clone(),
                app: item.app.clone(),
                record_type: format!("{}_answer", item.kind),
                version: 1,
                content: json!({ "item": cid, "response": response }),
                provenance: json!({ "source": member, "via": "shell", "trust": "member" }),
                trust: "member".into(),
                audience: Audience::Members { members: item.audience.iter().cloned().collect() },
                written_by: member.clone(),
                at: self.now(),
                client_key: None,
                deleted: false,
            },
        );
        if let Some(c) = self.contributions.iter_mut().find(|c| c.id == cid) {
            c.answered_by.push(member.clone());
        }

        let plain = format!("{} answered {}'s {} {cid}; the answer is record {rid}, not a grant.", self.member_name(&member), self.app_name(&item.app), item.kind);
        self.journal(Entry::new("answer", Source::principal(&member), plain).app(&item.app).member(&member).object(format!("record:{rid}")).detail(json!({ "item": cid })));
        Ok(json!({ "record": rid }))
    }

    /// Wipe (G-9): the factory reset. Every app's records, grants and journal
    /// go, and it is the one action that leaves no ledger line behind.
    pub fn wipe(&mut self, who: &Who, _body: &Value) -> Res {
        let member = self.shell_member(who, "Wiping the household")?;
        if !self.is_owner(&member) {
            return Err(refuse("not_owner", "only the owner wipes the household").into());
        }

        let apps: Vec<String> = self.apps.keys().cloned().collect();
        for app in apps {
            let a = self.apps.get_mut(&app).unwrap();
            a.on = false;
            a.switched_on_by = None;
        }
        self.proposals.clear();
        self.receipts.clear();
        self.records.clear();
        self.schedules.clear();
        self.notifications.clear();
        self.contributions.clear();
        self.app_read_allows.clear();
        self.core.wipe();
        Ok(json!({ "wiped": true }))
    }
}
