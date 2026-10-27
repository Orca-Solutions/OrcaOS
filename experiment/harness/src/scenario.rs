// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One run of Scenario A, step by step, as the facilitator would drive it,
//! on either build. Every run has its own identifier, so its agents or
//! workflows, journal, stores and mock records are its own.
//!
//! Scorecard: excluded (test harness and facilitator controls).

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::golem::{lit, Golem};
use crate::inspector;
use crate::mock::Mock;
use crate::platform::Platform;
use crate::wasmtime_trace;

pub struct Run<'a> {
    pub id: String,
    pub platform: &'a Platform,
    pub mock: &'a Mock,
}

pub struct Approved {
    pub authorization: String,
    pub expires_at_ms: u64,
    pub message: Value,
}

fn parse(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("{e}: {text}"))
}

impl<'a> Run<'a> {
    pub fn new(id: String, platform: &'a Platform, mock: &'a Mock) -> Self {
        Self { id, platform, mock }
    }

    /// The Golem server, for the steps that only exist there.
    pub fn golem(&self) -> Result<&Golem, String> {
        match self.platform {
            Platform::Golem(g) => Ok(g),
            _ => Err("this control needs the Golem build".into()),
        }
    }

    pub fn agent(&self, kind: &str) -> String {
        format!("{kind}({})", lit(&self.id))
    }

    pub fn agent2(&self, kind: &str, second: &str) -> String {
        format!("{kind}({},{})", lit(&self.id), lit(second))
    }

    pub fn call(&self, agent: &str, method: &str, args: &[&str]) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        self.golem()?.invoke(agent, method, &args)
    }

    /// A principal control: grant, revoke, cancel, close, record_denials.
    pub fn supervisor(&self, method: &str, args: &[&str]) -> Result<String, String> {
        match self.platform {
            Platform::Golem(g) => {
                let args: Vec<String> = args.iter().map(|a| lit(a)).collect();
                g.invoke(&self.agent("Supervisor"), method, &args)
            }
            Platform::Conventional(c) => c.ctl(&self.id, &method.replace('_', "-"), args),
        }
    }

    pub fn journal(&self) -> Value {
        let export = match self.platform {
            Platform::Golem(_) => self.call(&self.agent("Journal"), "export", &[]),
            Platform::Conventional(c) => c.ctl(&self.id, "export", &[]),
        };
        export.ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Seeds the document store and records the grant table.
    pub fn setup(&self) -> Result<Value, String> {
        match self.platform {
            Platform::Golem(_) => {
                let loaded = self.call(&self.agent("Seeder"), "load", &[])?;
                self.supervisor("setup", &[])?;
                serde_json::from_str(&loaded).map_err(|e| e.to_string())
            }
            Platform::Conventional(c) => {
                let loaded = c.seed(&self.id)?;
                self.supervisor("setup", &[])?;
                Ok(loaded)
            }
        }
    }

    /// A1.1's coordinator runs processor-1.
    pub fn run_processor_1(&self) -> Result<String, String> {
        match self.platform {
            Platform::Golem(_) => self.call(&self.agent2("Activity", "A1.1"), "run_processor_1", &[]),
            Platform::Conventional(c) => c.step(&self.id, json!({ "op": "run_processor_1", "run": self.id })),
        }
    }

    /// A1.2's coordinator runs one step by the named holder.
    pub fn step_processor_2(&self, holder: &str, step: &str) -> Result<String, String> {
        match self.platform {
            Platform::Golem(_) => self.call(&self.agent2("Activity", "A1.2"), "step_processor_2", &[&lit(holder), &lit(step)]),
            Platform::Conventional(c) => {
                c.step(&self.id, json!({ "op": "step_processor_2", "run": self.id, "holder": holder, "step": step }))
            }
        }
    }

    /// A1's coordinator runs the drafter, which submits its proposal.
    pub fn draft(&self) -> Result<Value, String> {
        let out = match self.platform {
            Platform::Golem(_) => self.call(&self.agent2("Activity", "A1"), "draft", &[])?,
            Platform::Conventional(c) => c.step(&self.id, json!({ "op": "draft", "run": self.id }))?,
        };
        parse(&out)
    }

    /// A child reserves from the shared budget.
    pub fn reserve(&self, child: &str, reservation: &str, cpu_ms: u64) -> Result<String, String> {
        match self.platform {
            Platform::Golem(_) => self.call(&self.agent2("Activity", child), "reserve", &[&lit(reservation), &cpu_ms.to_string()]),
            Platform::Conventional(c) => c.step(
                &self.id,
                json!({ "op": "reserve", "run": self.id, "activity": child, "reservation": reservation, "cpu_ms": cpu_ms }),
            ),
        }
    }

    /// Runs both children and the drafter. Returns the proposal identifier.
    pub fn process_and_draft(&self) -> Result<String, String> {
        self.run_processor_1()?;
        for step in ["extract", "finish"] {
            self.step_processor_2("processor-2", step)?;
        }
        let draft = self.draft()?;
        draft["proposal"].as_str().map(str::to_string).ok_or_else(|| format!("no proposal: {draft}"))
    }

    /// Submits a proposal for A1 on the drafter's behalf.
    pub fn submit(&self, message: &Value) -> Result<String, String> {
        match self.platform {
            Platform::Golem(_) => self.call(&self.agent("ApprovalPath"), "submit", &[&lit("A1"), &lit("drafter"), &lit(&message.to_string())]),
            Platform::Conventional(c) => {
                let body = json!({ "run": self.id, "activity": "A1", "holder": "drafter", "message": message.to_string() });
                c.post("/submit", body)
            }
        }
    }

    pub fn display(&self, proposal: &str) -> Result<Value, String> {
        match self.platform {
            Platform::Golem(_) => parse(&self.call(&self.agent("ApprovalPath"), "display", &[&lit(proposal)])?),
            Platform::Conventional(c) => c.display(&self.id, proposal),
        }
    }

    /// The principal approves what the approval path displayed.
    pub fn approve(&self, proposal: &str, grant: &str) -> Result<Approved, String> {
        let shown = self.display(proposal)?;
        let hash = shown["payload_hash"].as_str().ok_or("no payload hash")?.to_string();
        let out = match self.platform {
            Platform::Golem(_) => self.call(&self.agent("ApprovalPath"), "approve", &[&lit(proposal), &lit(&hash), &lit(grant)])?,
            Platform::Conventional(c) => {
                let body = json!({ "run": self.id, "proposal": proposal, "payload_hash": hash, "grant": grant });
                c.post("/approve", body)?
            }
        };
        let out: Value = serde_json::from_str(&out).map_err(|_| out.clone())?;

        Ok(Approved {
            authorization: out["authorization"].as_str().ok_or("no authorization")?.to_string(),
            expires_at_ms: out["expires_at_ms"].as_u64().unwrap_or_default(),
            message: shown["message"].clone(),
        })
    }

    /// Hands a message to the adapter directly, as a tampered path would.
    pub fn deliver_directly(&self, authorization: &str, expires_at_ms: u64, message: &Value) -> Result<String, String> {
        match self.platform {
            Platform::Golem(_) => self.call(
                &self.agent("MailAdapter"),
                "deliver",
                &[&lit("A1"), &lit(authorization), &expires_at_ms.to_string(), &lit(&message.to_string())],
            ),
            Platform::Conventional(c) => c.deliver_directly(&self.id, authorization, expires_at_ms, message),
        }
    }

    /// processor-1 tries three unrelated reads and one write, called
    /// directly, past its coordinator. Returns what it reports.
    pub fn misbehave(&self) -> Result<Value, String> {
        let out = match self.platform {
            Platform::Golem(_) => self.call(&self.agent2("Processor1", "processor-1"), "misbehave", &[])?,
            Platform::Conventional(c) => c.ctl(&self.id, "participant", &["processor-1", "misbehave"])?,
        };
        parse(&out)
    }

    /// The store's own record of the calls it refused processor-1:
    /// (operation, object, reason) each.
    pub fn store_denials(&self) -> Result<Vec<(String, String, String)>, String> {
        Ok(match self.platform {
            Platform::Golem(g) => inspector::oplog_denials(&g.oplog(&self.agent2("Processor1", "processor-1"))?),
            Platform::Conventional(c) => wasmtime_trace::denials(&c.trace(&self.id, "processor-1")),
        })
    }

    /// A snapshot object's hash as the store holds it now.
    pub fn snapshot_hash(&self, object: &str) -> Result<Value, String> {
        match self.platform {
            Platform::Golem(_) => {
                let now: Value = serde_json::from_str(&self.call(&self.agent("Seeder"), "read", &[&lit("docs"), &lit(object)])?).unwrap_or_default();
                Ok(now["hash"].clone())
            }
            Platform::Conventional(c) => Ok(c.snapshot_hash(&self.id, object)),
        }
    }

    /// Replaces a participant in an activity. Returns the platform's answer.
    pub fn replace(&self, activity: &str, old: &str, new: &str) -> Result<Value, String> {
        let out = match self.platform {
            Platform::Golem(_) => self.supervisor("replace", &[activity, old, &self.agent2("Processor2", old), new])?,
            Platform::Conventional(c) => c.ctl(&self.id, "replace", &[activity, old, new])?,
        };
        Ok(serde_json::from_str(&out).unwrap_or(json!(out)))
    }

    /// Calls processor-2's finish step directly, past its coordinator.
    pub fn finish_directly(&self, holder: &str) -> Result<String, String> {
        match self.platform {
            Platform::Golem(_) => self.call(&self.agent2("Processor2", holder), "finish", &[&lit("c3@v1")]),
            Platform::Conventional(c) => c.ctl(&self.id, "participant", &[holder, "finish", "out/A1.2", "c3@v1"]),
        }
    }

    /// Whether the store refused a direct call, by the platform's marker.
    pub fn refused_by_store(&self, out: &str) -> bool {
        match self.platform {
            Platform::Golem(_) => out.contains("permission denied"),
            // The holder's container mounts no store object, so the read
            // finds nothing there.
            Platform::Conventional(_) => out.contains("No such file or directory") && !out.contains("\"written\""),
        }
    }

    /// Waits until the adapter reaches a hold at `point`.
    pub fn wait_held(&self, point: &str, timeout: Duration) -> Option<u64> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if let Some(h) = self.mock.held(&self.id).iter().find(|h| h["point"] == point) {
                return h["id"].as_u64();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }

    /// Releases this run's holds at one checkpoint only.
    pub fn release_at(&self, point: &str) {
        for h in self.mock.held(&self.id).iter().filter(|h| h["point"] == point) {
            if let Some(id) = h["id"].as_u64() {
                let _ = self.mock.release(id);
            }
        }
    }

    /// Releases every hold this run still has, including ones whose
    /// connection died with the platform.
    pub fn release_all(&self) {
        for h in self.mock.held(&self.id) {
            if let Some(id) = h["id"].as_u64() {
                let _ = self.mock.release(id);
            }
        }
    }

    /// Polls the journal until `done` holds or the timeout passes.
    pub fn wait_journal(&self, timeout: Duration, done: impl Fn(&Value) -> bool) -> (Value, bool) {
        let started = Instant::now();
        loop {
            let journal = self.journal();
            if done(&journal) {
                return (journal, true);
            }
            if started.elapsed() > timeout {
                return (journal, false);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

/// Journal entries of one kind.
pub fn entries<'v>(journal: &'v Value, kind: &str) -> Vec<&'v Value> {
    journal["journal"]["entries"]
        .as_array()
        .map(|e| e.iter().filter(|e| e["entry"] == kind).collect())
        .unwrap_or_default()
}

pub fn receipt<'v>(journal: &'v Value, request_id: &str) -> &'v Value {
    &journal["journal"]["receipts"][request_id]
}

pub fn outcome_state(journal: &Value, request_id: &str) -> String {
    receipt(journal, request_id)["outcome"]["state"].as_str().unwrap_or("none").to_string()
}

pub fn activity_state(journal: &Value, activity: &str) -> String {
    journal["journal"]["activities"][activity]["state"].as_str().unwrap_or("none").to_string()
}
