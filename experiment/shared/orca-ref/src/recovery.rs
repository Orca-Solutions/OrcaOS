// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The recovery rule for a request whose outcome is not yet known.
//!
//! A status check comes first. Only a "not received" reply that does not yet
//! certify non-start leads to a re-dispatch attempt, and the journal writer
//! still refuses it unless the activity is open, the grant live and the
//! operation idempotent. Everything else is a reconciliation: it queries, it
//! never acts.

use crate::model::StatusObservation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Record the status result as a reconciliation; dispatch nothing.
    Reconcile,
    /// Ask the journal writer for a live re-dispatch check; reconcile if refused.
    ReDispatchIfLive,
}

pub fn decide(observation: &StatusObservation) -> RecoveryAction {
    match observation {
        StatusObservation::NotReceived { certified: false } => RecoveryAction::ReDispatchIfLive,
        // Accepted, refused and certified non-start are definitive. An
        // unavailable status endpoint means the service may already hold the
        // request, so nothing is sent.
        StatusObservation::Accepted { .. }
        | StatusObservation::Refused { .. }
        | StatusObservation::NotReceived { certified: true }
        | StatusObservation::Unavailable { .. } => RecoveryAction::Reconcile,
    }
}
