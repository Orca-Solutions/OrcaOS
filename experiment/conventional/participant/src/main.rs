// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Thin WASI wrappers around the shared task logic: the processors and the
//! drafter. They hold no authority of their own. Everything they can reach is
//! under /stores, and the container only mounts there what the holder's live
//! grants name, read-only for a read grant.
//!
//!   participant process SPACE OBJECT...     processor-1
//!   participant extract SPACE OBJECT [alt]  processor-2 (alt: processor-2b)
//!   participant finish SPACE OBJECT         processor-2, processor-2b
//!   participant draft                       the drafter
//!   participant misbehave                   test 1
//!
//! Each prints one JSON report on stdout.
//!
//! Scorecard: excluded (processor and drafter components). Tests: 1, 5, all.

use serde_json::{json, Value};

fn path(store: &str) -> String {
    format!("/stores/{store}")
}

fn get(store: &str) -> Result<String, String> {
    std::fs::read_to_string(path(store)).map_err(|e| e.to_string())
}

fn set(store: &str, value: &str) -> Result<(), String> {
    std::fs::write(path(store), value).map_err(|e| e.to_string())
}

fn hash(text: &str) -> String {
    orca_ref::binding::sha256_hex(text.as_bytes())
}

fn written(object: &str, value: &str) -> Value {
    json!({ "object": object, "hash": hash(value) })
}

fn process(space: &str, objects: &[String]) -> Result<Value, String> {
    let mut written_objects = Vec::new();

    for object in objects {
        let out = scenario_task::digest(object, &get(&format!("docs/{object}"))?);
        set(&format!("{space}/{object}"), &out)?;
        written_objects.push(written(object, &out));
    }
    Ok(json!({ "written": written_objects }))
}

/// Processor 2 works in two steps, so test 5 can replace it midway: extract
/// writes notes, finish turns the notes into the digest.
fn extract(space: &str, object: &str, alt: bool) -> Result<Value, String> {
    let digest = if alt { scenario_task::digest_alt } else { scenario_task::digest };
    let notes = digest(object, &get(&format!("docs/{object}"))?);
    let key = format!("{object}-notes");

    set(&format!("{space}/{key}"), &notes)?;
    Ok(json!({ "written": [written(&key, &notes)] }))
}

fn finish(space: &str, object: &str) -> Result<Value, String> {
    let notes = get(&format!("{space}/{object}-notes"))?;

    set(&format!("{space}/{object}"), &notes)?;
    Ok(json!({ "written": [written(object, &notes)] }))
}

/// Builds the summary from both output spaces and writes the draft. The
/// coordinator submits it as the drafter's proposal.
fn draft() -> Result<Value, String> {
    let mut digests = Vec::new();
    for store in ["out/A1.1/c1@v1", "out/A1.1/c2@v1", "out/A1.2/c3@v1"] {
        digests.push(get(store)?);
    }

    let message = scenario_task::compose(&scenario_task::summary(&digests));
    let message = serde_json::to_string(&message).unwrap();
    set("out/A1/draft", &message)?;
    Ok(json!({ "draft_hash": hash(&message) }))
}

/// Test 1: the processor itself attempts reads and a write outside its grants,
/// so a refusal shows enforcement, not restraint.
fn misbehave() -> Value {
    let attempts = [
        ("read", "tax-return-2025", get("docs/tax-return-2025").map(|_| ())),
        ("read", "doc-0005", get("docs/doc-0005").map(|_| ())),
        ("read", "c1@v2", get("docs/c1@v2").map(|_| ())),
        ("write", "c1@v1", set("docs/c1@v1", "overwritten by processor-1")),
    ];

    let results: Vec<Value> = attempts
        .iter()
        .map(|(op, object, result)| match result {
            Ok(()) => json!({ "operation": op, "object": object, "allowed": true }),
            Err(e) => json!({ "operation": op, "object": object, "allowed": false, "error": e }),
        })
        .collect();
    json!(results)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();

    let report = match arg(0).as_str() {
        "process" => process(&arg(1), &args[2..]),
        "extract" => extract(&arg(1), &arg(2), arg(3) == "alt"),
        "finish" => finish(&arg(1), &arg(2)),
        "draft" => draft(),
        "misbehave" => Ok(misbehave()),
        other => Err(format!("unknown command {other}")),
    };

    let report = report.unwrap_or_else(|e| json!({ "error": e }));
    println!("{report}");
}
