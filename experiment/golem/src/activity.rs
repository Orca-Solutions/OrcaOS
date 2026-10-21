// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One coordinator per activity. It checks the journal before it runs a
//! participant, reserves from the shared budget, and records each output with
//! the participant that produced it.
//!
//! Scorecard: counted. Category: grants (budget draw, journal-first check).
//! Tests: 5, 6; the normal run for all.

use golem_rust::{agent_definition, agent_implementation};
use orca_ref::{Command, Source};
use serde_json::{json, Value};

use crate::common::journal;
use crate::journal_agent::JournalClient;
use crate::participants::{DrafterClient, Processor1Client, Processor2AltClient, Processor2Client};

/// What one processor run reserves from the shared budget.
const RUN_CPU_MS: u64 = 60 * 1000;

#[agent_definition]
pub trait Activity {
    fn new(run: String, id: String) -> Self;
    /// A1.1: processor-1 digests c1 and c2.
    async fn run_processor_1(&mut self) -> String;
    /// A1.2, step one or two, by the named holder (processor-2, or the
    /// replacement processor-2b).
    async fn step_processor_2(&mut self, holder: String, step: String) -> String;
    /// Test 6: reserve an amount from the shared budget.
    async fn reserve(&mut self, reservation: String, cpu_ms: u64) -> String;
    /// A1: the drafter builds the proposal.
    async fn draft(&mut self) -> String;
}

struct ActivityImpl {
    run: String,
    id: String,
    reservations: u32,
}

impl ActivityImpl {
    async fn live(&self, holder: &str, capability: &str, object: &str) -> bool {
        JournalClient::get(self.run.clone())
            .live(holder.into(), capability.into(), object.into())
            .await
    }

    async fn reserve_run(&mut self) -> Result<(), String> {
        self.reservations += 1;
        let reservation = format!("{}:reserve:{}", self.id, self.reservations);
        let result = JournalClient::get(self.run.clone())
            .reserve(self.id.clone(), reservation, RUN_CPU_MS)
            .await;

        if result == "reserved" { Ok(()) } else { Err(result) }
    }

    /// Records each object a participant reports writing, attributed to it.
    async fn record_outputs(&self, holder: &str, space: &str, report: &str) -> String {
        let report: Value = serde_json::from_str(report).unwrap_or(json!({ "error": report }));

        if let Some(written) = report["written"].as_array() {
            for item in written {
                let command = Command::Output {
                    holder: holder.into(),
                    activity: self.id.clone(),
                    space: space.into(),
                    object: item["object"].as_str().unwrap_or_default().into(),
                    hash: item["hash"].as_str().unwrap_or_default().into(),
                };
                journal(&self.run, &Source::service(&format!("activity-{}", self.id)), command).await;
            }
        }
        report.to_string()
    }
}

#[agent_implementation]
impl Activity for ActivityImpl {
    fn new(run: String, id: String) -> Self {
        Self { run, id, reservations: 0 }
    }

    async fn run_processor_1(&mut self) -> String {
        if !self.live("processor-1", "read", "c1@v1").await {
            return "refused: processor-1 holds no live grant".into();
        }
        if let Err(e) = self.reserve_run().await {
            return e;
        }

        let report = Processor1Client::get(self.run.clone(), "processor-1".into())
            .process(vec!["c1@v1".into(), "c2@v1".into()])
            .await;
        self.record_outputs("processor-1", "out/A1.1", &report).await
    }

    async fn step_processor_2(&mut self, holder: String, step: String) -> String {
        if !self.live(&holder, "read", "c3@v1").await {
            return format!("refused: {holder} holds no live grant");
        }
        if step == "extract" {
            if let Err(e) = self.reserve_run().await {
                return e;
            }
        }

        let object = "c3@v1".to_string();
        let report = match (holder.as_str(), step.as_str()) {
            ("processor-2", "extract") => Processor2Client::get(self.run.clone(), holder.clone()).extract(object).await,
            ("processor-2", "finish") => Processor2Client::get(self.run.clone(), holder.clone()).finish(object).await,
            ("processor-2b", "extract") => Processor2AltClient::get(self.run.clone(), holder.clone()).extract(object).await,
            ("processor-2b", "finish") => Processor2AltClient::get(self.run.clone(), holder.clone()).finish(object).await,
            _ => return format!("unknown holder or step: {holder} {step}"),
        };
        self.record_outputs(&holder, "out/A1.2", &report).await
    }

    async fn reserve(&mut self, reservation: String, cpu_ms: u64) -> String {
        JournalClient::get(self.run.clone()).reserve(self.id.clone(), reservation, cpu_ms).await
    }

    async fn draft(&mut self) -> String {
        if !self.live("drafter", "submit-proposal", "approval-path").await {
            return "refused: drafter holds no live grant".into();
        }

        let report = DrafterClient::get(self.run.clone(), "drafter".into()).draft(self.id.clone()).await;
        let value: Value = serde_json::from_str(&report).unwrap_or(json!({ "error": report }));
        if let Some(hash) = value["draft_hash"].as_str() {
            let command = Command::Output {
                holder: "drafter".into(),
                activity: self.id.clone(),
                space: "out/A1".into(),
                object: "draft".into(),
                hash: hash.into(),
            };
            journal(&self.run, &Source::service(&format!("activity-{}", self.id)), command).await;
        }
        value.to_string()
    }
}
