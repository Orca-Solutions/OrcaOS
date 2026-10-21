// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Harness infrastructure, not a participant: loads the synthetic inputs into
//! the document store and reads stores back so the harness can hash them.
//!
//! Scorecard: excluded (test harness).

use golem_rust::{agent_definition, agent_implementation};
use serde_json::json;

use crate::common::store;

#[agent_definition]
pub trait Seeder {
    fn new(run: String) -> Self;
    fn load(&self) -> String;
    fn read(&self, bucket: String, key: String) -> String;
}

struct SeederImpl;

#[agent_implementation]
impl Seeder for SeederImpl {
    fn new(_run: String) -> Self {
        Self
    }

    fn load(&self) -> String {
        let mut loaded = Vec::new();
        for (id, text) in scenario_task::inputs::ALL {
            match store::set("docs", id, text) {
                Ok(()) => loaded.push(json!({ "object": id, "hash": orca_ref::binding::sha256_hex(text.as_bytes()) })),
                Err(e) => return json!({ "error": e }).to_string(),
            }
        }
        json!(loaded).to_string()
    }

    fn read(&self, bucket: String, key: String) -> String {
        match store::get(&bucket, &key) {
            Ok(Some(value)) => json!({ "value": value, "hash": orca_ref::binding::sha256_hex(value.as_bytes()) }).to_string(),
            Ok(None) => json!({ "value": null }).to_string(),
            Err(e) => json!({ "error": e }).to_string(),
        }
    }
}
