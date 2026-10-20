// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The approval path: holds each proposal, shows the principal exactly what
//! will be sent with its payload hash, records the approval against that hash,
//! mints a single-use authorization and hands it straight to the mail adapter.
//! The drafter only ever holds its proposal's reference.
//!
//! Scorecard: counted. Category: approvals. Tests: 2, 3, 7.

use std::collections::BTreeMap;

use golem_rust::{agent_definition, agent_implementation};
use orca_ref::{payload_hash, Command, Message, Response, Source};
use serde_json::json;

use crate::adapter::MailAdapterClient;
use crate::common::{journal, now_ms, ADAPTER, PRINCIPAL};

/// How long a minted authorization stays usable.
const AUTHORIZATION_TTL_MS: u64 = 60 * 60 * 1000;

#[agent_definition]
pub trait ApprovalPath {
    fn new(run: String) -> Self;
    async fn submit(&mut self, activity: String, holder: String, message: String) -> String;
    fn display(&self, proposal_id: String) -> String;
    /// The principal approves what they were shown, by its hash, resting on
    /// the named send grant.
    async fn approve(&mut self, proposal_id: String, payload_hash: String, grant_id: String) -> String;
}

struct Proposal {
    activity: String,
    message: String,
    payload_hash: String,
}

struct ApprovalPathImpl {
    run: String,
    proposals: BTreeMap<String, Proposal>,
}

fn service() -> Source {
    Source::service("approval-path")
}

#[agent_implementation]
impl ApprovalPath for ApprovalPathImpl {
    fn new(run: String) -> Self {
        Self { run, proposals: BTreeMap::new() }
    }

    async fn submit(&mut self, activity: String, holder: String, message: String) -> String {
        let parsed: Message = match serde_json::from_str(&message) {
            Ok(m) => m,
            Err(e) => return format!("refused: malformed message: {e}"),
        };
        let hash = payload_hash(&parsed);
        let proposal_id = format!("prop-{}", self.proposals.len() + 1);

        let command = Command::Proposal {
            proposal_id: proposal_id.clone(),
            activity: activity.clone(),
            holder,
            payload_hash: hash.clone(),
        };
        if let Response::Refused { reason, .. } = journal(&self.run, &service(), command).await {
            return format!("refused: {reason}");
        }

        self.proposals.insert(proposal_id.clone(), Proposal { activity, message, payload_hash: hash });
        proposal_id
    }

    fn display(&self, proposal_id: String) -> String {
        match self.proposals.get(&proposal_id) {
            Some(p) => json!({
                "proposal_id": proposal_id,
                "activity": p.activity,
                "message": serde_json::from_str::<serde_json::Value>(&p.message).unwrap(),
                "payload_hash": p.payload_hash,
            })
            .to_string(),
            None => json!({ "error": "no such proposal" }).to_string(),
        }
    }

    async fn approve(&mut self, proposal_id: String, payload_hash: String, grant_id: String) -> String {
        let Some(proposal) = self.proposals.get(&proposal_id) else {
            return "refused: no such proposal".into();
        };
        let (activity, message, stored_hash) = (proposal.activity.clone(), proposal.message.clone(), proposal.payload_hash.clone());

        // The principal approves what was displayed; a changed payload is refused.
        let command = Command::Approval { proposal_id: proposal_id.clone(), payload_hash };
        if let Response::Refused { reason, .. } = journal(&self.run, &Source::principal(PRINCIPAL), command).await {
            return format!("refused: {reason}");
        }

        let expires_at_ms = now_ms() + AUTHORIZATION_TTL_MS;
        let command = Command::MintAuthorization {
            activity: activity.clone(),
            operation: "send".into(),
            adapter: ADAPTER.into(),
            payload_hash: stored_hash,
            expires_at_ms,
            grant_id,
            proposal_id: Some(proposal_id),
        };
        match journal(&self.run, &service(), command).await {
            Response::Minted { authorization, .. } => {
                // Straight to the adapter; no participant sees it.
                MailAdapterClient::get(self.run.clone()).trigger_deliver(activity, authorization.clone(), expires_at_ms, message);
                json!({ "authorization": authorization, "expires_at_ms": expires_at_ms }).to_string()
            }
            Response::Refused { reason, .. } => format!("refused: {reason}"),
            other => format!("unexpected: {other:?}"),
        }
    }
}
