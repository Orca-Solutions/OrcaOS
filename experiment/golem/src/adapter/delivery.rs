// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The delivery protocol for one consequential operation, independent of the
//! external system: early check, consume-if-live, dispatch only after the
//! consumption commits, record the submission and the receipt, and on
//! recovery check status before anything else.
//!
//! Golem would resend an HTTP request that was in flight when the host
//! failed. To keep that from becoming a forbidden re-dispatch, everything from
//! consumption on runs inside one custom durable operation: if the host fails
//! inside it, Golem re-runs the body live instead of replaying recorded
//! effects, and the journal tells the body it has already consumed (the same
//! delivery token), which puts it on the recovery path.
//!
//! Scorecard: counted. Category: recovery (approvals for the binding check).
//! Tests: 2, 3, 4, 7. Developer model: reusable for any adapter contract.

use golem_rust::durability::{Durability, DurableFunctionType};
use orca_ref::{
    authorization_binding, decide, request_id, Command, RecoveryAction, Response, Source, StatusObservation, SubmitResult,
};
use serde_json::json;

use crate::common::journal;

/// What an adapter contract supplies: how to hash a payload, submit it and
/// check its status by request identifier.
#[allow(async_fn_in_trait)]
pub trait Contract {
    fn system(&self) -> &'static str;
    fn operation(&self) -> &'static str;
    /// Declared idempotent by request identifier.
    fn idempotent(&self) -> bool;
    /// The request identifier's validity window at the service.
    fn validity_ms(&self) -> u64;
    fn payload_hash(&self, payload: &str) -> Result<String, String>;
    async fn submit(&self, request_id: &str, not_after_ms: u64, payload: &str) -> SubmitResult;
    async fn status(&self, request_id: &str, not_after_ms: u64) -> StatusObservation;
    /// Harness hook: the crash and pause points. A no-op outside the experiment.
    async fn checkpoint(&self, point: &str, request_id: &str);
}

/// Where a delivery ended for now.
pub enum Delivered {
    Refused(String),
    Definitive { request_id: String, summary: String },
    /// The outcome is not yet known; a later status check must follow, soon
    /// if the status endpoint did not answer, otherwise once the validity
    /// window has closed and a not-received reply certifies non-start.
    Pending { request_id: String, not_after_ms: u64, retry_soon: bool, summary: String },
}

impl Delivered {
    pub fn summary(&self) -> String {
        match self {
            Self::Refused(reason) => json!({ "refused": reason }).to_string(),
            Self::Definitive { summary, .. } | Self::Pending { summary, .. } => summary.clone(),
        }
    }
}

pub struct Adapter<'a, C: Contract> {
    pub run: &'a str,
    pub name: &'a str,
    pub contract: &'a C,
}

impl<C: Contract> Adapter<'_, C> {
    fn source(&self) -> Source {
        Source::external(self.contract.system(), self.name)
    }

    pub async fn deliver(&self, activity: &str, authorization: &str, expires_at_ms: u64, payload: &str) -> Delivered {
        let operation = self.contract.operation();
        let hash = match self.contract.payload_hash(payload) {
            Ok(hash) => hash,
            Err(e) => return Delivered::Refused(format!("malformed payload: {e}")),
        };
        let binding = authorization_binding(&hash, operation, self.name, activity, expires_at_ms);
        let rid = request_id(activity, operation, authorization);
        // Durable: the same delivery resuming after a failure has the same token.
        let delivery = golem_rust::generate_idempotency_key().to_string();

        self.contract.checkpoint("pre", &rid).await;
        let check = Command::CheckLive {
            authorization: authorization.into(),
            binding: binding.clone(),
            delivery: delivery.clone(),
        };
        if let Response::Refused { reason, .. } = journal(self.run, &self.source(), check).await {
            return Delivered::Refused(reason);
        }
        self.contract.checkpoint("P1", &rid).await;

        let input = format!("{authorization}/{delivery}");
        let result: Result<String, String> =
            Durability::<String, String>::new("orca-adapter", "consume-and-dispatch", DurableFunctionType::WriteRemote, &input)
                .with_forced_commit()
                .run_async(|| async {
                    let delivered = self.consume_and_dispatch(activity, authorization, &binding, &delivery, payload).await;
                    Ok(encode(&delivered))
                })
                .await;

        decode(&result.unwrap_or_else(|e| e))
    }

    async fn consume_and_dispatch(&self, activity: &str, authorization: &str, binding: &str, delivery: &str, payload: &str) -> Delivered {
        let consume = Command::ConsumeIfLive {
            authorization: authorization.into(),
            binding: binding.into(),
            delivery: delivery.into(),
            validity_ms: self.contract.validity_ms(),
            reservation: Some(format!("{activity}:reserve:{authorization}")),
        };

        match journal(self.run, &self.source(), consume).await {
            Response::Consumed { request_id, not_after_ms, .. } => {
                self.dispatch(&request_id, not_after_ms, delivery, payload).await
            }
            // The same delivery, re-run after a failure: never dispatch blindly.
            Response::AlreadyConsumed { request_id, not_after_ms, .. } => {
                self.contract.checkpoint("recover", &request_id).await;
                self.reconcile(&request_id, not_after_ms, Some(delivery), payload).await
            }
            Response::Refused { reason, .. } => Delivered::Refused(reason),
            other => Delivered::Refused(format!("unexpected journal response: {other:?}")),
        }
    }

    /// The first dispatch, only after the consumption record committed.
    async fn dispatch(&self, rid: &str, not_after_ms: u64, delivery: &str, payload: &str) -> Delivered {
        self.contract.checkpoint("P2", rid).await;
        self.contract.checkpoint("C1", rid).await;

        let result = self.contract.submit(rid, not_after_ms, payload).await;
        self.contract.checkpoint("C2", rid).await;

        let submission = Command::Submission { request_id: rid.into(), delivery: delivery.into() };
        journal(self.run, &self.source(), submission).await;
        self.contract.checkpoint("C3", rid).await;

        self.receipt(rid, not_after_ms, delivery, payload, result).await
    }

    async fn receipt(&self, rid: &str, not_after_ms: u64, delivery: &str, payload: &str, result: SubmitResult) -> Delivered {
        let no_reply = matches!(result, SubmitResult::NoReply { .. });
        let update = Command::ReceiptUpdate { request_id: rid.into(), result: result.clone() };
        journal(self.run, &self.source(), update).await;

        if no_reply {
            // A lost acknowledgment: find out by status before anything else.
            return self.reconcile(rid, not_after_ms, Some(delivery), payload).await;
        }
        Delivered::Definitive {
            request_id: rid.into(),
            summary: json!({ "request_id": rid, "result": result }).to_string(),
        }
    }

    /// A status check and its result, then, for a delivery resuming, a
    /// re-dispatch only if the status says not received and the journal writer
    /// finds everything still live. A scheduled check passes no delivery and
    /// never re-dispatches.
    pub async fn reconcile(&self, rid: &str, not_after_ms: u64, delivery: Option<&str>, payload: &str) -> Delivered {
        let observation = self.contract.status(rid, not_after_ms).await;
        let reconciliation = Command::Reconciliation { request_id: rid.into(), observation: observation.clone() };
        journal(self.run, &self.source(), reconciliation).await;

        let definitive = matches!(
            observation,
            StatusObservation::Accepted { .. } | StatusObservation::Refused { .. } | StatusObservation::NotReceived { certified: true }
        );
        if definitive {
            return Delivered::Definitive {
                request_id: rid.into(),
                summary: json!({ "request_id": rid, "reconciled": observation }).to_string(),
            };
        }

        if let (Some(delivery), RecoveryAction::ReDispatchIfLive) = (delivery, decide(&observation)) {
            let ask = Command::ReDispatchIfLive {
                request_id: rid.into(),
                delivery: delivery.into(),
                idempotent: self.contract.idempotent(),
            };
            if let Response::ReDispatchAllowed { .. } = journal(self.run, &self.source(), ask).await {
                let result = self.contract.submit(rid, not_after_ms, payload).await;
                let update = Command::ReceiptUpdate { request_id: rid.into(), result: result.clone() };
                journal(self.run, &self.source(), update).await;

                if !matches!(result, SubmitResult::NoReply { .. }) {
                    return Delivered::Definitive {
                        request_id: rid.into(),
                        summary: json!({ "request_id": rid, "re_dispatched": result }).to_string(),
                    };
                }
            }
        }

        Delivered::Pending {
            request_id: rid.into(),
            not_after_ms,
            retry_soon: matches!(observation, StatusObservation::Unavailable { .. }),
            summary: json!({ "request_id": rid, "pending": observation }).to_string(),
        }
    }
}

// The durable operation records its result as a string.
fn encode(delivered: &Delivered) -> String {
    match delivered {
        Delivered::Refused(reason) => json!({ "kind": "refused", "reason": reason }),
        Delivered::Definitive { request_id, summary } => json!({ "kind": "definitive", "request_id": request_id, "summary": summary }),
        Delivered::Pending { request_id, not_after_ms, retry_soon, summary } => json!({
            "kind": "pending",
            "request_id": request_id,
            "not_after_ms": not_after_ms,
            "retry_soon": retry_soon,
            "summary": summary,
        }),
    }
    .to_string()
}

fn decode(text: &str) -> Delivered {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();

    match v["kind"].as_str() {
        Some("definitive") => Delivered::Definitive { request_id: s("request_id"), summary: s("summary") },
        Some("pending") => Delivered::Pending {
            request_id: s("request_id"),
            not_after_ms: v["not_after_ms"].as_u64().unwrap_or_default(),
            retry_soon: v["retry_soon"].as_bool().unwrap_or_default(),
            summary: s("summary"),
        },
        Some("refused") => Delivered::Refused(s("reason")),
        _ => Delivered::Refused(format!("undecodable durable result: {text}")),
    }
}
