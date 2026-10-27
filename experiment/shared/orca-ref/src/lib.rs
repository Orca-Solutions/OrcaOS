// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The shared reference implementation for the Scenario A experiment.
//!
//! It holds the parts of the protocol that every platform build must behave
//! identically on: request identifiers, payload binding, the journal with its
//! consume-if-live rule, receipts on two axes, and the recovery decision for a
//! request whose outcome is not yet known. It does no I/O and keeps no clock;
//! each platform hosts it, persists it and supplies the time.
//!
//! The experiment plan excludes this crate from the custom-machinery ratio and
//! reports it separately, as part of what OrcaOS would have to maintain.

pub mod binding;
pub mod journal;
pub mod model;
pub mod recovery;

pub use binding::{authorization_binding, payload_hash, Attachment, Message};
pub use journal::{Command, Entry, Journal, Kind, Response};
pub use model::*;
pub use recovery::{decide, RecoveryAction};

/// The request identifier format fixed by the protocol:
/// `<activity>:<operation>:<authorization>`, for example `A1:send:auth-0001`.
pub fn request_id(activity: &str, operation: &str, authorization: &str) -> String {
    format!("{activity}:{operation}:{authorization}")
}
