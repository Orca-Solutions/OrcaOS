// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The activity lifecycle: six states and only the transitions the contract
//! names (surface 2). Code outside the enforcement core reads an activity's
//! state but cannot set it; every change goes through `Core::transition`,
//! which refuses any other move and journals the ones it makes.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub fn is_live(self) -> bool {
        matches!(self, Self::Created | Self::Running | Self::Waiting)
    }

    pub fn is_ending(self) -> bool {
        matches!(self, Self::Closing | Self::Closed)
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Suspended => "suspended",
            Self::Closing => "closing",
            Self::Closed => "closed",
        }
    }

    /// The contract's transitions, and no others: Created to Running; Running
    /// to Waiting and back; Running to Suspended and back; any open state to
    /// Closing; Closing to Closed.
    pub fn may_become(self, to: Self) -> bool {
        use ActivityState::*;
        match (self, to) {
            (Created, Running) => true,
            (Running, Waiting) | (Waiting, Running) => true,
            (Running, Suspended) | (Suspended, Running) => true,
            (from, Closing) => !from.is_ending(),
            (Closing, Closed) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Activity {
    pub id: String,
    pub app: String,
    pub routine: String,
    /// Set only by `Core::transition`.
    pub(in crate::core) state: ActivityState,
    /// The member on whose authority the activity runs. Software is never a principal.
    pub acts_for: String,
    /// Members who may read the activity and be notified about it.
    pub audience: BTreeSet<String>,
    pub parent: Option<String>,
    pub context: Value,
    pub require_approval: BTreeSet<String>,
    pub opened_at: DateTime<Utc>,
    pub deadline: DateTime<Utc>,
    pub cost_cents: u64,
    pub cost_allowance_cents: Option<u64>,
    pub concurrency_key: Option<String>,
    pub used_grants: BTreeSet<String>,
    pub loosened: BTreeSet<String>,
}

impl Activity {
    /// A new activity, in Created.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        app: &str,
        routine: &str,
        acts_for: &str,
        audience: BTreeSet<String>,
        parent: Option<String>,
        context: Value,
        require_approval: BTreeSet<String>,
        opened_at: DateTime<Utc>,
        deadline: DateTime<Utc>,
        cost_allowance_cents: Option<u64>,
        concurrency_key: Option<String>,
    ) -> Self {
        Self {
            id,
            app: app.into(),
            routine: routine.into(),
            state: ActivityState::Created,
            acts_for: acts_for.into(),
            audience,
            parent,
            context,
            require_approval,
            opened_at,
            deadline,
            cost_cents: 0,
            cost_allowance_cents,
            concurrency_key,
            used_grants: BTreeSet::new(),
            loosened: BTreeSet::new(),
        }
    }

    pub fn state(&self) -> ActivityState {
        self.state
    }
}

/// A move the lifecycle does not allow. Reaching one is a bug in the
/// runtime, not a refusal of a caller: the move is not made and not journaled.
#[derive(Debug)]
pub struct IllegalTransition {
    pub activity: String,
    pub from: ActivityState,
    pub to: ActivityState,
}

impl std::fmt::Display for IllegalTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "activity {} cannot go from {} to {}", self.activity, self.from.word(), self.to.word())
    }
}
