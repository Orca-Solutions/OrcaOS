// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The delivery protocol for one consequential operation, independent of the
//! external system: early check, consume-if-live, dispatch only after the
//! consumption commits, record the submission and the receipt, and on
//! recovery check status before anything else.
//!
//! Temporal runs this as an activity and retries an attempt that crashed,
//! with the same delivery token. A retry re-runs this code; it resends
//! nothing by itself. The journal tells a retry that its delivery already
//! consumed the authorization, which puts it on the recovery path.
//!
//! Scorecard: counted. Category: recovery (approvals for the binding check).
//! Tests: 2, 3, 4, 7. Developer model: reusable for any adapter contract.

use orca_ref::{
    authorization_binding, decide, request_id, Command, RecoveryAction, Response, Source, StatusObservation, SubmitResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::journal;

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
#[derive(Clone, Debug, Serialize, Deserialize)]
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

    async fn journal(&self, command: Command) -> anyhow::Result<Response> {
        journal::apply(self.run, &self.source(), command).await
    }

    /// `delivery` names this delivery of the authorization; a retry of the
    /// same delivery carries the same token.
    pub async fn deliver(&self, activity: &str, authorization: &str, expires_at_ms: u64, payload: &str, delivery: &str) -> anyhow::Result<Delivered> {
        let operation = self.contract.operation();
        let hash = match self.contract.payload_hash(payload) {
            Ok(hash) => hash,
            Err(e) => return Ok(Delivered::Refused(format!("malformed payload: {e}"))),
        };
        let binding = authorization_binding(&hash, operation, self.name, activity, expires_at_ms);
        let rid = request_id(activity, operation, authorization);

        self.contract.checkpoint("pre", &rid).await;
        let check = Command::CheckLive {
            authorization: authorization.into(),
            binding: binding.clone(),
            delivery: delivery.into(),
        };
        if let Response::Refused { reason, .. } = self.journal(check).await? {
            return Ok(Delivered::Refused(reason));
        }
        self.contract.checkpoint("P1", &rid).await;

        let consume = Command::ConsumeIfLive {
            authorization: authorization.into(),
            binding,
            delivery: delivery.into(),
            validity_ms: self.contract.validity_ms(),
            reservation: Some(format!("{activity}:reserve:{authorization}")),
        };
        match self.journal(consume).await? {
            Response::Consumed { request_id, not_after_ms, .. } => self.dispatch(&request_id, not_after_ms, delivery, payload).await,
            // The same delivery, retried after a failure: never dispatch blindly.
            Response::AlreadyConsumed { request_id, not_after_ms, .. } => {
                self.contract.checkpoint("recover", &request_id).await;
                self.reconcile(&request_id, not_after_ms, Some(delivery), payload).await
            }
            Response::Refused { reason, .. } => Ok(Delivered::Refused(reason)),
            other => Ok(Delivered::Refused(format!("unexpected journal response: {other:?}"))),
        }
    }

    /// The first dispatch, only after the consumption record committed.
    async fn dispatch(&self, rid: &str, not_after_ms: u64, delivery: &str, payload: &str) -> anyhow::Result<Delivered> {
        self.contract.checkpoint("P2", rid).await;
        self.contract.checkpoint("C1", rid).await;

        let result = self.contract.submit(rid, not_after_ms, payload).await;
        self.contract.checkpoint("C2", rid).await;

        self.journal(Command::Submission { request_id: rid.into(), delivery: delivery.into() }).await?;
        self.contract.checkpoint("C3", rid).await;

        self.receipt(rid, not_after_ms, delivery, payload, result).await
    }

    async fn receipt(&self, rid: &str, not_after_ms: u64, delivery: &str, payload: &str, result: SubmitResult) -> anyhow::Result<Delivered> {
        let no_reply = matches!(result, SubmitResult::NoReply { .. });
        self.journal(Command::ReceiptUpdate { request_id: rid.into(), result: result.clone() }).await?;

        if no_reply {
            // A lost acknowledgment: find out by status before anything else.
            return self.reconcile(rid, not_after_ms, Some(delivery), payload).await;
        }
        Ok(Delivered::Definitive {
            request_id: rid.into(),
            summary: json!({ "request_id": rid, "result": result }).to_string(),
        })
    }

    /// A status check and its result, then, for a delivery resuming, a
    /// re-dispatch only if the status says not received and the journal writer
    /// finds everything still live. A scheduled check passes no delivery and
    /// never re-dispatches.
    pub async fn reconcile(&self, rid: &str, not_after_ms: u64, delivery: Option<&str>, payload: &str) -> anyhow::Result<Delivered> {
        let observation = self.contract.status(rid, not_after_ms).await;
        self.journal(Command::Reconciliation { request_id: rid.into(), observation: observation.clone() }).await?;

        let definitive = matches!(
            observation,
            StatusObservation::Accepted { .. } | StatusObservation::Refused { .. } | StatusObservation::NotReceived { certified: true }
        );
        if definitive {
            return Ok(Delivered::Definitive {
                request_id: rid.into(),
                summary: json!({ "request_id": rid, "reconciled": observation }).to_string(),
            });
        }

        if let (Some(delivery), RecoveryAction::ReDispatchIfLive) = (delivery, decide(&observation)) {
            let ask = Command::ReDispatchIfLive {
                request_id: rid.into(),
                delivery: delivery.into(),
                idempotent: self.contract.idempotent(),
            };
            if let Response::ReDispatchAllowed { .. } = self.journal(ask).await? {
                let result = self.contract.submit(rid, not_after_ms, payload).await;
                self.journal(Command::ReceiptUpdate { request_id: rid.into(), result: result.clone() }).await?;

                if !matches!(result, SubmitResult::NoReply { .. }) {
                    return Ok(Delivered::Definitive {
                        request_id: rid.into(),
                        summary: json!({ "request_id": rid, "re_dispatched": result }).to_string(),
                    });
                }
            }
        }

        Ok(Delivered::Pending {
            request_id: rid.into(),
            not_after_ms,
            retry_soon: matches!(observation, StatusObservation::Unavailable { .. }),
            summary: json!({ "request_id": rid, "pending": observation }).to_string(),
        })
    }
}
