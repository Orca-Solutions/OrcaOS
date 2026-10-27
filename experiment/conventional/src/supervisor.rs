// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Acts for principal-1: opens the activities, records the grant table,
//! revokes, cancels, closes and replaces participants. Every change is a
//! journal command; the container runner reads the journal's live grants
//! each time it starts a participant, so a revoked grant mounts nothing.
//!
//! Scorecard: counted. Category: grants. Tests: 3, 5, 7b; setup for all.

use orca_ref::{ActivityState, Command, Response, Source};
use serde_json::{json, Value};

use crate::common::{now_ms, PRINCIPAL};
use crate::journal::{apply, set_budget};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// A1's budget: 10 CPU-minutes.
pub const A1_BUDGET_MS: u64 = 10 * 60 * 1000;

/// The grant table from the experiment plan: (grant, holder, activity,
/// capability, object, parent).
pub const GRANT_TABLE: &[(&str, &str, &str, &str, &str, Option<&str>)] = &[
    ("g-read-c1", "A1", "A1", "read", "c1@v1", None),
    ("g-read-c2", "A1", "A1", "read", "c2@v1", None),
    ("g-read-c3", "A1", "A1", "read", "c3@v1", None),
    ("g-out", "A1", "A1", "append", "out/A1", None),
    ("g-send", "A1", "A1", "send", "accountant@mock.test", None),
    ("g-p1-read-c1", "processor-1", "A1.1", "read", "c1@v1", Some("g-read-c1")),
    ("g-p1-read-c2", "processor-1", "A1.1", "read", "c2@v1", Some("g-read-c2")),
    ("g-p1-append", "processor-1", "A1.1", "append", "out/A1.1", Some("g-out")),
    ("g-p2-read-c3", "processor-2", "A1.2", "read", "c3@v1", Some("g-read-c3")),
    ("g-p2-append", "processor-2", "A1.2", "append", "out/A1.2", Some("g-out")),
    ("g-d-read-a11", "drafter", "A1", "read", "out/A1.1", Some("g-out")),
    ("g-d-read-a12", "drafter", "A1", "read", "out/A1.2", Some("g-out")),
    ("g-d-write", "drafter", "A1", "write", "out/A1/draft", Some("g-out")),
    ("g-d-propose", "drafter", "A1", "submit-proposal", "approval-path", Some("g-send")),
];

fn principal() -> Source {
    Source::principal(PRINCIPAL)
}

fn summary(response: &Response) -> Value {
    serde_json::to_value(response).unwrap()
}

pub async fn setup(run: &str) -> anyhow::Result<Value> {
    let mut out = Vec::new();

    for (activity, parent, policy) in [
        ("A1", None, vec!["send".to_string()]),
        ("A1.1", Some("A1"), vec![]),
        ("A1.2", Some("A1"), vec![]),
    ] {
        let command = Command::OpenActivity {
            activity: activity.into(),
            parent: parent.map(str::to_string),
            approval_policy: policy,
        };
        out.push(summary(&apply(run, &principal(), command).await?));
    }

    let expires = now_ms() + DAY_MS;
    for (grant_id, holder, activity, capability, object, parent) in GRANT_TABLE {
        let command = Command::Grant {
            grant_id: grant_id.to_string(),
            holder: holder.to_string(),
            activity: activity.to_string(),
            capability: capability.to_string(),
            object: object.to_string(),
            parent: parent.map(str::to_string),
            // Everything delegated to a child carries a 24-hour lifespan.
            expires_at_ms: parent.map(|_| expires),
        };
        let source = if parent.is_some() { Source::service("activity-supervisor") } else { principal() };
        out.push(summary(&apply(run, &source, command).await?));
    }

    set_budget(run, "A1", A1_BUDGET_MS).await?;
    for activity in ["A1", "A1.1", "A1.2"] {
        let command = Command::Transition { activity: activity.into(), to: ActivityState::Running };
        apply(run, &principal(), command).await?;
    }

    Ok(json!(out))
}

pub async fn grant(run: &str, grant_id: &str, holder: &str, activity: &str, capability: &str, object: &str) -> anyhow::Result<Value> {
    let command = Command::Grant {
        grant_id: grant_id.into(),
        holder: holder.into(),
        activity: activity.into(),
        capability: capability.into(),
        object: object.into(),
        parent: None,
        expires_at_ms: Some(now_ms() + DAY_MS),
    };
    Ok(summary(&apply(run, &principal(), command).await?))
}

pub async fn revoke(run: &str, grant_id: &str) -> anyhow::Result<Value> {
    Ok(summary(&apply(run, &principal(), Command::Revoke { grant_id: grant_id.into() }).await?))
}

pub async fn cancel(run: &str, activity: &str) -> anyhow::Result<Value> {
    Ok(summary(&apply(run, &principal(), Command::Cancel { activity: activity.into() }).await?))
}

pub async fn close(run: &str, activity: &str) -> anyhow::Result<Value> {
    Ok(summary(&apply(run, &principal(), Command::Close { activity: activity.into() }).await?))
}

/// Replaces a participant: journals the revocation of the old holder's grants
/// and equivalent grants for the new holder, then natively stops any container
/// the old holder still has running, since a running container keeps the
/// mounts it started with.
pub async fn replace(run: &str, activity: &str, old_holder: &str, new_holder: &str) -> anyhow::Result<Value> {
    let mut out = Vec::new();

    let replaced: Vec<_> = GRANT_TABLE.iter().filter(|g| g.1 == old_holder && g.2 == activity).collect();
    for (grant_id, ..) in &replaced {
        out.push(summary(&apply(run, &principal(), Command::Revoke { grant_id: grant_id.to_string() }).await?));
    }
    for (grant_id, _, activity, capability, object, parent) in &replaced {
        let command = Command::Grant {
            grant_id: format!("{grant_id}-{new_holder}"),
            holder: new_holder.into(),
            activity: activity.to_string(),
            capability: capability.to_string(),
            object: object.to_string(),
            parent: parent.map(str::to_string),
            expires_at_ms: Some(now_ms() + DAY_MS),
        };
        out.push(summary(&apply(run, &Source::service("activity-supervisor"), command).await?));
    }

    let stopped = crate::containers::stop(run, old_holder).await?;
    Ok(json!({ "journal": out, "native": { "stopped_containers": stopped } }))
}

/// Records denials read from the participants' Wasmtime traces, with the
/// container's file system as their source.
pub async fn record_denials(run: &str, denials: &str) -> anyhow::Result<Value> {
    let denials: Vec<(String, String, String, String)> = serde_json::from_str(denials)?;
    let mut out = Vec::new();

    for (holder, operation, object, reason) in denials {
        let command = Command::Denial { holder, operation, object, reason };
        out.push(summary(&apply(run, &Source::service("container-fs"), command).await?));
    }
    Ok(json!(out))
}
