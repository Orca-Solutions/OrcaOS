// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Drives the conventional composition: workflows through the Temporal CLI,
//! the principal's controls through `conventional ctl`, the approval service
//! over HTTP, the run's stores on disk, and the kill -9 and restart of every
//! platform process that the crash tests need.
//!
//! Scorecard: excluded (test harness).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub struct Conventional {
    pub bin: PathBuf,
    pub temporal: PathBuf,
    pub stack: PathBuf,
    pub stores: PathBuf,
    pub approval: String,
}

/// Synthetic inputs loaded into every run's document store, as the Golem
/// build's Seeder loads them.
pub const INPUTS: [&str; 5] = ["c1@v1", "c1@v2", "c2@v1", "c3@v1", "tax-return-2025"];

fn sha256(path: &std::path::Path) -> String {
    Command::new("sha256sum")
        .arg(path)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap_or_default().to_string())
        .unwrap_or_default()
}

/// A workflow identifier no earlier call in this run has used.
fn unique(run: &str, label: &str) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    format!("{run}/{label}/{nanos:x}")
}

impl Conventional {
    pub fn from_env() -> Self {
        let var = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.to_string());
        let base = var("ORCA_BASE", "/opt/baseline");
        Self {
            bin: PathBuf::from(var("ORCA_CONVENTIONAL_BIN", "target/debug/conventional")),
            temporal: PathBuf::from(var("ORCA_TEMPORAL_CLI", &format!("{base}/bin/temporal"))),
            stack: PathBuf::from(var("ORCA_STACK", "conventional/stack.sh")),
            stores: PathBuf::from(var("ORCA_STORES", &format!("{base}/stores"))),
            approval: var("ORCA_APPROVAL", "http://127.0.0.1:18090"),
        }
    }

    /// One principal control or platform tool; its printed result.
    pub fn ctl(&self, run: &str, op: &str, args: &[&str]) -> Result<String, String> {
        let output = Command::new(&self.bin).args(["ctl", run, op]).args(args).output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Starts a workflow, waits for it and returns its string result.
    pub fn workflow(&self, kind: &str, queue: &str, id: &str, input: &Value) -> Result<String, String> {
        let output = Command::new(&self.temporal)
            .args(["workflow", "execute", "--type", kind, "--task-queue", queue, "--workflow-id", id, "-o", "json", "--input"])
            .arg(input.to_string())
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        let doc: Value = serde_json::from_slice(&output.stdout).map_err(|_| String::from_utf8_lossy(&output.stderr).to_string())?;

        match doc["result"].as_str() {
            Some(result) if doc["status"] == "COMPLETED" => Ok(result.to_string()),
            _ => Err(doc["closeEvent"].to_string()),
        }
    }

    /// One coordinator step on the coordinator worker's queue.
    pub fn step(&self, run: &str, step: Value) -> Result<String, String> {
        let label = step["op"].as_str().unwrap_or("step").to_string();
        self.workflow("Coordinate", "scenario-a", &unique(run, &label), &step)
    }

    /// Hands a message to the adapter's queue directly, as a tampered path
    /// would, bypassing the approval service.
    pub fn deliver_directly(&self, run: &str, authorization: &str, expires_at_ms: u64, message: &Value) -> Result<String, String> {
        let delivery = json!({
            "run": run,
            "activity": "A1",
            "authorization": authorization,
            "expires_at_ms": expires_at_ms,
            "message": message.to_string(),
        });
        self.workflow("DeliveryWorkflow", "mail-adapter", &unique(run, "direct"), &delivery)
    }

    pub fn post(&self, path: &str, body: Value) -> Result<String, String> {
        ureq::post(&format!("{}{path}", self.approval))
            .send_json(body)
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())
    }

    pub fn display(&self, run: &str, proposal: &str) -> Result<Value, String> {
        ureq::get(&format!("{}/display", self.approval))
            .query("run", run)
            .query("proposal", proposal)
            .call()
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())
    }

    /// Loads the synthetic inputs into the run's document store.
    pub fn seed(&self, run: &str) -> Result<Value, String> {
        let docs = self.stores.join(run).join("docs");
        std::fs::create_dir_all(&docs).map_err(|e| e.to_string())?;

        let mut loaded = Vec::new();
        for id in INPUTS {
            std::fs::copy(format!("shared/synthetic-inputs/{id}.txt"), docs.join(id)).map_err(|e| e.to_string())?;
            loaded.push(json!({ "object": id, "hash": sha256(&docs.join(id)) }));
        }
        Ok(json!(loaded))
    }

    pub fn snapshot_hash(&self, run: &str, object: &str) -> Value {
        json!(sha256(&self.stores.join(run).join("docs").join(object)))
    }

    /// Wasmtime's trace of every WASI call the holder's containers made.
    pub fn trace(&self, run: &str, holder: &str) -> String {
        std::fs::read_to_string(self.stores.join(run).join("traces").join(format!("{holder}.log"))).unwrap_or_default()
    }

    /// What the holder's next container would mount, in a stable order.
    pub fn mounts(&self, run: &str, holder: &str) -> Result<Value, String> {
        let listed: Value = serde_json::from_str(&self.ctl(run, "mounts", &[holder])?).map_err(|e| e.to_string())?;
        let mut mounts = listed.as_array().cloned().unwrap_or_default();
        mounts.sort_by_key(|m| m["path"].to_string());
        Ok(json!(mounts))
    }

    fn stack(&self, op: &str) -> Result<(), String> {
        let status = Command::new("bash").arg(&self.stack).arg(op).stdin(Stdio::null()).status().map_err(|e| e.to_string())?;
        status.success().then_some(()).ok_or_else(|| format!("stack.sh {op} failed"))
    }

    /// Stops every platform process with SIGKILL, starts them again and
    /// waits until they answer. Returns how long the restart took.
    pub fn kill_and_restart(&self) -> Result<Duration, String> {
        let started = Instant::now();
        self.stack("kill")?;
        self.stack("start")?;
        Ok(started.elapsed())
    }

    pub fn version(&self, program: &str, args: &[&str]) -> String {
        Command::new(program)
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or_default().trim().to_string())
            .unwrap_or_default()
    }
}
