// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The vocabulary the journal records, named as the Glossary names it.

use serde::{Deserialize, Serialize};

/// Who an entry comes from. Every journal entry carries one, so the inspector
/// never shows a participant's claim as an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A principal's own decision, such as a grant, a revocation or a cancellation.
    Principal { id: String },
    /// What a participant says happened.
    Participant { id: String },
    /// An observation by a named enforcement service.
    Service { name: String },
    /// A statement by a named external system, relayed by a named adapter.
    External { system: String, adapter: String },
}

impl Source {
    pub fn principal(id: &str) -> Self {
        Self::Principal { id: id.into() }
    }

    pub fn service(name: &str) -> Self {
        Self::Service { name: name.into() }
    }

    pub fn external(system: &str, adapter: &str) -> Self {
        Self::External {
            system: system.into(),
            adapter: adapter.into(),
        }
    }
}

/// The six lifecycle states of an activity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Created,
    Running,
    Waiting,
    Suspended,
    Closing,
    Closed,
}

impl ActivityState {
    /// Open activities may still exercise their grants.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Created | Self::Running | Self::Waiting)
    }
}

/// How much the system saw of an outcome. Kept apart from the outcome state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Claimed,
    Attested,
    Observed,
    Acknowledged,
}

/// What an unknown outcome was reconciled to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "snake_case")]
pub enum Resolution {
    Acknowledged { message_id: String },
    /// Refused by the external system, or certified not started.
    Failed { reason: FailureReason },
}

/// The outcome state of a consequential action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    /// The request left the adapter. Nothing is known about its effect.
    Submitted,
    /// The external system confirmed acceptance.
    Acknowledged { message_id: String },
    /// Consumed or submitted, with no acknowledgment or definitive failure seen.
    Unknown,
    /// Refused by the external system, or certified not started.
    Failed { reason: FailureReason },
    /// An unknown outcome resolved later, recording what it resolved to and how.
    Reconciled { resolution: Resolution, evidence: Evidence },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum FailureReason {
    Refused { reason: String },
    CertifiedNotStarted,
}

impl Outcome {
    /// Definitive outcomes need no further status check.
    pub fn is_definitive(&self) -> bool {
        !matches!(self, Self::Submitted | Self::Unknown)
    }
}

/// What a status check by request identifier returned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum StatusObservation {
    Accepted { message_id: String },
    Refused { reason: String },
    /// Not received. `certified` is true only when the adapter contract names the
    /// reply as a certification that the request can no longer start.
    NotReceived { certified: bool },
    /// The status endpoint could not be reached or did not answer.
    Unavailable { detail: String },
}

/// What a submission returned to the adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum SubmitResult {
    Accepted { message_id: String },
    Refused { reason: String },
    /// No reply arrived, or the connection failed after the request may have left.
    NoReply { detail: String },
}

/// Why an authorization exists: a sufficient grant, or a grant plus an approval
/// where the activity's approval policy marks the operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum Basis {
    Grant { grant_id: String },
    Approval { grant_id: String, proposal_id: String },
}

impl Basis {
    pub fn grant_id(&self) -> &str {
        match self {
            Self::Grant { grant_id } | Self::Approval { grant_id, .. } => grant_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AuthorizationState {
    Unconsumed,
    Consumed { request_id: String, delivery: String },
    Invalidated { reason: String },
}
