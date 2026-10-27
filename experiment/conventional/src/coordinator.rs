// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One coordinator per activity step, as a Temporal workflow on the
//! coordinator worker's task queue. It checks the journal before it runs a
//! participant, reserves from the shared budget, and records each output with
//! the participant that produced it. Temporal makes each step durable: a
//! completed activity is not run again when the worker restarts.
//!
//! Scorecard: counted. Category: grants (budget draw, journal-first check,
//! the drafter's proposal). Tests: 5, 6; the normal run for all.

use std::time::Duration;

use orca_ref::{Command, Kind, Source};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::{ActivityOptions, WorkflowContext, WorkflowResult};

use crate::common::env;
use crate::{containers, journal};

/// What one processor run reserves from the shared budget.
const RUN_CPU_MS: u64 = 60 * 1000;

/// One step, named as the Golem build's coordinator methods are.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Step {
    /// A1.1: processor-1 digests c1 and c2.
    #[serde(rename = "run_processor_1")]
    RunProcessor1 { run: String },
    /// A1.2, step one or two, by the named holder (processor-2, or the
    /// replacement processor-2b).
    #[serde(rename = "step_processor_2")]
    StepProcessor2 { run: String, holder: String, step: String },
    /// Test 6: reserve an amount from the shared budget.
    Reserve { run: String, activity: String, reservation: String, cpu_ms: u64 },
    /// A1: the drafter builds the proposal.
    Draft { run: String },
}

#[workflow]
#[derive(Default)]
pub struct Coordinate;

fn options() -> ActivityOptions {
    ActivityOptions::start_to_close_timeout(Duration::from_secs(120))
}

#[workflow_methods]
impl Coordinate {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, step: Step) -> WorkflowResult<String> {
        let live = |run: &str, holder: &str, capability: &str, object: &str| {
            let input = (run.to_string(), holder.to_string(), capability.to_string(), object.to_string());
            ctx.execute_activity(Coordinator::live, input, options())
        };

        let report = match step {
            Step::RunProcessor1 { run } => {
                if !live(&run, "processor-1", "read", "c1@v1").await? {
                    return Ok("refused: processor-1 holds no live grant".into());
                }
                let reservation = format!("A1.1:reserve:{}", ctx.workflow_id());
                let reserved = ctx.execute_activity(Coordinator::reserve, (run.clone(), "A1.1".into(), reservation, RUN_CPU_MS), options()).await?;
                if reserved != "reserved" {
                    return Ok(reserved);
                }

                let args = ["process", "out/A1.1", "c1@v1", "c2@v1"].map(String::from).to_vec();
                let report = ctx.execute_activity(Coordinator::participant, (run.clone(), "processor-1".into(), args), options()).await?;
                ctx.execute_activity(Coordinator::record_outputs, (run, "A1.1".into(), "processor-1".into(), "out/A1.1".into(), report), options()).await?
            }
            Step::StepProcessor2 { run, holder, step } => {
                if !live(&run, &holder, "read", "c3@v1").await? {
                    return Ok(format!("refused: {holder} holds no live grant"));
                }
                if step == "extract" {
                    let reservation = format!("A1.2:reserve:{}", ctx.workflow_id());
                    let reserved = ctx.execute_activity(Coordinator::reserve, (run.clone(), "A1.2".into(), reservation, RUN_CPU_MS), options()).await?;
                    if reserved != "reserved" {
                        return Ok(reserved);
                    }
                }

                let mut args = vec![step.clone(), "out/A1.2".into(), "c3@v1".into()];
                if holder == "processor-2b" && step == "extract" {
                    args.push("alt".into());
                }
                let report = ctx.execute_activity(Coordinator::participant, (run.clone(), holder.clone(), args), options()).await?;
                ctx.execute_activity(Coordinator::record_outputs, (run, "A1.2".into(), holder, "out/A1.2".into(), report), options()).await?
            }
            Step::Reserve { run, activity, reservation, cpu_ms } => {
                ctx.execute_activity(Coordinator::reserve, (run, activity, reservation, cpu_ms), options()).await?
            }
            Step::Draft { run } => {
                if !live(&run, "drafter", "submit-proposal", "approval-path").await? {
                    return Ok("refused: drafter holds no live grant".into());
                }
                let report = ctx.execute_activity(Coordinator::participant, (run.clone(), "drafter".into(), vec!["draft".into()]), options()).await?;
                ctx.execute_activity(Coordinator::propose, (run, report), options()).await?
            }
        };
        Ok(report)
    }
}

pub struct Coordinator;

/// Journals one output, attributed to its producer. Temporal may run an
/// activity again after a crash, so an output already recorded is not
/// recorded twice.
async fn record_output(run: &str, activity: &str, holder: &str, space: &str, object: &str, hash: &str) -> anyhow::Result<()> {
    let recorded = journal::read(run).await?;
    let already = recorded.entries.iter().any(|e| {
        matches!(&e.kind, Kind::Output { holder: h, activity: a, object: o, hash: x, .. }
            if h == holder && a == activity && o == object && x == hash)
    });
    if already {
        return Ok(());
    }

    let command = Command::Output {
        holder: holder.into(),
        activity: activity.into(),
        space: space.into(),
        object: object.into(),
        hash: hash.into(),
    };
    journal::apply(run, &Source::service(&format!("activity-{activity}")), command).await?;
    Ok(())
}

#[activities]
impl Coordinator {
    #[activity]
    pub async fn live(_ctx: ActivityContext, input: (String, String, String, String)) -> Result<bool, ActivityError> {
        let (run, holder, capability, object) = input;
        Ok(journal::live(&run, &holder, &capability, &object).await?)
    }

    #[activity]
    pub async fn reserve(_ctx: ActivityContext, input: (String, String, String, u64)) -> Result<String, ActivityError> {
        let (run, activity, reservation, cpu_ms) = input;
        Ok(journal::reserve(&run, &activity, &reservation, cpu_ms).await?)
    }

    #[activity]
    pub async fn participant(_ctx: ActivityContext, input: (String, String, Vec<String>)) -> Result<Value, ActivityError> {
        let (run, holder, args) = input;
        Ok(containers::run(&run, &holder, &args).await?)
    }

    /// Records each object a participant reports writing, attributed to it.
    #[activity]
    pub async fn record_outputs(_ctx: ActivityContext, input: (String, String, String, String, Value)) -> Result<String, ActivityError> {
        let (run, activity, holder, space, report) = input;

        for item in report["written"].as_array().into_iter().flatten() {
            let (object, hash) = (item["object"].as_str().unwrap_or_default(), item["hash"].as_str().unwrap_or_default());
            record_output(&run, &activity, &holder, &space, object, hash).await?;
        }
        Ok(report.to_string())
    }

    /// Submits the drafter's draft as its proposal. The drafter has no
    /// network; the coordinator reads the draft from the drafter's output
    /// object and submits it for the drafter. The approval service checks the
    /// drafter's grant when it receives the proposal.
    #[activity]
    pub async fn propose(ctx: ActivityContext, input: (String, Value)) -> Result<String, ActivityError> {
        let (run, report) = input;
        let Some(hash) = report["draft_hash"].as_str() else {
            return Ok(report.to_string());
        };
        let message = std::fs::read_to_string(containers::stores(&run).join("out/A1/draft"))?;

        // A retried attempt keeps the workflow and activity identifiers, so
        // the approval service returns the proposal it already made.
        let submission = format!("{}/{}", ctx.info().workflow_id.clone().unwrap_or_default(), ctx.info().activity_id);
        let submit = json!({ "run": run, "activity": "A1", "holder": "drafter", "message": message, "submission": submission });
        let proposal = reqwest::Client::builder()
            .no_proxy()
            .build()?
            .post(format!("{}/submit", env("ORCA_APPROVAL", "http://127.0.0.1:18090")))
            .json(&submit)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        record_output(&run, "A1", "drafter", "out/A1", "draft", hash).await?;
        Ok(json!({ "proposal": proposal, "draft_hash": hash }).to_string())
    }
}
