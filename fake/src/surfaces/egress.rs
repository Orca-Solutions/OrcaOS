// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Surface 7, egress and disclosure (0.3). Every byte that leaves goes
//! through an adapter under a grant, and every disclosure is journaled with
//! what went where.

use chrono::Duration;
use serde_json::{Value, json};

use crate::fake::*;
use crate::ledger::{Entry, Source};
use crate::model::*;

impl Fake {
    /// Fetch: a page through the bounded browser, or (G-17, `api: true`) an
    /// API request whose raw body comes back with its content type. Either
    /// way the grant is scoped to the host, and only the query parameters
    /// the manifest declares for that host may be sent.
    pub fn fetch(&mut self, who: &Who, body: &Value) -> Res {
        let aid = self.acting_activity(who, body)?;
        let host = s(body, "host")?;
        let path = opt_s(body, "path").unwrap_or_else(|| "/".into());
        let api = body.get("api").and_then(Value::as_bool).unwrap_or(false);
        let query = body.get("query").and_then(Value::as_object).cloned().unwrap_or_default();
        let object = format!("web:{host}");

        let g = self.check_grant(&aid, "web.fetch", &object, opt_s(body, "grant"))?;
        let act = self.activities[&aid].clone();

        let declared = self.apps[&act.app].manifest.fetch_params(&host);
        let undeclared: Vec<&String> = query.keys().filter(|k| !declared.contains(k)).collect();
        if !undeclared.is_empty() {
            let names: Vec<&str> = undeclared.iter().map(|k| k.as_str()).collect();
            return Err(refuse("query_parameter_undeclared", format!("{} may not send {} to {host}; its manifest declares only {:?}", self.app_name(&act.app), names.join(", "), declared))
                .app(&act.app)
                .activity(&aid)
                .holder(format!("activity:{aid}"))
                .operation("web.fetch")
                .object(&object)
                .grant(&g.id)
                .detail(json!({ "undeclared": names, "declared": declared }))
                .into());
        }

        let page = self.mock.web.get(&host).and_then(|p| p.get(&path)).cloned();

        if let Some(to) = page.as_ref().and_then(|p| p.redirect_to.clone()) {
            let off_host = to.split_once("://").map(|(_, rest)| rest).unwrap_or(&to).split('/').next().unwrap_or_default() != host;
            if off_host {
                return Err(refuse("redirect_off_host", format!("{host}{path} redirects to {to}, off the granted host"))
                    .app(&act.app)
                    .activity(&aid)
                    .operation("web.fetch")
                    .object(&object)
                    .grant(&g.id)
                    .into());
            }
        }

        let (status, text, content_type) = match &page {
            Some(p) => (200, p.body.clone(), p.content_type.clone()),
            None => (404, String::new(), "text/plain".to_string()),
        };
        let mut params: Vec<&String> = query.keys().collect();
        params.sort();
        self.mock.fetches.push(json!({ "activity": aid, "host": host, "path": path, "query": query, "api": api, "status": status }));
        self.touch(&aid);
        self.activities.get_mut(&aid).unwrap().used_grants.insert(g.id.clone());

        let adapter = if api { "fetch adapter" } else { "bounded browser" };
        let plain = format!("{} fetched {host}{path} through the {adapter} under grant {} ({status}).", self.app_name(&act.app), g.id);
        self.journal(
            Entry::new("fetch", Source::external(&host, adapter), plain)
                .app(&act.app)
                .activity(&aid)
                .member(&act.acts_for)
                .grant(&g.id)
                .operation("web.fetch")
                .object(&object)
                .evidence("observed")
                .authority(json!({ "grant": g.id, "principal": g.principal }))
                .detail(json!({ "path": path, "api": api, "params": params, "status": status })),
        );
        Ok(json!({
            "status": status,
            "content_type": content_type,
            "body": text,
            "provenance": { "source": "external", "system": host, "relayed_by": adapter, "trust": "untrusted" },
        }))
    }

    pub fn disclose(&mut self, who: &Who, body: &Value) -> Res {
        let aid = self.acting_activity(who, body)?;
        let provider = s(body, "provider")?;
        let content = body.get("content").cloned().unwrap_or(Value::Null);
        let object = format!("provider:{provider}");
        let act = self.action_base(&aid, "provider.disclose", &object)?;
        let app = self.apps[&act.app].clone();
        let r = |code, text: String| refuse(code, text).app(&act.app).activity(&aid).holder(format!("activity:{aid}")).operation("provider.disclose").object(&object);

        if !app.manifest.providers.iter().any(|p| p.name == provider) {
            return Err(r("provider_not_declared", format!("{provider} is not a provider in {}'s manifest", app.manifest.display_name())).into());
        }

        // The categories are what the routine declares plus what the records it
        // sends always carry. The fake does not read free text.
        let mut categories: Vec<String> = strings(body, "categories");
        for id in strings(body, "records") {
            let Some(record) = self.records.get(&id).cloned() else {
                return Err(bad(format!("there is no record {id}")));
            };
            if !self.audience_members(&record.audience).contains(&act.acts_for) {
                return Err(r("outside_audience", format!("record {id} is outside {}'s audience", self.member_name(&act.acts_for))).into());
            }
            let carried = self.apps.get(&record.app).and_then(|a| a.manifest.record_type(&record.record_type)).map(|t| t.categories.clone()).unwrap_or_default();
            categories.extend(carried);
        }
        categories.sort();
        categories.dedup();

        for c in &categories {
            if self.household.locked.contains(c) {
                return Err(r("category_locked", format!("{c} is locked for the whole household")).detail(json!({ "categories": categories })).into());
            }
            if !app.manifest.disclosure_categories.contains(c) {
                return Err(r("category_not_allowed", format!("{c} is outside {}'s disclosure categories", app.manifest.display_name())).detail(json!({ "categories": categories })).into());
            }
            if SENSITIVE.contains(&c.as_str()) && !act.loosened.contains(c) {
                return Err(r("category_not_allowed", format!("{c} stays in unless a member loosens it for this matter")).detail(json!({ "categories": categories })).into());
            }
        }

        // Spend: the app's ceiling and the routine's cost allowance.
        let price = self.mock.providers.get(&provider).map(|p| p.price_cents).unwrap_or(2);
        self.roll_allowance(&act.app);
        let over_app = self.apps[&act.app].allowance.as_ref().is_some_and(|a| a.paused || a.spent_cents + price > a.ceiling_cents);
        let over_activity = act.cost_allowance_cents.is_some_and(|c| act.cost_cents + price > c);
        if over_app || over_activity {
            let why = if over_app { "the app's spend allowance is paused or spent" } else { "the activity's cost allowance is spent" };
            self.set_state(&aid, ActivityState::Waiting, why, Source::service("supervisor"));
            let code = if over_app { "spend_ceiling_reached" } else { "cost_allowance_spent" };
            return Err(r(code, why.into()).into());
        }

        let payload = json!({ "provider": provider, "categories": categories, "content": content });
        let cleared = self.enforce(&aid, "provider.disclose", &object, &payload, opt_s(body, "proposal"), opt_s(body, "grant"))?;

        let conversation = format!("conv-{aid}");
        self.mock.providers.entry(provider.clone()).or_insert(crate::mock::Provider { price_cents: price, disclosures: Vec::new() }).disclosures.push(json!({
            "request_id": cleared.request_id,
            "conversation": conversation,
            "categories": categories,
            "content": content,
        }));
        self.activities.get_mut(&aid).unwrap().cost_cents += price;
        if let Some(a) = self.apps.get_mut(&act.app).unwrap().allowance.as_mut() {
            a.spent_cents += price;
        }

        let detail = json!({ "provider": provider, "categories": categories, "conversation": conversation, "memory": "off", "cost_cents": price });
        let receipt = self.disclosure_receipt(&aid, &object, cleared, detail);
        Ok(receipt)
    }

    fn disclosure_receipt(&mut self, aid: &str, object: &str, cleared: crate::surfaces::actions::Cleared, detail: Value) -> Value {
        let act = self.activities[aid].clone();
        let provider = detail["provider"].as_str().unwrap_or_default().to_string();
        let outcome = json!({ "state": "acknowledged", "conversation": detail["conversation"] });
        self.receipts.insert(
            cleared.request_id.clone(),
            Receipt {
                request_id: cleared.request_id.clone(),
                activity: aid.into(),
                app: act.app.clone(),
                member: act.acts_for.clone(),
                operation: "provider.disclose".into(),
                object: object.into(),
                authority: cleared.authority.clone(),
                outcome: outcome.clone(),
                evidence: "acknowledged".into(),
                detail: detail.clone(),
            },
        );

        let cats = detail["categories"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        let grant = cleared.grant.clone().unwrap_or_default();
        let plain = format!(
            "{}, for {}, disclosed {} to {provider} under grant {grant}, in a conversation for this activity only, memory off.",
            self.app_name(&act.app),
            self.member_name(&act.acts_for),
            if cats.is_empty() { "uncategorised content".into() } else { format!("content in {cats}") },
        );
        self.journal(
            Entry::new("disclosure", Source::external(&provider, "provider adapter"), plain)
                .app(&act.app)
                .activity(aid)
                .member(&act.acts_for)
                .operation("provider.disclose")
                .object(object)
                .request_id(&cleared.request_id)
                .outcome(outcome.clone())
                .evidence("acknowledged")
                .authority(cleared.authority.clone())
                .maybe(Entry::grant, cleared.grant.clone())
                .detail(detail.clone()),
        );
        json!({ "request_id": cleared.request_id, "outcome": outcome, "evidence": "acknowledged", "authority": cleared.authority, "detail": detail })
    }

    /// A member loosens a sensitive category for one matter.
    pub fn loosen(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Loosening a category")?;
        let aid = s(body, "activity")?;
        let categories = strings(body, "categories");
        let act = self.activity(&aid)?.clone();
        if !self.in_activity_audience(who, &act) {
            return Err(refuse("outside_audience", format!("activity {aid} is outside {member}'s audience")).activity(&aid).into());
        }
        if let Some(c) = categories.iter().find(|c| self.household.locked.contains(*c)) {
            return Err(refuse("category_locked", format!("{c} is locked for the whole household")).activity(&aid).into());
        }

        self.activities.get_mut(&aid).unwrap().loosened.extend(categories.iter().cloned());
        let plain = format!("{} loosened {} for {aid} only.", self.member_name(&member), categories.join(", "));
        self.journal(
            Entry::new("loosened", Source::principal(&member), plain)
                .app(&act.app)
                .activity(&aid)
                .member(&member)
                .detail(json!({ "categories": categories })),
        );
        Ok(json!({ "activity": aid, "loosened": self.activities[&aid].loosened }))
    }

    /// Lock category (G-16): no member may loosen it for any matter.
    pub fn lock_category(&mut self, who: &Who, body: &Value) -> Res {
        let member = self.shell_member(who, "Locking a category")?;
        if !self.is_owner(&member) {
            return Err(refuse("not_owner", "only the owner locks or unlocks a category").into());
        }
        let category = s(body, "category")?;
        let unlock = body.get("unlock").and_then(Value::as_bool).unwrap_or(false);

        let word = if unlock {
            self.household.locked.remove(&category);
            "unlocked"
        } else {
            self.household.locked.insert(category.clone());
            "locked"
        };
        let plain = format!("{} {word} {category} for the whole household.", self.member_name(&member));
        self.journal(Entry::new("category_lock", Source::principal(&member), plain).member(&member).detail(json!({ "category": category, "locked": !unlock })));
        Ok(json!({ "locked": self.household.locked }))
    }

    pub fn lan(&mut self, who: &Who, body: &Value) -> Res {
        let aid = self.acting_activity(who, body)?;
        let target = s(body, "target")?;
        let job = body.get("job").cloned().unwrap_or(Value::Null);
        let object = format!("lan:{target}");

        let cleared = self.enforce(&aid, "lan.print", &object, &job, opt_s(body, "proposal"), opt_s(body, "grant"))?;
        Ok(self.dispatch(&aid, "lan.print", &object, &job, cleared, json!({ "target": target })))
    }

    /// Starts a new spend period when the old one has run out.
    pub fn roll_allowance(&mut self, app: &str) {
        let now = self.now;
        if let Some(a) = self.apps.get_mut(app).and_then(|a| a.allowance.as_mut()) {
            let length = match a.period.as_str() {
                "day" => Duration::days(1),
                "week" => Duration::weeks(1),
                _ => Duration::days(30),
            };
            if now - a.period_start >= length {
                a.period_start = now;
                a.spent_cents = 0;
            }
        }
    }
}
