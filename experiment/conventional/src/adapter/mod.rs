// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The trusted mail adapter service: a Temporal worker on its own task queue,
//! in its own process, the only holder of the mail credential. It acts only on
//! an authorization the approval service hands it. One delivery workflow per
//! authorization delivers, then keeps checking status on a durable timer
//! until the outcome is definitive. That workflow is the standing
//! reconciliation grant: cancellation and close are journal commands, not
//! Temporal cancellations, so it outlives both, and its scheduled checks
//! never re-dispatch.
//!
//! Scorecard: counted. Category: recovery (scheduling status checks, tracking
//! unknown outcomes, the retried attempt's delivery token). Tests: 4, 7.
//! Developer model: reusable, except the credential and validity settings,
//! which are mock-mail-specific.

pub mod delivery;
pub mod mock_mail;

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use temporalio_macros::{activities, workflow, workflow_methods};
use temporalio_sdk::activities::{ActivityContext, ActivityError};
use temporalio_sdk::{ActivityOptions, WorkflowContext, WorkflowResult};

use self::delivery::{Adapter, Delivered};
use self::mock_mail::MockMail;
use crate::common::{env, ADAPTER};

/// The protocol's validity window; the harness may shorten it for a run with
/// a dated protocol note, through the ORCA_VALIDITY_MS environment variable.
const DEFAULT_VALIDITY_MS: u64 = 10 * 60 * 1000;
/// The step between status checks that found nothing definitive, growing
/// with each check up to MAX_RETRY_MS.
const RETRY_MS: u64 = 2000;
const MAX_RETRY_MS: u64 = 5 * 60 * 1000;

/// What the approval service hands the adapter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delivery {
    pub run: String,
    pub activity: String,
    pub authorization: String,
    pub expires_at_ms: u64,
    pub message: String,
}

#[workflow]
#[derive(Default)]
pub struct DeliveryWorkflow;

#[workflow_methods]
impl DeliveryWorkflow {
    /// Returns what the first delivery attempt found.
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Delivery) -> WorkflowResult<String> {
        // A crashed adapter stops heartbeating; Temporal then retries the
        // attempt within seconds instead of waiting out the whole timeout.
        let deliver = ActivityOptions::with_start_to_close_timeout(Duration::from_secs(3600))
            .heartbeat_timeout(Duration::from_secs(5))
            .build();
        let first = ctx.execute_activity(MailAdapter::deliver, input.clone(), deliver).await?;
        let summary = first.summary();

        let mut state = first;
        let mut checks = 0;
        while let Delivered::Pending { request_id, not_after_ms, retry_soon, .. } = state {
            checks += 1;
            let now = ctx.workflow_time().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).unwrap_or_default().as_millis() as u64;
            // Keep checking until the outcome is definitive, backing off so a
            // long outage costs little; giving up would leave the outcome
            // Unknown and the reservation held for good.
            let at = if retry_soon || now > not_after_ms {
                now + (RETRY_MS * checks).min(MAX_RETRY_MS)
            } else {
                not_after_ms + RETRY_MS
            };
            ctx.timer(Duration::from_millis(at.saturating_sub(now))).await;

            let input = (input.run.clone(), request_id, not_after_ms, input.message.clone());
            state = ctx.execute_activity(MailAdapter::reconcile, input, ActivityOptions::start_to_close_timeout(Duration::from_secs(60))).await?;
        }
        Ok(summary)
    }
}

pub struct MailAdapter {
    pub credential: String,
    pub http: reqwest::Client,
}

impl MailAdapter {
    fn contract(&self, run: &str) -> MockMail {
        MockMail {
            run: run.into(),
            credential: self.credential.clone(),
            validity_ms: env("ORCA_VALIDITY_MS", "").parse().unwrap_or(DEFAULT_VALIDITY_MS),
            http: self.http.clone(),
        }
    }
}

/// Heartbeats every second while `work` runs.
async fn heartbeating<T>(ctx: &ActivityContext, work: impl Future<Output = T>) -> T {
    tokio::pin!(work);
    loop {
        tokio::select! {
            out = &mut work => return out,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                let _ = ctx.record_heartbeat("alive".to_string()).await;
            }
        }
    }
}

#[activities]
impl MailAdapter {
    #[activity]
    pub async fn deliver(self: Arc<Self>, ctx: ActivityContext, d: Delivery) -> Result<Delivered, ActivityError> {
        // The workflow and activity identifiers survive Temporal's retries, so
        // a retried attempt is the same delivery and the journal tells it so.
        let delivery = format!("{}/{}", ctx.info().workflow_id.clone().unwrap_or_default(), ctx.info().activity_id);
        let contract = self.contract(&d.run);
        let adapter = Adapter { run: &d.run, name: ADAPTER, contract: &contract };

        let work = adapter.deliver(&d.activity, &d.authorization, d.expires_at_ms, &d.message, &delivery);
        Ok(heartbeating(&ctx, work).await?)
    }

    /// A scheduled status check, under the standing reconciliation grant:
    /// status checks and receipt updates only, never a re-dispatch.
    #[activity]
    pub async fn reconcile(self: Arc<Self>, _ctx: ActivityContext, input: (String, String, u64, String)) -> Result<Delivered, ActivityError> {
        let (run, request_id, not_after_ms, payload) = input;
        let contract = self.contract(&run);
        let adapter = Adapter { run: &run, name: ADAPTER, contract: &contract };

        Ok(adapter.reconcile(&request_id, not_after_ms, None, &payload).await?)
    }
}
