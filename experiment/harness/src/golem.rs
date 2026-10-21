// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Drives the local Golem server through its CLI: invocations, oplogs, card
//! lists, and the kill -9 and restart the crash tests need.
//!
//! Scorecard: excluded (test harness).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

pub struct Golem {
    /// The Golem application directory (experiment/golem).
    pub app_dir: PathBuf,
    pub cli: PathBuf,
    pub data_dir: PathBuf,
    pub run_dir: PathBuf,
}

/// A Rust string literal, the form the golem CLI parses string arguments in.
pub fn lit(s: &str) -> String {
    format!("{s:?}")
}

impl Golem {
    pub fn from_env() -> Self {
        let var = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.to_string());
        Self {
            app_dir: PathBuf::from(var("ORCA_GOLEM_APP", "golem")),
            cli: PathBuf::from(var("ORCA_GOLEM_CLI", "/opt/golem/golem")),
            data_dir: PathBuf::from(var("ORCA_GOLEM_DATA", "/opt/golem/data")),
            run_dir: PathBuf::from(var("ORCA_RUN_DIR", "/opt/golem/run")),
        }
    }

    fn cli(&self) -> Command {
        let mut command = Command::new(&self.cli);
        command.current_dir(&self.app_dir).arg("-Y");
        command
    }

    /// Invokes a method and returns its string result.
    pub fn invoke(&self, agent: &str, method: &str, args: &[String]) -> Result<String, String> {
        let output = self
            .cli()
            .args(["--format", "json", "agent", "invoke", agent, method])
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;

        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(doc) = serde_json::from_str::<Value>(line) else { continue };
            if doc["$type"] == "agent.invoke" {
                let result = doc["result"].as_str().unwrap_or_default();
                // String results come back as a Rust string literal.
                return Ok(serde_json::from_str::<String>(result).unwrap_or_else(|_| result.to_string()));
            }
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr.chars().rev().take(1500).collect::<String>().chars().rev().collect())
    }

    pub fn oplog(&self, agent: &str) -> Result<String, String> {
        let output = self.cli().args(["agent", "oplog", agent]).output().map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn cards(&self, agent: &str) -> Result<Value, String> {
        let output = self
            .cli()
            .args(["--format", "json", "card", "list", "--agent", agent])
            .output()
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&output.stdout);
        let last = text.lines().filter(|l| l.starts_with('{')).last().unwrap_or("null");
        serde_json::from_str(last).map_err(|e| format!("{e}: {text}"))
    }

    pub fn version(&self) -> String {
        Command::new(&self.cli)
            .arg("--version")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }

    fn healthy(&self) -> bool {
        ureq::get("http://127.0.0.1:9881/healthcheck")
            .timeout(Duration::from_millis(500))
            .call()
            .is_ok()
    }

    /// Stops the server with SIGKILL, starts it again and waits until it
    /// answers. Returns how long the restart took.
    pub fn kill_and_restart(&self) -> Result<Duration, String> {
        let started = Instant::now();
        let pid_file = self.run_dir.join("golem.pid");

        if let Ok(pid) = std::fs::read_to_string(&pid_file) {
            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
        }
        let _ = Command::new("pkill").args(["-9", "-x", "golem"]).status();
        std::thread::sleep(Duration::from_millis(300));

        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.run_dir.join("golem.log"))
            .map_err(|e| e.to_string())?;
        let child = Command::new(&self.cli)
            .args(["server", "run", "--data-dir"])
            .arg(&self.data_dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .map_err(|e| e.to_string())?;
        std::fs::write(&pid_file, child.id().to_string()).map_err(|e| e.to_string())?;

        while started.elapsed() < Duration::from_secs(90) {
            if self.healthy() {
                return Ok(started.elapsed());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err("the Golem server did not come back within 90 s".into())
    }
}
