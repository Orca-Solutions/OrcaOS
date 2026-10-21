// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One run of Scenario A, step by step, as the facilitator would drive it.
//! Every run has its own identifier, so its agents, journal and mock records
//! are its own.
//!
//! Scorecard: excluded (test harness and facilitator controls).

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::golem::{lit, Golem};
use crate::mock::Mock;

pub struct Run<'a> {
    pub id: String,
    pub golem: &'a Golem,
    pub mock: &'a Mock,
}

pub struct Approved {
    pub authorization: String,
    pub expires_at_ms: u64,
    pub message: Value,
}

impl<'a> Run<'a> {
    pub fn new(id: String, golem: &'a Golem, mock: &'a Mock) -> Self {
        Self { id, golem, mock }
    }

    pub fn agent(&self, kind: &str) -> String {
        format!("{kind}({})", lit(&self.id))
    }

    pub fn agent2(&self, kind: &str, second: &str) -> String {
        format!("{kind}({},{})", lit(&self.id), lit(second))
    }

    pub fn call(&self, agent: &str, method: &str, args: &[&str]) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        self.golem.invoke(agent, method, &args)
    }

    pub fn supervisor(&self, method: &str, args: &[&str]) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|a| lit(a)).collect();
        self.golem.invoke(&self.agent("Supervisor"), method, &args)
    }

    pub fn journal(&self) -> Value {
        self.call(&self.agent("Journal"), "export", &[])
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Seeds the document store and records the grant table.
    pub fn setup(&self) -> Result<Value, String> {
        let loaded = self.call(&self.agent("Seeder"), "load", &[])?;
        self.supervisor("setup", &[])?;
        serde_json::from_str(&loaded).map_err(|e| e.to_string())
    }

    /// Runs both children and the drafter. Returns the proposal identifier.
    pub fn process_and_draft(&self) -> Result<String, String> {
        self.call(&self.agent2("Activity", "A1.1"), "run_processor_1", &[])?;
        for step in ["extract", "finish"] {
            self.call(&self.agent2("Activity", "A1.2"), "step_processor_2", &[&lit("processor-2"), &lit(step)])?;
        }
        let draft: Value = serde_json::from_str(&self.call(&self.agent2("Activity", "A1"), "draft", &[])?).map_err(|e| e.to_string())?;
        draft["proposal"].as_str().map(str::to_string).ok_or_else(|| format!("no proposal: {draft}"))
    }

    pub fn display(&self, proposal: &str) -> Result<Value, String> {
        let shown = self.call(&self.agent("ApprovalPath"), "display", &[&lit(proposal)])?;
        serde_json::from_str(&shown).map_err(|e| e.to_string())
    }

    /// The principal approves what the approval path displayed.
    pub fn approve(&self, proposal: &str, grant: &str) -> Result<Approved, String> {
        let shown = self.display(proposal)?;
        let hash = shown["payload_hash"].as_str().ok_or("no payload hash")?.to_string();
        let out = self.call(&self.agent("ApprovalPath"), "approve", &[&lit(proposal), &lit(&hash), &lit(grant)])?;
        let out: Value = serde_json::from_str(&out).map_err(|_| out.clone())?;

        Ok(Approved {
            authorization: out["authorization"].as_str().ok_or("no authorization")?.to_string(),
            expires_at_ms: out["expires_at_ms"].as_u64().unwrap_or_default(),
            message: shown["message"].clone(),
        })
    }

    /// Hands a message to the adapter directly, as a tampered path would.
    pub fn deliver_directly(&self, authorization: &str, expires_at_ms: u64, message: &Value) -> Result<String, String> {
        self.call(
            &self.agent("MailAdapter"),
            "deliver",
            &[&lit("A1"), &lit(authorization), &expires_at_ms.to_string(), &lit(&message.to_string())],
        )
    }

    /// Waits until the adapter reaches a hold at `point`.
    pub fn wait_held(&self, point: &str, timeout: Duration) -> Option<u64> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if let Some(h) = self.mock.held(&self.id).iter().find(|h| h["point"] == point) {
                return h["id"].as_u64();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    /// Releases every hold this run still has, including ones whose
    /// connection died with the platform.
    /// Releases this run's holds at one checkpoint only.
    pub fn release_at(&self, point: &str) {
        for h in self.mock.held(&self.id).iter().filter(|h| h["point"] == point) {
            if let Some(id) = h["id"].as_u64() {
                let _ = self.mock.release(id);
            }
        }
    }

    pub fn release_all(&self) {
        for h in self.mock.held(&self.id) {
            if let Some(id) = h["id"].as_u64() {
                let _ = self.mock.release(id);
            }
        }
    }

    /// Polls the journal until `done` holds or the timeout passes.
    pub fn wait_journal(&self, timeout: Duration, done: impl Fn(&Value) -> bool) -> (Value, bool) {
        let started = Instant::now();
        loop {
            let journal = self.journal();
            if done(&journal) {
                return (journal, true);
            }
            if started.elapsed() > timeout {
                return (journal, false);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

/// Journal entries of one kind.
pub fn entries<'v>(journal: &'v Value, kind: &str) -> Vec<&'v Value> {
    journal["journal"]["entries"]
        .as_array()
        .map(|e| e.iter().filter(|e| e["entry"] == kind).collect())
        .unwrap_or_default()
}

pub fn receipt<'v>(journal: &'v Value, request_id: &str) -> &'v Value {
    &journal["journal"]["receipts"][request_id]
}

pub fn outcome_state(journal: &Value, request_id: &str) -> String {
    receipt(journal, request_id)["outcome"]["state"].as_str().unwrap_or("none").to_string()
}

pub fn activity_state(journal: &Value, activity: &str) -> String {
    journal["journal"]["activities"][activity]["state"].as_str().unwrap_or("none").to_string()
}
