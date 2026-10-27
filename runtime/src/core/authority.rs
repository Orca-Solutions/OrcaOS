// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Grants and authorizations (Decision log, 2026-10-20: "Authorization is
//! per-action").
//!
//! A grant names the object, the operation, the holder, the duration and the
//! principal. It is never itself enough to dispatch a consequential action:
//! for every one, the enforcement core mints a single-use authorization from
//! a grant or, where the approval policy marks the operation, from an
//! approval. The authorization is bound to the payload hash, the operation,
//! the adapter, the activity and an expiry. It lives inside the core and is
//! never serialized or handed to an app or a routine; the only thing that
//! leaves the core is a `Consumed`, and only `Core::consume_if_live` makes one.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};

use crate::model::{GrantKind, object_matches};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Holder {
    Activity(String),
    App(String),
    Support(String),
}

impl Holder {
    pub fn label(&self) -> String {
        match self {
            Self::Activity(id) => format!("activity:{id}"),
            Self::App(id) => format!("app:{id}"),
            Self::Support(id) => format!("support:{id}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantState {
    Live,
    /// A one-time grant that has been used.
    Used,
    Revoked,
    Expired,
    /// Ended with its activity, or by a restart.
    Ended,
    /// An install grant of an app that is switched off: kept, not live, and
    /// shown again for confirmation when the app is switched on.
    Held,
}

#[derive(Clone, Debug, Serialize)]
pub struct Grant {
    pub id: String,
    pub holder: Holder,
    pub app: String,
    pub operation: String,
    pub object: String,
    pub principal: String,
    pub kind: GrantKind,
    pub bound_hash: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Set only inside the enforcement core.
    pub(in crate::core) state: GrantState,
    pub derived_from: Option<String>,
}

impl Grant {
    /// A new live grant. It confers nothing until `Core::issue_grant` has
    /// journaled it.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        holder: Holder,
        app: &str,
        operation: &str,
        object: &str,
        principal: &str,
        kind: GrantKind,
        bound_hash: Option<String>,
        expires_at: Option<DateTime<Utc>>,
        derived_from: Option<String>,
    ) -> Self {
        Self {
            id,
            holder,
            app: app.into(),
            operation: operation.into(),
            object: object.into(),
            principal: principal.into(),
            kind,
            bound_hash,
            expires_at,
            state: GrantState::Live,
            derived_from,
        }
    }

    pub fn state(&self) -> GrantState {
        self.state
    }

    pub fn covers(&self, operation: &str, object: &str) -> bool {
        self.operation == operation && object_matches(&self.object, object)
    }
}

/// What an authorization was minted from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Basis {
    /// A grant sufficient for the operation.
    Grant { grant: String, principal: String },
    /// A member's approval of a proposal, where the policy marks the operation.
    Approval { proposal: String, member: String },
}

impl Basis {
    /// On whose authority, as a ledger line says it.
    pub fn authority(&self) -> Value {
        match self {
            Self::Grant { grant, principal } => json!({ "grant": grant, "principal": principal }),
            Self::Approval { proposal, member } => json!({ "approval": proposal, "member": member }),
        }
    }

    pub fn grant(&self) -> Option<&str> {
        match self {
            Self::Grant { grant, .. } => Some(grant),
            Self::Approval { .. } => None,
        }
    }

    pub fn proposal(&self) -> Option<&str> {
        match self {
            Self::Approval { proposal, .. } => Some(proposal),
            Self::Grant { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthorizationState {
    Unconsumed,
    Consumed { request_id: String },
    Invalidated { reason: String },
}

/// A single-use authorization for one consequential action. Deliberately
/// neither `Serialize` nor public in its fields: it never leaves the core.
#[derive(Debug)]
pub struct Authorization {
    pub(in crate::core) id: String,
    pub(in crate::core) activity: String,
    pub(in crate::core) operation: String,
    pub(in crate::core) object: String,
    pub(in crate::core) adapter: String,
    pub(in crate::core) payload_hash: String,
    pub(in crate::core) basis: Basis,
    pub(in crate::core) expires_at: DateTime<Utc>,
    pub(in crate::core) state: AuthorizationState,
}

/// What the approval path and the enforcement check may know about an
/// authorization without holding it.
#[derive(Clone, Debug)]
pub struct AuthorizationView {
    pub id: String,
    pub payload_hash: String,
    pub expires_at: DateTime<Utc>,
    pub unconsumed: bool,
}

impl Authorization {
    pub(in crate::core) fn view(&self) -> AuthorizationView {
        AuthorizationView {
            id: self.id.clone(),
            payload_hash: self.payload_hash.clone(),
            expires_at: self.expires_at,
            unconsumed: self.state == AuthorizationState::Unconsumed,
        }
    }
}

/// Proof that an authorization was consumed and its consumption record
/// committed. An adapter dispatches only with one of these, takes it by
/// value, and cannot make one: so nothing is dispatched before its
/// consumption is in the journal, and nothing twice on one consumption.
#[derive(Debug)]
pub struct Consumed {
    pub request_id: String,
    pub authority: Value,
    pub grant: Option<String>,
    /// What was consumed, so the adapter sends exactly that.
    pub operation: String,
    pub object: String,
    pub payload_hash: String,
    _sealed: (),
}

impl Consumed {
    pub(in crate::core) fn new(request_id: String, authority: Value, grant: Option<String>, operation: String, object: String, payload_hash: String) -> Self {
        Self {
            request_id,
            authority,
            grant,
            operation,
            object,
            payload_hash,
            _sealed: (),
        }
    }

    /// Stops the runtime if an adapter is about to send something other than
    /// what was consumed. Reaching it is a bug, never a caller's doing.
    pub fn must_cover(&self, operation: &str, object: &str, payload_hash: &str) {
        assert!(
            self.operation == operation && self.object == object && self.payload_hash == payload_hash,
            "fail-stop: {} was consumed for {} on {}, not for {operation} on {object} with this payload",
            self.request_id,
            self.operation,
            self.object
        );
    }
}

/// Why consume-if-live did not consume: one of the three things it checks in
/// one step was not live when the consumption record would have committed.
#[derive(Debug, PartialEq, Eq)]
pub enum NotLive {
    Authorization { code: &'static str, why: String },
    Grant { code: &'static str, grant: String, why: String },
    Activity { code: &'static str, why: String },
}

impl NotLive {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Authorization { code, .. } | Self::Grant { code, .. } | Self::Activity { code, .. } => code,
        }
    }

    pub fn why(&self) -> &str {
        match self {
            Self::Authorization { why, .. } | Self::Grant { why, .. } | Self::Activity { why, .. } => why,
        }
    }
}
