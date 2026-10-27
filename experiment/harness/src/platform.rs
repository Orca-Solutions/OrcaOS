// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The build under test, chosen by ORCA_PLATFORM: `golem` (the default) or
//! `conventional`. The tests and their checks are the same for both; only
//! how a step is driven and where native evidence is read differ.
//!
//! Scorecard: excluded (test harness).

use std::time::Duration;

use crate::conventional::Conventional;
use crate::golem::Golem;

pub enum Platform {
    Golem(Golem),
    Conventional(Conventional),
}

impl Platform {
    pub fn from_env() -> Self {
        match std::env::var("ORCA_PLATFORM").as_deref() {
            Ok("conventional") => Self::Conventional(Conventional::from_env()),
            _ => Self::Golem(Golem::from_env()),
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Golem(_) => "Golem",
            Self::Conventional(_) => "the conventional composition",
        }
    }

    /// The durable-execution engine, which could resend on its own.
    pub fn engine(&self) -> &'static str {
        match self {
            Self::Golem(_) => "Golem",
            Self::Conventional(_) => "Temporal",
        }
    }

    /// The journal source a store denial is recorded under.
    pub fn store_source(&self) -> &'static str {
        match self {
            Self::Golem(_) => "golem-kv",
            Self::Conventional(_) => "container-fs",
        }
    }

    /// Where the store's own record of a refused call is read.
    pub fn denial_record(&self) -> &'static str {
        match self {
            Self::Golem(_) => "Golem's oplog",
            Self::Conventional(_) => "Wasmtime's trace",
        }
    }

    pub fn kill_and_restart(&self) -> Result<Duration, String> {
        match self {
            Self::Golem(g) => g.kill_and_restart(),
            Self::Conventional(c) => c.kill_and_restart(),
        }
    }
}
