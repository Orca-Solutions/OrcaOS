// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scenario A on the conventional composition. See ../README.md.
//!
//!   conventional worker      Temporal worker: the activity coordinators
//!   conventional adapter     Temporal worker: the mail adapter (holds the credential)
//!   conventional approval    the approval service
//!   conventional ctl RUN OP  the principal's controls and the platform's tools
//!
//! Scorecard: process wiring, not counted (as lib.rs in the Golem build).

mod adapter;
mod approval;
mod common;
mod containers;
mod coordinator;
mod journal;
mod supervisor;

use serde_json::{json, Value};
use temporalio_sdk::{Runtime, Worker, WorkerOptions};

use common::{env, temporal, ADAPTER_QUEUE, COORDINATOR_QUEUE};

async fn ctl(run: &str, op: &str, rest: &[String]) -> anyhow::Result<Value> {
    let arg = |i: usize| rest.get(i).map(String::as_str).unwrap_or_default();

    Ok(match op {
        "init-db" => {
            common::db().await?.batch_execute(include_str!("../schema.sql")).await?;
            json!("ok")
        }
        "setup" => supervisor::setup(run).await?,
        "grant" => supervisor::grant(run, arg(0), arg(1), arg(2), arg(3), arg(4)).await?,
        "revoke" => supervisor::revoke(run, arg(0)).await?,
        "cancel" => supervisor::cancel(run, arg(0)).await?,
        "close" => supervisor::close(run, arg(0)).await?,
        "replace" => supervisor::replace(run, arg(0), arg(1), arg(2)).await?,
        "record-denials" => supervisor::record_denials(run, arg(0)).await?,
        "export" => journal::export(run).await?,
        "mounts" => {
            let mounts = containers::mounts(run, arg(0)).await?;
            json!(mounts.iter().map(|(_, inside, writable)| json!({ "path": inside, "writable": writable })).collect::<Vec<_>>())
        }
        // Runs a participant directly, past its coordinator, as test 1 and
        // test 5 do; the container still mounts only its live grants.
        "participant" => containers::run(run, arg(0), &rest[1..]).await?,
        other => anyhow::bail!("unknown control {other}"),
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("worker") | Some("adapter") => {
            let logging = temporalio_common::telemetry::Logger::Console { filter: env("ORCA_LOG", "warn"), format: None };
            let telemetry = temporalio_common::telemetry::TelemetryOptions::builder().logging(logging).build();
            let runtime = Runtime::from_current_tokio(temporalio_sdk::runtime::RuntimeOptions::builder().telemetry_options(telemetry).build().map_err(anyhow::Error::msg)?)?;
            let options = if args[0] == "worker" {
                WorkerOptions::new(COORDINATOR_QUEUE)
                    .register_workflow::<coordinator::Coordinate>()?
                    .register_activities(coordinator::Coordinator)
                    .build()
            } else {
                let credential = std::fs::read_to_string(env("MOCK_MAIL_CREDENTIAL_FILE", "/opt/baseline/run/mail-credential"))?;
                let http = reqwest::Client::builder().no_proxy().build()?;
                let adapter = adapter::MailAdapter { credential: credential.trim().into(), http };
                WorkerOptions::new(ADAPTER_QUEUE)
                    .register_workflow::<adapter::DeliveryWorkflow>()?
                    .register_activities(adapter)
                    .build()
            };
            Worker::new(&runtime, temporal().await?, options)?.run().await?;
        }
        Some("approval") => approval::serve().await?,
        Some("ctl") if args.len() >= 3 => {
            let out = ctl(&args[1], &args[2], &args[3..]).await?;
            println!("{out}");
        }
        _ => {
            eprintln!("usage: conventional worker|adapter|approval|ctl RUN OP ARGS (see src/main.rs)");
            std::process::exit(2);
        }
    }
    Ok(())
}
