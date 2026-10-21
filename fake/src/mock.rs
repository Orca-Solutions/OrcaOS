// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The outside systems the fake's adapters talk to: mail, calendar, web
//! hosts, model providers and LAN targets. They live in the same process and
//! keep the ground truth a test compares the ledger against. The mail service
//! follows the Scenario A mock mail contract: a submission carries a request
//! identifier, and a status check by that identifier is how an unknown
//! outcome is reconciled. Nothing here is ever re-sent.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

/// What a submission returned to the adapter.
pub enum Submitted {
    Accepted { message_id: String },
    Refused { reason: String },
    /// No reply arrived; the request may or may not have left.
    NoReply,
}

/// What a status check by request identifier returned.
pub enum Status {
    Accepted { message_id: String },
    Refused { reason: String },
    /// The service certifies it never received the request and never will
    /// under that identifier.
    NotReceived,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    /// The service accepts and delivers; its reply is lost.
    NeverAcknowledge,
    /// The service refuses the request.
    Refuse,
    /// The request is lost before it arrives.
    DropRequest,
    /// The status endpoint does not answer.
    StatusUnavailable,
}

impl Fault {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "never_acknowledge" => Self::NeverAcknowledge,
            "refuse" => Self::Refuse,
            "drop_request" => Self::DropRequest,
            "status_unavailable" => Self::StatusUnavailable,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Mailbox {
    pub account: String,
    pub inbox: Vec<Value>,
    /// Every submission that arrived, by request identifier.
    pub arrived: BTreeMap<String, Value>,
    pub deliveries: Vec<Value>,
    pub refused: Vec<String>,
    /// Armed faults, applied to the next submissions in order.
    pub faults: Vec<Fault>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Provider {
    pub price_cents: u64,
    pub disclosures: Vec<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Page {
    pub body: String,
    pub content_type: String,
    pub redirect_to: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Mock {
    pub mail: BTreeMap<String, Mailbox>,
    pub calendar: BTreeMap<String, Vec<Value>>,
    pub web: BTreeMap<String, BTreeMap<String, Page>>,
    pub fetches: Vec<Value>,
    pub providers: BTreeMap<String, Provider>,
    pub lan: BTreeMap<String, Vec<Value>>,
    pub promoted: Vec<Value>,
    next_message: u64,
}

impl Mock {
    pub fn add_connection(&mut self, kind: &str, handle: &str, account: &str) {
        match kind {
            "mail" => {
                self.mail.entry(handle.into()).or_insert_with(|| Mailbox {
                    account: account.into(),
                    ..Default::default()
                });
            }
            "calendar" => {
                self.calendar.entry(handle.into()).or_default();
            }
            "lan" => {
                self.lan.entry(handle.into()).or_default();
            }
            "provider" => {
                self.providers.entry(handle.into()).or_insert(Provider {
                    price_cents: 2,
                    disclosures: Vec::new(),
                });
            }
            _ => {}
        }
    }

    pub fn submit_mail(&mut self, handle: &str, request_id: &str, message: &Value) -> Submitted {
        let Some(mailbox) = self.mail.get_mut(handle) else {
            return Submitted::Refused { reason: format!("no mail account behind {handle}") };
        };
        let fault = (!mailbox.faults.is_empty()).then(|| mailbox.faults.remove(0));

        if fault == Some(Fault::DropRequest) {
            return Submitted::NoReply;
        }

        // A request identifier seen before is never delivered twice.
        if mailbox.arrived.contains_key(request_id) {
            return Submitted::Refused { reason: "duplicate request identifier".into() };
        }
        mailbox.arrived.insert(request_id.into(), message.clone());

        if fault == Some(Fault::Refuse) {
            mailbox.refused.push(request_id.into());
            return Submitted::Refused { reason: "the mail service refused the message".into() };
        }

        self.next_message += 1;
        let message_id = format!("msg-{}", self.next_message);
        let mailbox = self.mail.get_mut(handle).unwrap();
        mailbox.deliveries.push(json!({
            "request_id": request_id,
            "message_id": message_id,
            "message": message,
        }));

        match fault {
            Some(Fault::NeverAcknowledge) => Submitted::NoReply,
            _ => Submitted::Accepted { message_id },
        }
    }

    pub fn mail_status(&mut self, handle: &str, request_id: &str) -> Status {
        let Some(mailbox) = self.mail.get_mut(handle) else {
            return Status::Unavailable;
        };
        if mailbox.faults.first() == Some(&Fault::StatusUnavailable) {
            mailbox.faults.remove(0);
            return Status::Unavailable;
        }
        if mailbox.refused.iter().any(|r| r == request_id) {
            return Status::Refused { reason: "the mail service refused the message".into() };
        }

        let delivered = mailbox.deliveries.iter().find(|d| d["request_id"] == request_id);
        match delivered {
            Some(d) => Status::Accepted { message_id: d["message_id"].as_str().unwrap_or_default().into() },
            None => Status::NotReceived,
        }
    }

    pub fn summary(&self) -> Value {
        let mail: BTreeMap<_, _> = self
            .mail
            .iter()
            .map(|(handle, m)| {
                (
                    handle.clone(),
                    json!({
                        "account": m.account,
                        "inbox_count": m.inbox.len(),
                        "arrived_count": m.arrived.len(),
                        "delivered_count": m.deliveries.len(),
                        "deliveries": m.deliveries,
                        "armed_faults": m.faults,
                    }),
                )
            })
            .collect();
        let providers: BTreeMap<_, _> = self
            .providers
            .iter()
            .map(|(name, p)| {
                (
                    name.clone(),
                    json!({
                        "price_cents": p.price_cents,
                        "disclosure_count": p.disclosures.len(),
                        "disclosures": p.disclosures,
                    }),
                )
            })
            .collect();

        json!({
            "mail": mail,
            "calendar": self.calendar,
            "fetches": self.fetches,
            "fetch_count": self.fetches.len(),
            "providers": providers,
            "lan": self.lan,
            "promoted": self.promoted,
        })
    }
}
