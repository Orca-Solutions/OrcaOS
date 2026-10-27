// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The reproducibility record stored next to every result, and the check of
//! each participant's native authority (Golem's card lists, the conventional
//! build's mounts) that runs before and after a suite.
//!
//! Scorecard: excluded (test harness).

use std::process::Command;

use serde_json::{json, Value};

use crate::conventional::Conventional;
use crate::golem::{lit, Golem};
use crate::platform::Platform;

fn sh(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn sha256(path: &str) -> String {
    sh("sha256sum", &[path]).split_whitespace().next().unwrap_or("missing").to_string()
}

/// A package's version in a Cargo.lock.
fn locked(lock_file: &str, package: &str) -> String {
    let lock = std::fs::read_to_string(lock_file).unwrap_or_default();
    lock.split("[[package]]")
        .find(|p| p.contains(&format!("name = \"{package}\"")))
        .and_then(|p| p.lines().find(|l| l.starts_with("version")))
        .unwrap_or("unknown")
        .trim()
        .to_string()
}

pub fn reproducibility(platform: &Platform, validity_ms: u64, seed: u64) -> Value {
    let mut record = match platform {
        Platform::Golem(golem) => golem_record(golem, validity_ms),
        Platform::Conventional(c) => conventional_record(c, validity_ms),
    };
    let inputs: Vec<Value> = ["c1@v1", "c1@v2", "c2@v1", "c3@v1", "tax-return-2025"]
        .iter()
        .map(|id| json!({ "object": id, "sha256": sha256(&format!("shared/synthetic-inputs/{id}.txt")) }))
        .collect();

    let shared = json!({
        "mock_service": {
            "binary_sha256": sha256("target/debug/mock-mail"),
        },
        "synthetic_inputs": inputs,
        "base_seed": seed,
        "repository_commit": sh("git", &["rev-parse", "HEAD"]),
        "repository_dirty": !sh("git", &["status", "--porcelain", "--", "."]).is_empty(),
        "host": {
            "kernel": sh("uname", &["-srm"]),
            "os": sh("sh", &["-c", ". /etc/os-release && echo $PRETTY_NAME"]),
            "cpus": sh("nproc", &[]),
            "cpu_model": sh("sh", &["-c", "grep -m1 'model name' /proc/cpuinfo | cut -d: -f2"]),
            "memory": sh("sh", &["-c", "free -m | awk '/Mem:/ {print $2 \" MiB\"}'"]),
        },
    });
    for (key, value) in shared.as_object().unwrap() {
        record[key] = value.clone();
    }
    record
}

fn golem_record(golem: &Golem, validity_ms: u64) -> Value {
    let app = golem.app_dir.display().to_string();
    json!({
        "protocol": "OrcaOS experiment plan, protocol as of 2026-10-20",
        "platform": {
            "golem_cli_and_server": golem.version(),
            "golem_binary_sha256": sha256(&golem.cli.display().to_string()),
            "container_image": sh("docker", &["image", "inspect", "--format", "{{.RepoTags}} {{.Id}} {{.RepoDigests}}", "mirror.gcr.io/golemservices/golem:v1.6.0"]),
            "golem_rust": locked(&golem.app_dir.join("Cargo.lock").display().to_string(), "golem-rust"),
        },
        "configuration": {
            "golem_manifest_sha256": sha256(&format!("{app}/golem.yaml")),
            "component_wasm_sha256": sha256(&format!("{app}/golem-temp/agents/scenario_a_main.wasm")),
            "component_wasm_debug_sha256": sha256(&format!("{app}/target/wasm32-wasip2/debug/scenario_a_main.wasm")),
            "validity_window_ms": validity_ms,
        },
    })
}

fn conventional_record(c: &Conventional, validity_ms: u64) -> Value {
    let pg = std::env::var("PGBIN").unwrap_or_else(|_| "/usr/lib/postgresql/16/bin".into());
    let image = std::env::var("ORCA_PARTICIPANT_IMAGE").unwrap_or_else(|_| "localhost/orca-participant:latest".into());
    let wasmtime = std::env::var("ORCA_WASMTIME").unwrap_or_else(|_| "/opt/baseline/bin/wasmtime".into());
    json!({
        "protocol": "OrcaOS experiment plan, protocol as of 2026-10-20",
        "platform": {
            "temporal_cli_and_dev_server": c.version(&c.temporal.display().to_string(), &["--version"]),
            "temporal_cli_source": std::env::var("ORCA_TEMPORAL_SOURCE").unwrap_or_else(|_| "github.com/temporalio/cli v1.9.1, built from source".into()),
            "temporal_binary_sha256": sha256(&c.temporal.display().to_string()),
            "temporalio_sdk": locked("Cargo.lock", "temporalio-sdk"),
            "postgresql": c.version(&format!("{pg}/postgres"), &["--version"]),
            "podman": c.version("podman", &["--version"]),
            "wasmtime": c.version(&wasmtime, &["--version"]),
            "participant_image": sh("podman", &["image", "inspect", "--format", "{{.Id}}", &image]),
            "tokio_postgres": locked("Cargo.lock", "tokio-postgres"),
        },
        "configuration": {
            "schema_sha256": sha256("conventional/schema.sql"),
            "containerfile_sha256": sha256("conventional/Containerfile"),
            "stack_script_sha256": sha256("conventional/stack.sh"),
            "services_binary_sha256": sha256(&c.bin.display().to_string()),
            "participant_wasm_sha256": sha256("target/wasm32-wasip2/release/participant.wasm"),
            "validity_window_ms": validity_ms,
        },
    })
}

/// Each participant's native authority against its declaration, before and
/// after a suite.
pub fn check_native(platform: &Platform, run: &str) -> (bool, Value) {
    match platform {
        Platform::Golem(golem) => check_cards(golem, run),
        Platform::Conventional(c) => check_mounts(c, run),
    }
}

/// Compares what each participant's container would mount with the grant
/// table the journal recorded at setup. A mount the table does not give
/// stops the run.
fn check_mounts(c: &Conventional, run: &str) -> (bool, Value) {
    let export: Value = c.ctl(run, "export", &[]).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut ok = true;
    let mut rows = Vec::new();

    for holder in ["processor-1", "processor-2", "processor-2b", "drafter"] {
        let mut declared: Vec<Value> = export["journal"]["grants"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, g)| g["holder"] == holder)
            .filter_map(|(_, g)| {
                let object = g["object"].as_str().unwrap_or_default();
                let path = if object.starts_with("out/") { format!("/stores/{object}") } else { format!("/stores/docs/{object}") };
                match g["capability"].as_str() {
                    Some("read") => Some(json!({ "path": path, "writable": false })),
                    Some("append" | "write") => Some(json!({ "path": path, "writable": true })),
                    _ => None,
                }
            })
            .collect();
        let mut mounted = c.mounts(run, holder).ok().and_then(|m| m.as_array().cloned()).unwrap_or_default();
        declared.sort_by_key(|m| m.to_string());
        mounted.sort_by_key(|m| m.to_string());

        let row_ok = mounted == declared;
        ok &= row_ok;
        rows.push(json!({ "holder": holder, "declared": declared, "mounted": mounted, "ok": row_ok }));
    }
    (ok, json!(rows))
}

/// The initialCard grants declared per agent type in golem.yaml.
fn declared(golem: &Golem) -> Vec<(String, Vec<String>)> {
    let manifest = std::fs::read_to_string(golem.app_dir.join("golem.yaml")).unwrap_or_default();
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let (mut in_agents, mut in_lower) = (false, false);

    for line in manifest.lines() {
        if line == "agents:" {
            in_agents = true;
            continue;
        }
        if !in_agents {
            continue;
        }
        if line.len() > 2 && line.starts_with("  ") && !line.starts_with("   ") && line.trim_end().ends_with(':') {
            out.push((line.trim().trim_end_matches(':').to_string(), Vec::new()));
        } else if line.trim() == "lowerBound:" {
            in_lower = true;
        } else if line.trim() == "upperBound:" {
            in_lower = false;
        } else if let Some(grant) = line.trim().strip_prefix("- \"").and_then(|g| g.strip_suffix('"')) {
            if in_lower {
                if let Some((_, grants)) = out.last_mut() {
                    grants.push(grant.to_string());
                }
            }
        }
    }
    out
}

/// A grant without its recipient, so the manifest's `?agent` and the card
/// list's resolved agent path compare equal.
fn without_recipient(grant: &str) -> String {
    let (left, right) = grant.split_once(" : ").unwrap_or((grant, ""));
    format!("{} : {right}", left.split(" @ ").next().unwrap_or(left))
}

/// Compares every agent type's card list with its declared initialCard. A
/// card that grants anything the declaration does not stops the run.
fn check_cards(golem: &Golem, run: &str) -> (bool, Value) {
    let mut ok = true;
    let mut rows = Vec::new();

    for (agent_type, grants) in declared(golem) {
        let agent = match agent_type.as_str() {
            "Activity" => format!("Activity({},{})", lit(run), lit("A1")),
            "Processor1" => format!("Processor1({},{})", lit(run), lit("processor-1")),
            "Processor2" => format!("Processor2({},{})", lit(run), lit("processor-2")),
            "Processor2Alt" => format!("Processor2Alt({},{})", lit(run), lit("processor-2b")),
            "Drafter" => format!("Drafter({},{})", lit(run), lit("drafter")),
            other => format!("{other}({})", lit(run)),
        };
        let mut expected: Vec<String> = grants.iter().map(|g| without_recipient(g)).collect();
        expected.sort();

        let cards = golem.cards(&agent).unwrap_or_default();
        let mut extra = Vec::new();
        let mut found = false;
        for card in cards["cards"].as_array().into_iter().flatten() {
            let mut held: Vec<String> = card["lowerPositive"].as_array().into_iter().flatten().filter_map(Value::as_str).map(without_recipient).collect();
            held.sort();
            if card["type"] == "Polymorphic" && held == expected {
                found = true;
            } else if !held.is_empty() {
                extra.push(card.clone());
            }
        }

        let row_ok = found && extra.is_empty();
        ok &= row_ok;
        rows.push(json!({ "agent": agent, "declared": expected, "matches": found, "undeclared_cards": extra, "ok": row_ok }));
    }
    (ok, json!(rows))
}
