// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The mock mail service's ground truth: what arrived, what it received, what
//! it delivered, and the faults and checkpoints the harness has armed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Submit {
    pub request_id: String,
    pub not_after_ms: u64,
    pub message: orca_ref::Message,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SubmitReply {
    Accepted { message_id: String },
    Refused { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Submit,
    Status,
}

/// The fault switches the protocol names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// The request is lost in transit: it never reaches the service.
    DropRequest,
    /// The service processes the request; its reply is lost.
    DropReply,
    /// The reply arrives after `delay_ms`.
    DelayReply,
    /// The service refuses the request.
    Refuse,
    /// The endpoint answers 503.
    Offline,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fault {
    pub target: Target,
    pub action: Action,
    #[serde(default)]
    pub rid_prefix: String,
    #[serde(default)]
    pub delay_ms: u64,
    /// How many more requests it applies to; `None` until cleared.
    #[serde(default)]
    pub remaining: Option<u32>,
}

/// Holds the adapter at a named crash or pause point until the harness
/// releases it, or kills the platform while it is held.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointRule {
    pub point: String,
    #[serde(default)]
    pub rid_prefix: String,
    #[serde(default)]
    pub remaining: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Held {
    pub id: u64,
    pub point: String,
    pub request_id: String,
    pub at_ms: u64,
}

struct Request {
    not_after_ms: u64,
    reply: SubmitReply,
}

pub struct Logs {
    dir: Option<std::path::PathBuf>,
    files: BTreeMap<&'static str, File>,
    pub entries: BTreeMap<&'static str, Vec<Value>>,
}

pub const LOGS: [&str; 5] = ["wire", "attempts", "deliveries", "statuses", "checkpoints"];

impl Logs {
    fn open(dir: Option<&Path>) -> Self {
        let mut logs = Self {
            dir: dir.map(Path::to_path_buf),
            files: BTreeMap::new(),
            entries: LOGS.iter().map(|name| (*name, Vec::new())).collect(),
        };
        logs.reopen();
        logs
    }

    fn reopen(&mut self) {
        self.files.clear();
        let Some(dir) = &self.dir else { return };

        std::fs::create_dir_all(dir).expect("log directory");
        for name in LOGS {
            let path = dir.join(format!("{name}.jsonl"));
            let file = OpenOptions::new().create(true).append(true).open(path).expect("log file");
            self.files.insert(name, file);
        }
    }

    /// Appends and flushes before the caller does anything else, so a log entry
    /// always precedes the effect it describes.
    pub fn append(&mut self, log: &'static str, mut entry: Value) {
        let seq = self.entries[log].len() + 1;
        entry["seq"] = json!(seq);
        entry["at_ms"] = json!(now_ms());

        if let Some(file) = self.files.get_mut(log) {
            writeln!(file, "{entry}").expect("log write");
            file.sync_data().expect("log sync");
        }
        self.entries.get_mut(log).unwrap().push(entry);
    }
}

pub struct State {
    pub credential: String,
    pub logs: Logs,
    requests: BTreeMap<String, Request>,
    /// Identifiers certified not started can never start afterwards.
    burned: BTreeSet<String>,
    faults: Vec<Fault>,
    checkpoint_rules: Vec<CheckpointRule>,
    pub held: BTreeMap<u64, Held>,
    pub released: BTreeSet<u64>,
    next_held: u64,
    next_message: u64,
}

/// What the service decided to do with one submission.
pub enum SubmitOutcome {
    Reply { reply: SubmitReply, delay_ms: u64 },
    DropReply,
    DropRequest,
    Unauthenticated,
}

impl State {
    pub fn new(credential: String, log_dir: Option<&Path>) -> Self {
        Self {
            credential,
            logs: Logs::open(log_dir),
            requests: BTreeMap::new(),
            burned: BTreeSet::new(),
            faults: Vec::new(),
            checkpoint_rules: Vec::new(),
            held: BTreeMap::new(),
            released: BTreeSet::new(),
            next_held: 0,
            next_message: 0,
        }
    }

    pub fn reset(&mut self) {
        let credential = std::mem::take(&mut self.credential);
        let dir = self.logs.dir.clone();

        if let Some(dir) = &dir {
            for name in LOGS {
                let _ = std::fs::remove_file(dir.join(format!("{name}.jsonl")));
            }
        }
        *self = Self::new(credential, dir.as_deref());
    }

    pub fn add_fault(&mut self, fault: Fault) {
        self.faults.push(fault);
    }

    pub fn clear_faults(&mut self) {
        self.faults.clear();
    }

    pub fn add_checkpoint_rule(&mut self, rule: CheckpointRule) {
        self.checkpoint_rules.push(rule);
    }

    pub fn clear_checkpoint_rules(&mut self) {
        self.checkpoint_rules.clear();
    }

    fn take_fault(&mut self, target: Target, request_id: &str, wanted: &[Action]) -> Option<Fault> {
        let index = self.faults.iter().position(|f| {
            f.target == target
                && wanted.contains(&f.action)
                && request_id.starts_with(&f.rid_prefix)
                && f.remaining != Some(0)
        })?;
        let fault = &mut self.faults[index];
        if let Some(remaining) = &mut fault.remaining {
            *remaining -= 1;
        }

        Some(fault.clone())
    }

    /// `transport_key` is the platform's own idempotency header, if it sends
    /// one; it shows whether a platform resent a request by itself.
    pub fn submit(&mut self, authorized: bool, submit: Submit, transport_key: Option<String>) -> SubmitOutcome {
        let payload_hash = orca_ref::payload_hash(&submit.message);
        let rid = submit.request_id.clone();

        // The edge of the network sees every request, including those lost in transit.
        let lost = self.take_fault(Target::Submit, &rid, &[Action::DropRequest]);
        self.logs.append(
            "wire",
            json!({
                "request_id": rid,
                "payload_hash": payload_hash,
                "lost_in_transit": lost.is_some(),
                "transport_key": transport_key,
            }),
        );
        if lost.is_some() {
            return SubmitOutcome::DropRequest;
        }

        // Every submission the service receives, before and independently of deduplication.
        self.logs.append(
            "attempts",
            json!({ "request_id": rid, "payload_hash": payload_hash, "authenticated": authorized }),
        );
        if !authorized {
            return SubmitOutcome::Unauthenticated;
        }

        let reply = self.process(&submit, &payload_hash);
        let fault = self.take_fault(
            Target::Submit,
            &rid,
            &[Action::DropReply, Action::DelayReply],
        );
        match fault {
            Some(Fault {
                action: Action::DropReply,
                ..
            }) => SubmitOutcome::DropReply,
            Some(Fault { delay_ms, .. }) => SubmitOutcome::Reply { reply, delay_ms },
            None => SubmitOutcome::Reply { reply, delay_ms: 0 },
        }
    }

    fn process(&mut self, submit: &Submit, payload_hash: &str) -> SubmitReply {
        let rid = &submit.request_id;

        // Repeating an identifier returns the original result, with no second delivery.
        if let Some(request) = self.requests.get(rid) {
            return request.reply.clone();
        }

        let now = now_ms();
        let refusal = if self.burned.contains(rid) {
            Some("request identifier was certified not started".to_string())
        } else if now >= submit.not_after_ms {
            Some("request identifier's validity window has closed".to_string())
        } else {
            self.take_fault(Target::Submit, rid, &[Action::Refuse])
                .map(|_| "refused by the service".to_string())
        };

        let reply = match refusal {
            Some(reason) => SubmitReply::Refused { reason },
            None => {
                self.next_message += 1;
                let message_id = format!("msg-{:04}", self.next_message);
                self.logs.append(
                    "deliveries",
                    json!({
                        "request_id": rid,
                        "message_id": message_id,
                        "payload_hash": payload_hash,
                        "to": submit.message.to,
                    }),
                );
                SubmitReply::Accepted { message_id }
            }
        };

        // A refusal for a closed window does not claim the identifier.
        if !matches!(&reply, SubmitReply::Refused { reason } if reason.contains("validity window")) {
            self.requests.insert(
                rid.clone(),
                Request {
                    not_after_ms: submit.not_after_ms,
                    reply: reply.clone(),
                },
            );
        }

        reply
    }

    /// `None` means the endpoint is offline.
    pub fn status(&mut self, rid: &str, not_after_ms: u64) -> Option<Value> {
        if self.take_fault(Target::Status, rid, &[Action::Offline]).is_some() {
            self.logs.append("statuses", json!({ "request_id": rid, "offline": true }));
            return None;
        }

        let status = match self.requests.get(rid) {
            Some(Request {
                reply: SubmitReply::Accepted { message_id },
                ..
            }) => json!({ "status": "accepted", "message_id": message_id }),
            Some(Request {
                reply: SubmitReply::Refused { reason },
                ..
            }) => json!({ "status": "refused", "reason": reason }),
            None => {
                // The contract: "not received" after the window closes certifies
                // that the request did not start and can no longer start.
                let certified = now_ms() >= not_after_ms;
                if certified {
                    self.burned.insert(rid.to_string());
                }
                json!({ "status": "not_received", "certified": certified })
            }
        };

        let window = self.requests.get(rid).map(|r| r.not_after_ms);
        self.logs.append(
            "statuses",
            json!({ "request_id": rid, "offline": false, "reply": status, "window_ms": window }),
        );
        Some(status)
    }

    /// Registers an adapter arriving at a checkpoint. Returns the hold id when
    /// an armed rule matches.
    pub fn checkpoint(&mut self, point: &str, request_id: &str) -> Option<u64> {
        let index = self.checkpoint_rules.iter().position(|r| {
            r.point == point && request_id.starts_with(&r.rid_prefix) && r.remaining != Some(0)
        });
        let held = index.is_some();

        if let Some(index) = index {
            if let Some(remaining) = &mut self.checkpoint_rules[index].remaining {
                *remaining -= 1;
            }
        }
        self.logs.append(
            "checkpoints",
            json!({ "point": point, "request_id": request_id, "held": held }),
        );
        if !held {
            return None;
        }

        self.next_held += 1;
        let id = self.next_held;
        self.held.insert(
            id,
            Held {
                id,
                point: point.into(),
                request_id: request_id.into(),
                at_ms: now_ms(),
            },
        );
        Some(id)
    }
}
