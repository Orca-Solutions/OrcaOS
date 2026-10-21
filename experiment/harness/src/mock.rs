// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The mock mail service's control port: logs, fault switches and the crash
//! and pause points.
//!
//! Scorecard: excluded (test harness).

use serde_json::{json, Value};

pub struct Mock {
    pub control: String,
}

impl Mock {
    pub fn from_env() -> Self {
        Self {
            control: std::env::var("ORCA_MOCK_CONTROL").unwrap_or_else(|_| "http://127.0.0.1:18082".into()),
        }
    }

    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        ureq::post(&format!("{}{path}", self.control))
            .send_json(body)
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        ureq::get(&format!("{}{path}", self.control))
            .call()
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())
    }

    pub fn logs(&self) -> Value {
        self.get("/control/logs").unwrap_or_default()
    }

    /// One log's entries for one run, oldest first.
    pub fn entries(&self, log: &str, run: &str) -> Vec<Value> {
        let prefix = format!("{run}/");
        self.logs()[log]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e["request_id"].as_str().is_some_and(|r| r.starts_with(&prefix)))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn fault(&self, target: &str, action: &str, run: &str, remaining: Option<u32>, delay_ms: u64) -> Result<(), String> {
        let body = json!({
            "target": target,
            "action": action,
            "rid_prefix": format!("{run}/"),
            "remaining": remaining,
            "delay_ms": delay_ms,
        });
        self.post("/control/fault", body).map(|_| ())
    }

    pub fn clear(&self) -> Result<(), String> {
        self.post("/control/clear-faults", json!({})).map(|_| ())
    }

    pub fn hold_at(&self, point: &str, run: &str) -> Result<(), String> {
        let body = json!({ "point": point, "rid_prefix": format!("{run}/"), "remaining": 1 });
        self.post("/control/checkpoint-rule", body).map(|_| ())
    }

    /// The holds still waiting for this run.
    pub fn held(&self, run: &str) -> Vec<Value> {
        let prefix = format!("{run}/");
        self.get("/control/held")
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|h| h["request_id"].as_str().is_some_and(|r| r.starts_with(&prefix)))
            .collect()
    }

    pub fn release(&self, id: u64) -> Result<(), String> {
        ureq::post(&format!("{}/control/release?id={id}", self.control))
            .call()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
