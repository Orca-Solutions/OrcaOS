// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Test controls. These belong to the fake, not to the contract: they set up
//! a household, move the clock, put mail in a mailbox and arm the mock
//! services' faults. No app should call them; the real runtime will not have
//! them.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use crate::fake::*;
use crate::mock::{Fault, Page};
use crate::model::*;

impl Fake {
    pub fn control(&mut self, path: &str, body: &Value) -> Res {
        match path {
            "setup" => self.setup(body),
            "reset" => {
                *self = Fake::new();
                Ok(json!({ "reset": true }))
            }
            "now" => Ok(json!({ "now": stamp(self.now), "local": crate::schedule::local(&self.household.zone, self.now) })),
            "clock/set" => {
                let at = time(body, "at")?.ok_or_else(|| bad("missing `at`"))?;
                self.move_clock(at, body)
            }
            "clock/advance" => {
                let n = |k: &str| body.get(k).and_then(Value::as_i64).unwrap_or(0);
                let step = Duration::minutes(n("minutes")) + Duration::hours(n("hours")) + Duration::days(n("days"));
                let at = self.now + step;
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
                let page = Page { body: opt_s(body, "body").unwrap_or_default(), redirect_to: opt_s(body, "redirect_to") };
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
            "ledger" => Ok(json!({ "entries": self.ledger })),
            "mock" => Ok(self.mock.summary()),
            "state" => Ok(json!({
                "now": stamp(self.now),
                "household": self.household,
                "connections": self.connections,
                "apps": self.apps,
                "activities": self.activities,
                "grants": self.grants,
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
        if at < self.now {
            return Err(bad("the clock only moves forward"));
        }
        let asleep = body.get("asleep").and_then(Value::as_bool).unwrap_or(false);
        let from = self.ledger.len();
        self.advance_to(at, asleep);
        let happened: Vec<&crate::ledger::Entry> = self.ledger[from..].iter().collect();
        Ok(json!({ "now": stamp(self.now), "entries": happened }))
    }

    /// Starts over with a household, its connections and the mock services' content.
    fn setup(&mut self, body: &Value) -> Res {
        *self = Fake::new();
        if let Some(now) = time(body, "now")? {
            self.now = now;
        }
        if let Some(h) = body.get("household") {
            self.household = serde_json::from_value(h.clone()).map_err(|e| bad(format!("the household could not be read: {e}")))?;
        }
        if self.household.zone.parse::<chrono_tz::Tz>().is_err() {
            return Err(bad(format!("{} is not a time zone", self.household.zone)));
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
        Ok(json!({ "now": stamp(self.now), "members": self.household.members.len() }))
    }
}
