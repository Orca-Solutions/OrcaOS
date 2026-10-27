// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The journal writer as a Golem agent: one per run, applying one command at a
//! time, so it is the single point of order. Golem runs an agent's invocations
//! one after another and persists each in the agent's oplog; the forced commit
//! makes "the record commits" mean the oplog write reached storage before the
//! caller sees the response.
//!
//! The shared budget is Golem-specific: Golem's quotas reserve capacity only
//! for the length of one invocation, so they cannot hold a budget that
//! children draw from over the life of an activity.
//!
//! Scorecard: counted. Category: grants (budget ledger, hosting the journal).
//! Tests: all; the budget ledger serves test 6.

use std::collections::BTreeMap;

use golem_rust::{agent_definition, agent_implementation};
use orca_ref::{Command, Journal as Writer, Kind, Response, Source};
use serde::{Deserialize, Serialize};

use crate::common::now_ms;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    /// Root activity to its CPU allowance in milliseconds.
    pub budgets: BTreeMap<String, u64>,
    pub reservations: BTreeMap<String, Reservation>,
    pub log: Vec<LedgerEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reservation {
    pub activity: String,
    pub root: String,
    pub cpu_ms: u64,
    pub released: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub at_ms: u64,
    pub reservation: String,
    pub activity: String,
    pub cpu_ms: u64,
    pub result: String,
}

#[agent_definition]
pub trait Journal {
    fn new(run: String) -> Self;
    /// Applies one command. `source` and `command` are JSON.
    fn apply(&mut self, source: String, command: String) -> String;
    /// Sets a root activity's CPU allowance.
    fn set_budget(&mut self, activity: String, cpu_ms: u64) -> String;
    /// Reserves from the root's allowance, atomically. A reservation
    /// identifier is never reserved twice; repeating it returns the original
    /// result.
    fn reserve(&mut self, activity: String, reservation: String, cpu_ms: u64) -> String;
    /// Whether the holder has a live grant for the capability on the object.
    fn live(&self, holder: String, capability: String, object: String) -> bool;
    fn export(&self) -> String;
}

struct JournalImpl {
    writer: Writer,
    ledger: Ledger,
}

impl JournalImpl {
    fn root(&self, activity: &str) -> String {
        let mut current = activity.to_string();
        while let Some(parent) = self.writer.activities.get(&current).and_then(|a| a.parent.clone()) {
            current = parent;
        }
        current
    }

    /// Reservations the shared journal released (a certified non-start) are
    /// released in the ledger too.
    fn follow_releases(&mut self, from: usize) {
        for entry in &self.writer.entries[from..] {
            if let Kind::ReservationReleased { reservation, .. } = &entry.kind {
                if let Some(r) = self.ledger.reservations.get_mut(reservation) {
                    r.released = true;
                }
            }
        }
    }
}

#[agent_implementation]
impl Journal for JournalImpl {
    fn new(_run: String) -> Self {
        Self {
            writer: Writer::new(),
            ledger: Ledger::default(),
        }
    }

    fn apply(&mut self, source: String, command: String) -> String {
        let response = match (serde_json::from_str::<Source>(&source), serde_json::from_str::<Command>(&command)) {
            (Ok(source), Ok(command)) => {
                let before = self.writer.entries.len();
                let response = self.writer.apply(source, now_ms(), command);
                self.follow_releases(before);
                response
            }
            (Err(e), _) | (_, Err(e)) => Response::Refused {
                reason: format!("malformed request: {e}"),
                seq: None,
            },
        };

        golem_rust::oplog_commit(1);
        serde_json::to_string(&response).unwrap()
    }

    fn set_budget(&mut self, activity: String, cpu_ms: u64) -> String {
        self.ledger.budgets.insert(activity, cpu_ms);
        golem_rust::oplog_commit(1);
        "ok".into()
    }

    fn reserve(&mut self, activity: String, reservation: String, cpu_ms: u64) -> String {
        if let Some(entry) = self.ledger.log.iter().find(|e| e.reservation == reservation) {
            return entry.result.clone();
        }

        let now = now_ms();
        let root = self.root(&activity);
        let allowance = self.ledger.budgets.get(&root).copied().unwrap_or(0);
        let used: u64 = self
            .ledger
            .reservations
            .values()
            .filter(|r| r.root == root && !r.released)
            .map(|r| r.cpu_ms)
            .sum();

        let refusal = if !self.writer.activity_open(&activity) {
            Some(format!("{activity} is not open"))
        } else if used + cpu_ms > allowance {
            Some(format!("{root} has {} of {allowance} CPU-ms left", allowance - used))
        } else {
            None
        };

        let result = match refusal {
            Some(reason) => {
                self.writer.apply(
                    Source::service("budget"),
                    now,
                    Command::Denial {
                        holder: activity.clone(),
                        operation: "reserve".into(),
                        object: format!("{root} budget, {cpu_ms} CPU-ms"),
                        reason: reason.clone(),
                    },
                );
                format!("refused: {reason}")
            }
            None => {
                self.ledger.reservations.insert(
                    reservation.clone(),
                    Reservation {
                        activity: activity.clone(),
                        root,
                        cpu_ms,
                        released: false,
                    },
                );
                "reserved".to_string()
            }
        };

        self.ledger.log.push(LedgerEntry {
            at_ms: now,
            reservation,
            activity,
            cpu_ms,
            result: result.clone(),
        });
        golem_rust::oplog_commit(1);
        result
    }

    fn live(&self, holder: String, capability: String, object: String) -> bool {
        let now = now_ms();
        self.writer.grants.values().any(|g| {
            g.holder == holder
                && g.capability == capability
                && g.object == object
                && self.writer.grant_live(&g.id, now)
                && self.writer.activity_open(&g.activity)
        })
    }

    fn export(&self) -> String {
        serde_json::json!({ "journal": self.writer, "ledger": self.ledger }).to_string()
    }
}
