// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The mock mail contract: `POST /submit` and `GET /status/{request_id}`, and
//! the rule that a not-received reply after the validity window closes
//! certifies that the request never started.
//!
//! Scorecard: counted. Category: recovery. Tests: 2, 4, 7.
//! Developer model: specific to the mock mail contract.

use orca_ref::{payload_hash, Message, StatusObservation, SubmitResult};
use serde_json::{json, Value};

use super::delivery::Contract;

const SERVICE: &str = "http://127.0.0.1:18081";
/// The harness's control port; only the checkpoint hook below uses it.
const CONTROL: &str = "http://127.0.0.1:18082";

pub struct MockMail {
    /// The harness's run identifier; the shared mock keeps runs apart by it.
    pub run: String,
    pub credential: String,
    pub validity_ms: u64,
    pub http: reqwest::Client,
}

impl Contract for MockMail {
    fn system(&self) -> &'static str {
        "mock-mail"
    }

    fn operation(&self) -> &'static str {
        "send"
    }

    fn idempotent(&self) -> bool {
        true
    }

    fn validity_ms(&self) -> u64 {
        self.validity_ms
    }

    fn payload_hash(&self, payload: &str) -> Result<String, String> {
        let message: Message = serde_json::from_str(payload).map_err(|e| e.to_string())?;
        Ok(payload_hash(&message))
    }

    async fn submit(&self, request_id: &str, not_after_ms: u64, payload: &str) -> SubmitResult {
        let message: Value = serde_json::from_str(payload).unwrap_or_default();
        let body = json!({ "request_id": request_id, "not_after_ms": not_after_ms, "message": message });
        let request = self
            .http
            .post(format!("{SERVICE}/submit"))
            .bearer_auth(&self.credential)
            .header("x-orca-run", &self.run)
            .json(&body);

        match request.send().await {
            Err(e) => SubmitResult::NoReply { detail: e.to_string() },
            Ok(response) => {
                let status = response.status().as_u16();
                let reply: Value = response.json().await.unwrap_or_default();
                match (status, reply["result"].as_str()) {
                    (200, Some("accepted")) => SubmitResult::Accepted {
                        message_id: reply["message_id"].as_str().unwrap_or_default().into(),
                    },
                    (200, Some("refused")) => SubmitResult::Refused {
                        reason: reply["reason"].as_str().unwrap_or_default().into(),
                    },
                    // A server error says nothing about whether the message was
                    // accepted first; only a status check can settle it.
                    (code @ 500..=599, _) => SubmitResult::NoReply { detail: format!("HTTP {code}: {reply}") },
                    (code, _) => SubmitResult::Refused { reason: format!("HTTP {code}: {reply}") },
                }
            }
        }
    }

    async fn status(&self, request_id: &str, not_after_ms: u64) -> StatusObservation {
        let url = format!("{SERVICE}/status/{request_id}?not_after_ms={not_after_ms}");

        match self.http.get(url).header("x-orca-run", &self.run).send().await {
            Err(e) => StatusObservation::Unavailable { detail: e.to_string() },
            Ok(response) if response.status().as_u16() != 200 => StatusObservation::Unavailable {
                detail: format!("HTTP {}", response.status().as_u16()),
            },
            Ok(response) => {
                let reply: Value = response.json().await.unwrap_or_default();
                match reply["status"].as_str() {
                    Some("accepted") => StatusObservation::Accepted {
                        message_id: reply["message_id"].as_str().unwrap_or_default().into(),
                    },
                    Some("refused") => StatusObservation::Refused {
                        reason: reply["reason"].as_str().unwrap_or_default().into(),
                    },
                    Some("not_received") => StatusObservation::NotReceived {
                        certified: reply["certified"].as_bool().unwrap_or(false),
                    },
                    _ => StatusObservation::Unavailable { detail: format!("unreadable reply: {reply}") },
                }
            }
        }
    }

    // Scorecard: excluded (test harness hook).
    async fn checkpoint(&self, point: &str, request_id: &str) {
        let url = format!("{CONTROL}/control/checkpoint?point={point}&request_id={}/{request_id}", self.run);
        let _ = self.http.post(url).send().await;
    }
}
