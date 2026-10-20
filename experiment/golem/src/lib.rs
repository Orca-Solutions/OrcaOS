// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scenario A on Golem 1.6.0. See ../README.md for the layout and findings.

mod activity;
mod adapter;
mod approval;
mod common;
mod journal_agent;
mod participants;
mod seeder;
mod supervisor;

pub use activity::*;
pub use adapter::*;
pub use approval::*;
pub use journal_agent::*;
pub use participants::*;
pub use seeder::*;
pub use supervisor::*;
