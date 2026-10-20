// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The trusted mail adapter as a Golem agent. It is the only holder of the
//! mail credential and acts only on an authorization the approval path hands
//! it. Outcomes it cannot settle get a scheduled status check under its
//! standing reconciliation grant, which outlives cancellation and close.
//!
//! Scorecard: counted. Category: recovery (scheduling status checks, tracking
//! unknown outcomes). Tests: 4, 7. Developer model: reusable, except the
//! credential and validity settings, which are mock-mail-specific.

pub mod delivery;
pub mod mock_mail;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use golem_rust::agentic::{Config, Secret};
use golem_rust::{agent_definition, agent_implementation, ConfigSchema, ScheduledTime};

use self::delivery::{Adapter, Delivered};
use self::mock_mail::MockMail;
use crate::common::ADAPTER;

/// The protocol's validity window; the harness may shorten it for a run with
/// a dated protocol note, through the ORCA_VALIDITY_MS environment variable.
const DEFAULT_VALIDITY_MS: u64 = 10 * 60 * 1000;
/// The step between status checks that found nothing definitive, growing
/// with each check up to MAX_RETRY_MS.
const RETRY_MS: u64 = 2000;
const MAX_RETRY_MS: u64 = 5 * 60 * 1000;

#[derive(ConfigSchema)]
pub struct AdapterConfig {
    #[config_schema(secret)]
    pub mail_credential: Secret<String>,
}

#[agent_definition]
pub trait MailAdapter {
    fn new(run: String, #[agent_config] config: Config<AdapterConfig>) -> Self;
    async fn deliver(&mut self, activity: String, authorization: String, expires_at_ms: u64, message: String) -> String;
    /// A scheduled status check for an outcome not yet known.
    async fn reconcile(&mut self, request_id: String) -> String;
    fn pending(&self) -> String;
}

struct Pending {
    not_after_ms: u64,
    payload: String,
    checks: u32,
}

struct MailAdapterImpl {
    run: String,
    config: Config<AdapterConfig>,
    pending: BTreeMap<String, Pending>,
}

impl MailAdapterImpl {
    fn contract(&self) -> MockMail {
        let validity_ms = std::env::var("ORCA_VALIDITY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_VALIDITY_MS);
        MockMail {
            run: self.run.clone(),
            credential: self.config.get().expect("adapter config").mail_credential.get().unwrap_or_default(),
            validity_ms,
        }
    }

    fn schedule(&mut self, request_id: &str, at_ms: u64) {
        let at = Duration::from_millis(at_ms);
        let at = ScheduledTime { seconds: at.as_secs() as i64, nanoseconds: at.subsec_nanos() };

        let _ = MailAdapterClient::get(self.run.clone()).schedule_reconcile(request_id.to_string(), at);
    }

    fn settle(&mut self, delivered: &Delivered, payload: &str) {
        match delivered {
            Delivered::Pending { request_id, not_after_ms, retry_soon, .. } => {
                let entry = self.pending.entry(request_id.clone()).or_insert(Pending {
                    not_after_ms: *not_after_ms,
                    payload: payload.into(),
                    checks: 0,
                });
                entry.checks += 1;

                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
                // Keep checking until the outcome is definitive, backing off
                // so a long outage costs little; giving up would leave the
                // outcome Unknown and the reservation held for good.
                let at = if *retry_soon || now > entry.not_after_ms {
                    now + (RETRY_MS * entry.checks as u64).min(MAX_RETRY_MS)
                } else {
                    entry.not_after_ms + RETRY_MS
                };
                self.schedule(request_id, at);
            }
            Delivered::Definitive { request_id, .. } => {
                self.pending.remove(request_id);
            }
            Delivered::Refused(_) => {}
        }
    }
}

#[agent_implementation]
impl MailAdapter for MailAdapterImpl {
    fn new(run: String, #[agent_config] config: Config<AdapterConfig>) -> Self {
        Self { run, config, pending: BTreeMap::new() }
    }

    async fn deliver(&mut self, activity: String, authorization: String, expires_at_ms: u64, message: String) -> String {
        let contract = self.contract();
        let adapter = Adapter { run: &self.run, name: ADAPTER, contract: &contract };
        let delivered = adapter.deliver(&activity, &authorization, expires_at_ms, &message).await;

        self.settle(&delivered, &message);
        delivered.summary()
    }

    async fn reconcile(&mut self, request_id: String) -> String {
        let Some(p) = self.pending.get(&request_id) else {
            return "nothing pending".into();
        };
        let (not_after_ms, payload) = (p.not_after_ms, p.payload.clone());

        // Under the standing reconciliation grant: status checks and receipt
        // updates only, never a re-dispatch.
        let contract = self.contract();
        let adapter = Adapter { run: &self.run, name: ADAPTER, contract: &contract };
        let delivered = adapter.reconcile(&request_id, not_after_ms, None, &payload).await;

        self.settle(&delivered, &payload);
        delivered.summary()
    }

    fn pending(&self) -> String {
        let pending: Vec<_> = self.pending.iter().map(|(rid, p)| (rid.clone(), p.not_after_ms, p.checks)).collect();
        serde_json::to_string(&pending).unwrap()
    }
}
