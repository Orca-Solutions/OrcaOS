// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Thin Golem wrappers around the shared task logic: the processors and the
//! drafter. They hold no authority beyond their agent type's initialCard and
//! read and write only through Golem's key-value stores, where Golem checks
//! every call.
//!
//! Scorecard: excluded (processor and drafter components). Tests: 1, 5, all.

use golem_rust::{agent_definition, agent_implementation};
use serde_json::json;

use crate::approval::ApprovalPathClient;
use crate::common::store;

fn hash(text: &str) -> String {
    orca_ref::binding::sha256_hex(text.as_bytes())
}

/// Reads each snapshot, writes its digest to the output space and reports
/// what it wrote.
fn process(space: &str, objects: &[String], digest: fn(&str, &str) -> String) -> String {
    let mut written = Vec::new();

    for object in objects {
        let text = match store::get("docs", object) {
            Ok(Some(text)) => text,
            Ok(None) => return json!({ "error": format!("{object} not found") }).to_string(),
            Err(e) => return json!({ "error": e }).to_string(),
        };
        let out = digest(object, &text);
        if let Err(e) = store::set(space, object, &out) {
            return json!({ "error": e }).to_string();
        }
        written.push(json!({ "object": object, "hash": hash(&out) }));
    }

    json!({ "written": written }).to_string()
}

#[agent_definition]
pub trait Processor1 {
    fn new(run: String, holder: String) -> Self;
    fn process(&self, objects: Vec<String>) -> String;
    /// Test 1: the processor itself attempts reads and a write outside its
    /// grants, so a refusal shows enforcement, not restraint.
    fn misbehave(&self) -> String;
}

struct Processor1Impl;

#[agent_implementation]
impl Processor1 for Processor1Impl {
    fn new(_run: String, _holder: String) -> Self {
        Self
    }

    fn process(&self, objects: Vec<String>) -> String {
        process("out-A1-1", &objects, scenario_task::digest)
    }

    fn misbehave(&self) -> String {
        let attempts = [
            ("read", "tax-return-2025", store::get("docs", "tax-return-2025").map(|v| v.is_some())),
            ("read", "doc-0005", store::get("docs", "doc-0005").map(|v| v.is_some())),
            ("read", "c1@v2", store::get("docs", "c1@v2").map(|v| v.is_some())),
            ("write", "c1@v1", store::set("docs", "c1@v1", "overwritten by processor-1").map(|_| true)),
        ];

        let results: Vec<_> = attempts
            .iter()
            .map(|(op, object, result)| match result {
                Ok(found) => json!({ "operation": op, "object": object, "allowed": true, "found": found }),
                Err(e) => json!({ "operation": op, "object": object, "allowed": false, "error": e }),
            })
            .collect();
        json!(results).to_string()
    }
}

/// Processor 2 works in two steps, so test 5 can replace it midway: extract
/// writes notes, finish turns the notes into the digest.
fn extract(object: &str, digest: fn(&str, &str) -> String) -> String {
    match store::get("docs", object) {
        Ok(Some(text)) => {
            let notes = digest(object, &text);
            match store::set("out-A1-2", &format!("{object}-notes"), &notes) {
                Ok(()) => json!({ "written": [{ "object": format!("{object}-notes"), "hash": hash(&notes) }] }).to_string(),
                Err(e) => json!({ "error": e }).to_string(),
            }
        }
        Ok(None) => json!({ "error": format!("{object} not found") }).to_string(),
        Err(e) => json!({ "error": e }).to_string(),
    }
}

fn finish(object: &str) -> String {
    match store::get("out-A1-2", &format!("{object}-notes")) {
        Ok(Some(notes)) => match store::set("out-A1-2", object, &notes) {
            Ok(()) => json!({ "written": [{ "object": object, "hash": hash(&notes) }] }).to_string(),
            Err(e) => json!({ "error": e }).to_string(),
        },
        Ok(None) => json!({ "error": "no notes to finish from" }).to_string(),
        Err(e) => json!({ "error": e }).to_string(),
    }
}

#[agent_definition]
pub trait Processor2 {
    fn new(run: String, holder: String) -> Self;
    fn extract(&self, object: String) -> String;
    fn finish(&self, object: String) -> String;
}

struct Processor2Impl;

#[agent_implementation]
impl Processor2 for Processor2Impl {
    fn new(_run: String, _holder: String) -> Self {
        Self
    }

    fn extract(&self, object: String) -> String {
        extract(&object, scenario_task::digest)
    }

    fn finish(&self, object: String) -> String {
        finish(&object)
    }
}

#[agent_definition]
pub trait Processor2Alt {
    fn new(run: String, holder: String) -> Self;
    fn extract(&self, object: String) -> String;
    fn finish(&self, object: String) -> String;
}

struct Processor2AltImpl;

#[agent_implementation]
impl Processor2Alt for Processor2AltImpl {
    fn new(_run: String, _holder: String) -> Self {
        Self
    }

    fn extract(&self, object: String) -> String {
        extract(&object, scenario_task::digest_alt)
    }

    fn finish(&self, object: String) -> String {
        finish(&object)
    }
}

#[agent_definition]
pub trait Drafter {
    fn new(run: String, holder: String) -> Self;
    /// Builds the summary from both output spaces, writes the draft and
    /// submits it as a proposal. Returns only the proposal's reference.
    async fn draft(&mut self, activity: String) -> String;
}

struct DrafterImpl {
    run: String,
    holder: String,
}

#[agent_implementation]
impl Drafter for DrafterImpl {
    fn new(run: String, holder: String) -> Self {
        Self { run, holder }
    }

    async fn draft(&mut self, activity: String) -> String {
        let mut digests = Vec::new();

        for (space, object) in [("out-A1-1", "c1@v1"), ("out-A1-1", "c2@v1"), ("out-A1-2", "c3@v1")] {
            match store::get(space, object) {
                Ok(Some(digest)) => digests.push(digest),
                Ok(None) => return json!({ "error": format!("{space}/{object} missing") }).to_string(),
                Err(e) => return json!({ "error": e }).to_string(),
            }
        }

        let message = scenario_task::compose(&scenario_task::summary(&digests));
        let message = serde_json::to_string(&message).unwrap();
        if let Err(e) = store::set("out-A1", "draft", &message) {
            return json!({ "error": e }).to_string();
        }

        let proposal = ApprovalPathClient::get(self.run.clone())
            .submit(activity, self.holder.clone(), message.clone())
            .await;
        json!({ "proposal": proposal, "draft_hash": hash(&message) }).to_string()
    }
}
