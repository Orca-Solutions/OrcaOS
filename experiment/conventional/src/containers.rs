// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs a participant: one fresh Podman container per call, with no network,
//! no credentials and a read-only root, running the participant component
//! under Wasmtime. The only store objects it can see are the ones its live
//! grants name, mounted read-only for a read grant and writable for an append
//! or write grant. The grants are read from the journal at the moment the
//! container starts, so a revoked or expired grant mounts nothing.
//!
//! A running container keeps the mounts it started with, so after every
//! revocation, cancellation or close the supervisor stops any container whose
//! holder's live grants have changed since it started.
//!
//! Wasmtime traces every WASI call it serves. The trace goes beside the run's
//! stores, out of the participants' reach, where the inspector reads the
//! refused calls.
//!
//! Scorecard: counted. Category: grants (native enforcement of the grant
//! table). Tests: 1, 5; every participant run.

use std::path::PathBuf;

use serde_json::{json, Value};
use tokio::process::Command;

use crate::common::env;
use crate::journal::{live_grants, read};

pub fn image() -> String {
    env("ORCA_PARTICIPANT_IMAGE", "localhost/orca-participant:latest")
}

/// The run's stores on the host.
pub fn stores(run: &str) -> PathBuf {
    PathBuf::from(env("ORCA_STORES", "/opt/baseline/stores")).join(run)
}

/// Snapshots live under docs/; output spaces and their objects under out/.
fn store_path(object: &str) -> String {
    if object.starts_with("out/") { object.to_string() } else { format!("docs/{object}") }
}

/// (host path, path inside the container, writable) for each live grant.
pub async fn mounts(run: &str, holder: &str) -> anyhow::Result<Vec<(PathBuf, String, bool)>> {
    let writer = read(run).await?;
    let mut mounts = Vec::new();

    for grant in live_grants(&writer, holder) {
        let writable = match grant.capability.as_str() {
            "read" => false,
            "append" | "write" => true,
            _ => continue,
        };
        let store = store_path(&grant.object);
        let host = stores(run).join(&store);

        // An output space (out/A1.1) is a directory; an object inside one
        // (out/A1/draft) is a file. Bind mounts need the source to exist.
        if store.starts_with("out/") && store.matches('/').count() == 1 {
            std::fs::create_dir_all(&host)?;
        } else if store.starts_with("out/") && !host.exists() {
            std::fs::create_dir_all(host.parent().unwrap())?;
            std::fs::write(&host, "")?;
        }
        mounts.push((host, format!("/stores/{store}"), writable));
    }
    Ok(mounts)
}

/// A digest of what a container mounts, kept as a label so a container can
/// be compared with its holder's grants later.
fn digest(mounts: &[(PathBuf, String, bool)]) -> String {
    let mut listed: Vec<String> = mounts.iter().map(|(_, inside, writable)| format!("{inside}:{writable}")).collect();
    listed.sort();
    orca_ref::binding::sha256_hex(listed.join(",").as_bytes())
}

/// The `podman` arguments for one participant container, after the verb.
async fn container(run: &str, holder: &str, args: &[String]) -> anyhow::Result<Vec<String>> {
    let mounts = mounts(run, holder).await?;
    let mut command: Vec<String> = ["--network=none", "--read-only", "--cap-drop=all", "--security-opt=no-new-privileges"].map(String::from).to_vec();
    command.extend(["--label".into(), format!("orca.run={run}"), "--label".into(), format!("orca.holder={holder}")]);
    command.extend(["--label".into(), format!("orca.mounts={}", digest(&mounts))]);
    command.extend(["--env", "WASMTIME_LOG=wasmtime_wasi=trace", "--env", "RUST_LIB_BACKTRACE=0"].map(String::from));

    for (host, inside, writable) in &mounts {
        let mode = if *writable { "rw" } else { "ro" };
        command.extend(["--volume".into(), format!("{}:{inside}:{mode}", host.display())]);
    }
    command.push(image());
    command.extend(args.iter().cloned());
    Ok(command)
}

/// Creates the holder's container without starting it and returns what
/// Podman says about it, so the harness can check the real configuration.
pub async fn probe(run: &str, holder: &str) -> anyhow::Result<Value> {
    let created = Command::new("podman").arg("create").args(container(run, holder, &[]).await?).output().await?;
    if !created.status.success() {
        anyhow::bail!("podman create: {}", String::from_utf8_lossy(&created.stderr).trim());
    }
    let id = String::from_utf8_lossy(&created.stdout).trim().to_string();
    let inspected = Command::new("podman").args(["inspect", &id]).output().await?;

    Command::new("podman").args(["rm", &id]).output().await?;
    Ok(serde_json::from_slice::<Value>(&inspected.stdout)?[0].clone())
}

pub async fn run(run: &str, holder: &str, args: &[String]) -> anyhow::Result<Value> {
    let output = Command::new("podman").args(["run", "--rm"]).args(container(run, holder, args).await?).output().await?;

    let traces = stores(run).join("traces");
    std::fs::create_dir_all(&traces)?;
    let mut trace = std::fs::OpenOptions::new().create(true).append(true).open(traces.join(format!("{holder}.log")))?;
    std::io::Write::write_all(&mut trace, &output.stderr)?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let report = stdout.lines().last().and_then(|line| serde_json::from_str(line).ok());
    Ok(report.unwrap_or_else(|| json!({ "error": format!("no report; exit {}", output.status) })))
}

/// Stops every container in the run whose holder's live grants no longer
/// give what it started with.
pub async fn stop_narrowed(run: &str) -> anyhow::Result<Vec<String>> {
    let format = r#"{{.ID}}|{{index .Labels "orca.holder"}}|{{index .Labels "orca.mounts"}}"#;
    let listed = Command::new("podman").args(["ps", "--filter", &format!("label=orca.run={run}"), "--format", format]).output().await?;
    let mut stopped = Vec::new();

    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let parts: Vec<&str> = line.split('|').collect();
        let [id, holder, started] = parts[..] else { continue };
        if digest(&mounts(run, holder).await?) != started {
            Command::new("podman").args(["kill", id]).output().await?;
            stopped.push(id.to_string());
        }
    }
    Ok(stopped)
}
