// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The approval service: holds each proposal, shows the principal exactly
//! what will be sent with its payload hash, records the approval against that
//! hash, mints a single-use authorization and hands it straight to the mail
//! adapter by starting a delivery workflow on the adapter's task queue. The
//! drafter never sees the authorization.
//!
//! Each request's journal commands commit in one transaction with this
//! service's own rows, under the run's journal lock, so a crash keeps all of
//! a step or none of it. A minted authorization goes into the delivery outbox
//! in that same transaction; its delivery workflow is started after the
//! commit, and again by the outbox loop if that start did not happen.
//!
//!   POST /submit   {run, activity, holder, message, submission?} -> proposal id
//!   GET  /display?run=&proposal=                      -> what the principal sees
//!   POST /approve  {run, proposal, payload_hash, grant} -> the authorization
//!
//! Scorecard: counted. Category: approvals. Tests: 2, 3, 7.

use std::time::Duration;

use axum::extract::{Json, Query};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use orca_ref::{payload_hash, Command, Journal as Writer, Message, Response, Source};
use serde::Deserialize;
use serde_json::{json, Value};
use temporalio_client::errors::WorkflowStartError;
use temporalio_client::{WorkflowIdReusePolicy, WorkflowStartOptions};
use tokio_postgres::Transaction;

use crate::adapter::{Delivery, DeliveryWorkflow};
use crate::common::{db, env, now_ms, temporal, ADAPTER, ADAPTER_QUEUE, PRINCIPAL};
use crate::journal;

/// How long a minted authorization stays usable.
const AUTHORIZATION_TTL_MS: u64 = 60 * 60 * 1000;

type Failed = (StatusCode, String);
type Reply = Result<String, Failed>;

fn failed(e: impl std::fmt::Display) -> Failed {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn service() -> Source {
    Source::service("approval-path")
}

async fn commit(tx: Transaction<'_>, run: &str, writer: &Writer) -> Result<(), Failed> {
    journal::store(&tx, run, writer).await.map_err(failed)?;
    tx.commit().await.map_err(failed)
}

#[derive(Deserialize)]
struct Submit {
    run: String,
    activity: String,
    holder: String,
    message: String,
    /// Names one submission, so a retry of it is not a second proposal.
    submission: Option<String>,
}

async fn submit(Json(s): Json<Submit>) -> Reply {
    let parsed: Message = match serde_json::from_str(&s.message) {
        Ok(m) => m,
        Err(e) => return Ok(format!("refused: malformed message: {e}")),
    };
    let mut client = db().await.map_err(failed)?;
    let tx = client.transaction().await.map_err(failed)?;
    let mut writer = journal::lock(&tx, &s.run).await.map_err(failed)?;

    let earlier = tx
        .query_opt("SELECT proposal_id FROM proposals WHERE run = $1 AND submission = $2", &[&s.run, &s.submission])
        .await
        .map_err(failed)?;
    if let Some(row) = earlier {
        return Ok(row.get(0));
    }

    // Checked under the lock, so a revocation cannot commit in between.
    let live = journal::live_grants(&writer, &s.holder).iter().any(|g| g.capability == "submit-proposal" && g.object == "approval-path");
    if !live {
        let reason = format!("{} holds no live submit-proposal grant", s.holder);
        let denial = Command::Denial { holder: s.holder, operation: "submit proposal".into(), object: "approval-path".into(), reason: reason.clone() };
        writer.apply(service(), now_ms(), denial);
        commit(tx, &s.run, &writer).await?;
        return Ok(format!("refused: {reason}"));
    }

    let count: i64 = tx.query_one("SELECT count(*) FROM proposals WHERE run = $1", &[&s.run]).await.map_err(failed)?.get(0);
    let proposal_id = format!("prop-{}", count + 1);
    let hash = payload_hash(&parsed);
    let command = Command::Proposal {
        proposal_id: proposal_id.clone(),
        activity: s.activity.clone(),
        holder: s.holder,
        payload_hash: hash.clone(),
    };
    if let Response::Refused { reason, .. } = writer.apply(service(), now_ms(), command) {
        commit(tx, &s.run, &writer).await?;
        return Ok(format!("refused: {reason}"));
    }

    tx.execute(
        "INSERT INTO proposals (run, proposal_id, activity, message, payload_hash, submission) VALUES ($1, $2, $3, $4, $5, $6)",
        &[&s.run, &proposal_id, &s.activity, &s.message, &hash, &s.submission],
    )
    .await
    .map_err(failed)?;
    commit(tx, &s.run, &writer).await?;
    Ok(proposal_id)
}

#[derive(Deserialize)]
struct Shown {
    run: String,
    proposal: String,
}

async fn display(Query(q): Query<Shown>) -> Result<Json<Value>, Failed> {
    let row = db()
        .await
        .map_err(failed)?
        .query_opt(
            "SELECT activity, message, payload_hash FROM proposals WHERE run = $1 AND proposal_id = $2",
            &[&q.run, &q.proposal],
        )
        .await
        .map_err(failed)?;

    let shown = match row {
        Some(r) => json!({
            "proposal_id": q.proposal,
            "activity": r.get::<_, String>(0),
            "message": serde_json::from_str::<Value>(r.get(1)).unwrap_or_default(),
            "payload_hash": r.get::<_, String>(2),
        }),
        None => json!({ "error": "no such proposal" }),
    };
    Ok(Json(shown))
}

#[derive(Deserialize)]
struct Approve {
    run: String,
    proposal: String,
    payload_hash: String,
    grant: String,
}

/// The principal approves what they were shown, by its hash, resting on the
/// named send grant. The approval, the mint and the outbox row commit together.
async fn approve(Json(a): Json<Approve>) -> Reply {
    let mut client = db().await.map_err(failed)?;
    let tx = client.transaction().await.map_err(failed)?;
    let mut writer = journal::lock(&tx, &a.run).await.map_err(failed)?;

    let row = tx
        .query_opt(
            "SELECT activity, message, payload_hash FROM proposals WHERE run = $1 AND proposal_id = $2",
            &[&a.run, &a.proposal],
        )
        .await
        .map_err(failed)?;
    let Some(row) = row else {
        return Ok("refused: no such proposal".into());
    };
    let (activity, message, stored_hash): (String, String, String) = (row.get(0), row.get(1), row.get(2));
    let now = now_ms();

    // The principal approves what was displayed; a changed payload is refused.
    let command = Command::Approval { proposal_id: a.proposal.clone(), payload_hash: a.payload_hash };
    if let Response::Refused { reason, .. } = writer.apply(Source::principal(PRINCIPAL), now, command) {
        commit(tx, &a.run, &writer).await?;
        return Ok(format!("refused: {reason}"));
    }

    let recipient = serde_json::from_str::<Message>(&message).map(|m| m.to).unwrap_or_default();
    let expires_at_ms = now + AUTHORIZATION_TTL_MS;
    let command = Command::MintAuthorization {
        activity: activity.clone(),
        operation: "send".into(),
        target: recipient,
        adapter: ADAPTER.into(),
        payload_hash: stored_hash,
        expires_at_ms,
        grant_id: a.grant,
        proposal_id: Some(a.proposal),
    };
    let authorization = match writer.apply(service(), now, command) {
        Response::Minted { authorization, .. } => authorization,
        Response::Refused { reason, .. } => {
            commit(tx, &a.run, &writer).await?;
            return Ok(format!("refused: {reason}"));
        }
        other => return Ok(format!("unexpected: {other:?}")),
    };

    // Straight to the adapter; no participant sees it.
    let delivery = Delivery { run: a.run.clone(), activity, authorization: authorization.clone(), expires_at_ms, message };
    tx.execute(
        "INSERT INTO deliveries (run, authorization_id, delivery) VALUES ($1, $2, $3)",
        &[&a.run, &authorization, &tokio_postgres::types::Json(&delivery)],
    )
    .await
    .map_err(failed)?;
    commit(tx, &a.run, &writer).await?;

    // If this start fails, the outbox loop starts it.
    let _ = start(&delivery).await;
    Ok(json!({ "authorization": authorization, "expires_at_ms": expires_at_ms }).to_string())
}

/// Starts one authorization's delivery workflow and marks it started. The
/// workflow identifier is never reused, so a second start finds the first.
async fn start(delivery: &Delivery) -> anyhow::Result<()> {
    let id = format!("{}/deliver/{}", delivery.run, delivery.authorization);
    let options = WorkflowStartOptions::new(ADAPTER_QUEUE, id).id_reuse_policy(WorkflowIdReusePolicy::RejectDuplicate).build();

    match temporal().await?.start_workflow(DeliveryWorkflow::run, delivery.clone(), options).await {
        Ok(_) | Err(WorkflowStartError::AlreadyStarted { .. }) => {}
        Err(e) => return Err(e.into()),
    }
    db().await?
        .execute(
            "UPDATE deliveries SET started = true WHERE run = $1 AND authorization_id = $2",
            &[&delivery.run, &delivery.authorization],
        )
        .await?;
    Ok(())
}

/// Every two seconds, starts any minted authorization whose delivery
/// workflow was not started, such as after a crash just past the commit.
async fn outbox() {
    loop {
        if let Ok(client) = db().await {
            let rows = client.query("SELECT delivery FROM deliveries WHERE NOT started", &[]).await.unwrap_or_default();
            for row in rows {
                let delivery: tokio_postgres::types::Json<Delivery> = row.get(0);
                let _ = start(&delivery.0).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub async fn serve() -> anyhow::Result<()> {
    tokio::spawn(outbox());
    let app = Router::new()
        .route("/submit", post(submit))
        .route("/display", get(display))
        .route("/approve", post(approve));
    let listener = tokio::net::TcpListener::bind(env("ORCA_APPROVAL_BIND", "127.0.0.1:18090")).await?;

    axum::serve(listener, app).await?;
    Ok(())
}
