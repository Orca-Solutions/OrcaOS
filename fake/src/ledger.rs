// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The ledger: the journal as the shell reads it. Every entry carries its
//! source, and receipts carry an outcome and an evidence level kept apart, so a
//! participant's claim is never shown as an observation.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Who an entry comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A member's own decision, given through the shell.
    Principal { id: String },
    /// What an app or a routine says.
    Participant { id: String },
    /// What Diamond Dog saw: a named enforcement service.
    Service { name: String },
    /// What a named outside system said, through a named adapter.
    External { system: String, adapter: String },
}

impl Source {
    pub fn principal(id: &str) -> Self {
        Self::Principal { id: id.into() }
    }

    pub fn participant(id: &str) -> Self {
        Self::Participant { id: id.into() }
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

/// One ledger line. Fields that do not apply are left out of the JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub at: String,
    /// The same instant on the household's wall clock.
    pub local: String,
    pub kind: String,
    pub source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// The member on whose authority this happened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Receipts only: submitted, acknowledged, unknown, failed or reconciled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Value>,
    /// Claimed, attested, observed or acknowledged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// On whose authority: a grant, or an approval by a named member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Value>,
    /// Refusals only: a stable code and a plain reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    /// Members who may also read this line, beyond the usual audience: a
    /// child's guardians see every read of the child's records.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_to: Option<Vec<String>>,
    /// The line as the shell would say it.
    pub plain: String,
}

impl Entry {
    pub fn new(kind: &str, source: Source, plain: impl Into<String>) -> Self {
        Self {
            seq: 0,
            at: String::new(),
            local: String::new(),
            kind: kind.into(),
            source,
            app: None,
            activity: None,
            member: None,
            holder: None,
            operation: None,
            object: None,
            grant: None,
            proposal: None,
            request_id: None,
            outcome: None,
            evidence: None,
            authority: None,
            reason: None,
            detail: None,
            visible_to: None,
            plain: plain.into(),
        }
    }

    pub fn app(mut self, v: impl Into<String>) -> Self {
        self.app = Some(v.into());
        self
    }

    pub fn activity(mut self, v: impl Into<String>) -> Self {
        self.activity = Some(v.into());
        self
    }

    pub fn member(mut self, v: impl Into<String>) -> Self {
        self.member = Some(v.into());
        self
    }

    pub fn holder(mut self, v: impl Into<String>) -> Self {
        self.holder = Some(v.into());
        self
    }

    pub fn operation(mut self, v: impl Into<String>) -> Self {
        self.operation = Some(v.into());
        self
    }

    pub fn object(mut self, v: impl Into<String>) -> Self {
        self.object = Some(v.into());
        self
    }

    pub fn grant(mut self, v: impl Into<String>) -> Self {
        self.grant = Some(v.into());
        self
    }

    pub fn proposal(mut self, v: impl Into<String>) -> Self {
        self.proposal = Some(v.into());
        self
    }

    pub fn request_id(mut self, v: impl Into<String>) -> Self {
        self.request_id = Some(v.into());
        self
    }

    pub fn outcome(mut self, v: Value) -> Self {
        self.outcome = Some(v);
        self
    }

    pub fn evidence(mut self, v: &str) -> Self {
        self.evidence = Some(v.into());
        self
    }

    pub fn authority(mut self, v: Value) -> Self {
        self.authority = Some(v);
        self
    }

    pub fn detail(mut self, v: Value) -> Self {
        self.detail = Some(v);
        self
    }

    pub fn reason(mut self, code: &str, text: &str) -> Self {
        self.reason = Some(json!({ "code": code, "text": text }));
        self
    }

    /// Sets a field only when a value is known.
    pub fn maybe(self, set: fn(Self, String) -> Self, v: Option<String>) -> Self {
        match v {
            Some(v) => set(self, v),
            None => self,
        }
    }
}
