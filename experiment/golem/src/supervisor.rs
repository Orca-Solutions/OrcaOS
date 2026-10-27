// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Acts for principal-1: opens the activities, records the grant table,
//! revokes, cancels, closes and replaces participants. Every change is
//! journaled first; Golem's cards then enforce what the journal says.
//!
//! Golem 1.6 cards attenuate: a card installed into an agent narrows what the
//! agent already holds, and a card is derived only from the deriving agent's
//! own wallet. So participants get their authority from their agent type's
//! initialCard, and revoking a participant natively means installing an
//! empty store card into it (see README, "Findings").
//!
//! Scorecard: counted. Category: grants. Tests: 3, 5, 7b; setup for all.

use golem_rust::permissions::{self, Bounds, Holder, Lifespan, PatternGrant, WalletCard};
use golem_rust::{agent_definition, agent_implementation};
use orca_ref::{Command, Response, Source};

use crate::common::{journal, PRINCIPAL};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;
/// A1's budget: 10 CPU-minutes.
pub const A1_BUDGET_MS: u64 = 10 * 60 * 1000;

/// The grant table from the experiment plan: (grant, holder, activity,
/// capability, object, parent).
pub const GRANT_TABLE: &[(&str, &str, &str, &str, &str, Option<&str>)] = &[
    ("g-read-c1", "A1", "A1", "read", "c1@v1", None),
    ("g-read-c2", "A1", "A1", "read", "c2@v1", None),
    ("g-read-c3", "A1", "A1", "read", "c3@v1", None),
    ("g-out", "A1", "A1", "append", "out/A1", None),
    ("g-send", "A1", "A1", "send", "accountant@mock.test", None),
    ("g-p1-read-c1", "processor-1", "A1.1", "read", "c1@v1", Some("g-read-c1")),
    ("g-p1-read-c2", "processor-1", "A1.1", "read", "c2@v1", Some("g-read-c2")),
    ("g-p1-append", "processor-1", "A1.1", "append", "out/A1.1", Some("g-out")),
    ("g-p2-read-c3", "processor-2", "A1.2", "read", "c3@v1", Some("g-read-c3")),
    ("g-p2-append", "processor-2", "A1.2", "append", "out/A1.2", Some("g-out")),
    ("g-d-read-a11", "drafter", "A1", "read", "out/A1.1", Some("g-out")),
    ("g-d-read-a12", "drafter", "A1", "read", "out/A1.2", Some("g-out")),
    ("g-d-write", "drafter", "A1", "write", "out/A1/draft", Some("g-out")),
    ("g-d-propose", "drafter", "A1", "submit-proposal", "approval-path", Some("g-send")),
];

#[agent_definition]
pub trait Supervisor {
    fn new(run: String) -> Self;
    async fn setup(&mut self) -> String;
    async fn grant(&mut self, grant_id: String, holder: String, activity: String, capability: String, object: String) -> String;
    async fn revoke(&mut self, grant_id: String) -> String;
    async fn cancel(&mut self, activity: String) -> String;
    async fn close(&mut self, activity: String) -> String;
    /// Replaces a participant: journals the revocation of the old holder's
    /// grants and equivalent grants for the new holder, then natively narrows
    /// the old participant's agent to nothing.
    async fn replace(&mut self, activity: String, old_holder: String, old_agent: String, new_holder: String) -> String;
    /// Records denials the inspector read from Golem's oplog, with Golem's
    /// key-value store as their source.
    async fn record_denials(&mut self, denials: String) -> String;
}

struct SupervisorImpl {
    run: String,
}

fn principal() -> Source {
    Source::principal(PRINCIPAL)
}

fn summary(response: &Response) -> String {
    serde_json::to_string(response).unwrap()
}

/// The recipient prefix (account/application/environment/component) this
/// agent's own store grants are addressed to, read from its wallet.
fn component_recipient() -> Option<String> {
    for card in permissions::wallet() {
        let view = match &card {
            WalletCard::Template(c) => c.inspect(),
            WalletCard::Concrete(c) => c.inspect(),
            WalletCard::Scope(_) => continue,
        };
        if let Ok(view) = view {
            if let Some(g) = view.lower_positive.iter().find(|g| g.class == "kv") {
                return g.recipient.strip_suffix("/*").map(str::to_string);
            }
        }
    }
    None
}

#[agent_implementation]
impl Supervisor for SupervisorImpl {
    fn new(run: String) -> Self {
        Self { run }
    }

    async fn setup(&mut self) -> String {
        let run = self.run.clone();
        let mut out = Vec::new();

        for (activity, parent, policy) in [
            ("A1", None, vec!["send".to_string()]),
            ("A1.1", Some("A1"), vec![]),
            ("A1.2", Some("A1"), vec![]),
        ] {
            let command = Command::OpenActivity {
                activity: activity.into(),
                parent: parent.map(str::to_string),
                approval_policy: policy,
            };
            out.push(summary(&journal(&run, &principal(), command).await));
        }

        let expires = crate::common::now_ms() + DAY_MS;
        for (grant_id, holder, activity, capability, object, parent) in GRANT_TABLE {
            let command = Command::Grant {
                grant_id: grant_id.to_string(),
                holder: holder.to_string(),
                activity: activity.to_string(),
                capability: capability.to_string(),
                object: object.to_string(),
                parent: parent.map(str::to_string),
                // Everything delegated to a child carries a 24-hour lifespan.
                expires_at_ms: parent.map(|_| expires),
            };
            let source = if parent.is_some() { Source::service("activity-supervisor") } else { principal() };
            out.push(summary(&journal(&run, &source, command).await));
        }

        crate::journal_agent::JournalClient::get(run.clone())
            .set_budget("A1".into(), A1_BUDGET_MS)
            .await;
        for activity in ["A1", "A1.1", "A1.2"] {
            let command = Command::Transition {
                activity: activity.into(),
                to: orca_ref::ActivityState::Running,
            };
            journal(&run, &principal(), command).await;
        }

        format!("[{}]", out.join(","))
    }

    async fn grant(&mut self, grant_id: String, holder: String, activity: String, capability: String, object: String) -> String {
        let command = Command::Grant {
            grant_id,
            holder,
            activity,
            capability,
            object,
            parent: None,
            expires_at_ms: Some(crate::common::now_ms() + DAY_MS),
        };
        summary(&journal(&self.run, &principal(), command).await)
    }

    async fn revoke(&mut self, grant_id: String) -> String {
        summary(&journal(&self.run, &principal(), Command::Revoke { grant_id }).await)
    }

    async fn cancel(&mut self, activity: String) -> String {
        summary(&journal(&self.run, &principal(), Command::Cancel { activity }).await)
    }

    async fn close(&mut self, activity: String) -> String {
        summary(&journal(&self.run, &principal(), Command::Close { activity }).await)
    }

    async fn replace(&mut self, activity: String, old_holder: String, old_agent: String, new_holder: String) -> String {
        let run = self.run.clone();
        let mut out = Vec::new();

        let replaced: Vec<_> = GRANT_TABLE.iter().filter(|g| g.1 == old_holder && g.2 == activity).collect();
        for (grant_id, ..) in &replaced {
            out.push(summary(&journal(&run, &principal(), Command::Revoke { grant_id: grant_id.to_string() }).await));
        }
        for (grant_id, _, activity, capability, object, parent) in &replaced {
            let command = Command::Grant {
                grant_id: format!("{grant_id}-{new_holder}"),
                holder: new_holder.clone(),
                activity: activity.to_string(),
                capability: capability.to_string(),
                object: object.to_string(),
                parent: parent.map(str::to_string),
                expires_at_ms: Some(crate::common::now_ms() + DAY_MS),
            };
            out.push(summary(&journal(&run, &Source::service("activity-supervisor"), command).await));
        }

        // Natively: narrow the old participant's store access to an object
        // that does not exist. Cards in a wallet intersect, so this removes
        // every store grant its initial card gave it.
        let native = match component_recipient() {
            None => "no component recipient in wallet".to_string(),
            Some(base) => {
                let recipient = format!("{base}/{old_agent}");
                let grants = |rec: &str| {
                    vec![
                        PatternGrant::new("kv", "?env", rec, "read", "revoked.none"),
                        PatternGrant::new("kv", "?env", rec, "write", "revoked.none"),
                    ]
                };
                let bounds = Bounds {
                    lower_positive: grants(&recipient),
                    upper_positive: grants(&format!("{base}/*")),
                    ..Bounds::default()
                };
                let lifespan = Lifespan::iso8601("P1D").unwrap();
                match (permissions::derive_from_wallet(&bounds, Some(&lifespan)), golem_rust::resolve_agent_id("scenario-a:main", &old_agent)) {
                    (Ok(card), Some(target)) => format!("{:?}", card.install(&Holder::Agent(target))),
                    (Err(e), _) => format!("derive failed: {e:?}"),
                    (_, None) => format!("no such agent: {old_agent}"),
                }
            }
        };

        format!("{{\"journal\":[{}],\"native\":{}}}", out.join(","), serde_json::to_string(&native).unwrap())
    }

    async fn record_denials(&mut self, denials: String) -> String {
        let denials: Vec<(String, String, String, String)> = match serde_json::from_str(&denials) {
            Ok(d) => d,
            Err(e) => return format!("malformed: {e}"),
        };
        let mut out = Vec::new();

        for (holder, operation, object, reason) in denials {
            let command = Command::Denial { holder, operation, object, reason };
            out.push(summary(&journal(&self.run, &Source::service("golem-kv"), command).await));
        }
        format!("[{}]", out.join(","))
    }
}
