// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Test controls, the same ones the fake answers (interfaces/diamond-dog/
//! fake-fixtures.md), so the runtime runs the same fixtures. They set up a
//! household, move the clock, put mail in a mailbox and arm the mock
//! services' faults. They belong to the fixtures, not to the contract: the
//! fixture runner uses them in-process, and `serve` answers them over HTTP
//! only when started with `--test-controls`. No app should call them.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use crate::runtime::*;
use crate::mock::{Fault, Page};
use crate::model::*;

impl Runtime {
    pub fn control(&mut self, path: &str, body: &Value) -> Res {
        match path {
            "setup" => self.setup(body),
            "reset" => {
                *self = Runtime::new();
                Ok(json!({ "reset": true }))
            }
            "now" => Ok(json!({ "now": stamp(self.now()), "local": crate::schedule::local(&self.household.zone, self.now()) })),
            "clock/set" => {
                let at = time(body, "at")?.ok_or_else(|| bad("missing `at`"))?;
                self.move_clock(at, body)
            }
            "clock/advance" => {
                let n = |k: &str| body.get(k).and_then(Value::as_i64).unwrap_or(0);
                let step = Duration::minutes(n("minutes")) + Duration::hours(n("hours")) + Duration::days(n("days"));
                let at = self.now() + step;
                self.move_clock(at, body)
            }
            "mail/incoming" => {
                let handle = s(body, "handle")?;
                let mailbox = self.mock.mail.get_mut(&handle).ok_or_else(|| bad(format!("there is no mail connection {handle}")))?;
                let message = json!({
                    "from": body.get("from").cloned().unwrap_or(Value::Null),
                    "subject": body.get("subject").cloned().unwrap_or(Value::Null),
                    "body": body.get("body").cloned().unwrap_or(Value::Null),
                });
                mailbox.inbox.push(message);
                Ok(json!({ "inbox_count": mailbox.inbox.len() }))
            }
            "mail/fault" => {
                let handle = s(body, "handle")?;
                let fault = Fault::parse(&s(body, "fault")?).ok_or_else(|| bad("`fault` is never_acknowledge, refuse, drop_request or status_unavailable"))?;
                let count = body.get("count").and_then(Value::as_u64).unwrap_or(1);
                let mailbox = self.mock.mail.get_mut(&handle).ok_or_else(|| bad(format!("there is no mail connection {handle}")))?;
                mailbox.faults.extend(std::iter::repeat_n(fault, count as usize));
                Ok(json!({ "armed": mailbox.faults }))
            }
            "web/page" => {
                let host = s(body, "host")?;
                let path = opt_s(body, "path").unwrap_or_else(|| "/".into());
                let page = Page {
                    body: opt_s(body, "body").unwrap_or_default(),
                    content_type: opt_s(body, "content_type").unwrap_or_else(|| "text/html".into()),
                    redirect_to: opt_s(body, "redirect_to"),
                };
                self.mock.web.entry(host).or_default().insert(path, page);
                Ok(json!({ "ok": true }))
            }
            "provider" => {
                let name = s(body, "name")?;
                let price = body.get("price_cents").and_then(Value::as_u64).unwrap_or(2);
                self.mock.add_connection("provider", &name, "");
                self.mock.providers.get_mut(&name).unwrap().price_cents = price;
                Ok(json!({ "provider": name, "price_cents": price }))
            }
            "ledger" => Ok(json!({ "entries": self.ledger() })),
            "mock" => Ok(self.mock.summary()),
            "state" => Ok(json!({
                "now": stamp(self.now()),
                "household": self.household,
                "connections": self.connections,
                "apps": self.apps,
                "activities": self.core.activities().collect::<Vec<_>>(),
                "grants": self.core.grants().collect::<Vec<_>>(),
                "proposals": self.proposals,
                "receipts": self.receipts,
                "records": self.records,
                "schedules": self.schedules,
                "contributions": self.contributions,
            })),
            _ => Err(bad(format!("there is no test control `{path}`"))),
        }
    }

    fn move_clock(&mut self, at: DateTime<Utc>, body: &Value) -> Res {
        if at < self.now() {
            return Err(bad("the clock only moves forward"));
        }
        let asleep = body.get("asleep").and_then(Value::as_bool).unwrap_or(false);
        let from = self.ledger().len();
        self.advance_to(at, asleep);
        let happened: Vec<&crate::ledger::Entry> = self.ledger()[from..].iter().collect();
        Ok(json!({ "now": stamp(self.now()), "entries": happened }))
    }

    /// Starts over with a household, its connections and the mock services' content.
    fn setup(&mut self, body: &Value) -> Res {
        *self = Runtime::new();
        if let Some(now) = time(body, "now")? {
            self.core.clock.now = now;
        }
        self.configure(body)
    }

    /// Sets the household, its connections and the mock services' content,
    /// keeping the journal. `serve --setup` uses it at start-up: the contract
    /// has no call that adds members, so the household is configuration.
    pub fn configure(&mut self, body: &Value) -> Res {
        if let Some(h) = body.get("household") {
            let household: Household = serde_json::from_value(h.clone()).map_err(|e| bad(format!("the household could not be read: {e}")))?;
            if household.zone.parse::<chrono_tz::Tz>().is_err() {
                return Err(bad(format!("{} is not a time zone", household.zone)));
            }
            self.set_household(household);
        }

        for c in body.get("connections").and_then(Value::as_array).cloned().unwrap_or_default() {
            let c: Connection = serde_json::from_value(c).map_err(|e| bad(format!("a connection could not be read: {e}")))?;
            self.mock.add_connection(&c.kind, &c.handle, &c.account);
            self.connections.insert(c.handle.clone(), c);
        }
        for (name, p) in body.get("providers").and_then(Value::as_object).cloned().unwrap_or_default() {
            self.control("provider", &json!({ "name": name, "price_cents": p.get("price_cents") }))?;
        }
        for page in body.get("web").and_then(Value::as_array).cloned().unwrap_or_default() {
            self.control("web/page", &page)?;
        }
        for mail in body.get("mail").and_then(Value::as_array).cloned().unwrap_or_default() {
            self.control("mail/incoming", &mail)?;
        }
        Ok(json!({ "now": stamp(self.now()), "members": self.household.members.len() }))
    }
}
