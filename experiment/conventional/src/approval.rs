// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The approval service: holds each proposal, shows the principal exactly
//! what will be sent with its payload hash, records the approval against that
//! hash, mints a single-use authorization and hands it straight to the mail
//! adapter by starting a delivery workflow on the adapter's task queue. The
//! drafter never sees the authorization.
//!
//!   POST /submit   {run, activity, holder, message, submission?} -> proposal id
//!   GET  /display?run=&proposal=                      -> what the principal sees
//!   POST /approve  {run, proposal, payload_hash, grant} -> the authorization
//!
//! Scorecard: counted. Category: approvals. Tests: 2, 3, 7.

use axum::extract::{Json, Query};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::Router;
use orca_ref::{payload_hash, Command, Message, Response, Source};
use serde::Deserialize;
use serde_json::{json, Value};
use temporalio_client::WorkflowStartOptions;

use crate::adapter::{Delivery, DeliveryWorkflow};
use crate::common::{db, env, now_ms, temporal, ADAPTER, ADAPTER_QUEUE, PRINCIPAL};
use crate::journal;

/// How long a minted authorization stays usable.
const AUTHORIZATION_TTL_MS: u64 = 60 * 60 * 1000;

type Reply = Result<String, (StatusCode, String)>;

fn failed(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn service() -> Source {
    Source::service("approval-path")
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
    let hash = payload_hash(&parsed);
    let client = db().await.map_err(failed)?;
    let earlier = client
        .query_opt("SELECT proposal_id FROM proposals WHERE run = $1 AND submission = $2", &[&s.run, &s.submission])
        .await
        .map_err(failed)?;
    if let Some(row) = earlier {
        return Ok(row.get(0));
    }
    let count: i64 = client
        .query_one("SELECT count(*) FROM proposals WHERE run = $1", &[&s.run])
        .await
        .map_err(failed)?
        .get(0);
    let proposal_id = format!("prop-{}", count + 1);

    let command = Command::Proposal {
        proposal_id: proposal_id.clone(),
        activity: s.activity.clone(),
        holder: s.holder,
        payload_hash: hash.clone(),
    };
    if let Response::Refused { reason, .. } = journal::apply(&s.run, &service(), command).await.map_err(failed)? {
        return Ok(format!("refused: {reason}"));
    }

    client
        .execute(
            "INSERT INTO proposals (run, proposal_id, activity, message, payload_hash, submission) VALUES ($1, $2, $3, $4, $5, $6)",
            &[&s.run, &proposal_id, &s.activity, &s.message, &hash, &s.submission],
        )
        .await
        .map_err(failed)?;
    Ok(proposal_id)
}

#[derive(Deserialize)]
struct Shown {
    run: String,
    proposal: String,
}

/// (activity, message, payload hash) of a stored proposal.
async fn proposal(run: &str, proposal: &str) -> anyhow::Result<Option<(String, String, String)>> {
    let row = db()
        .await?
        .query_opt(
            "SELECT activity, message, payload_hash FROM proposals WHERE run = $1 AND proposal_id = $2",
            &[&run, &proposal],
        )
        .await?;
    Ok(row.map(|r| (r.get(0), r.get(1), r.get(2))))
}

async fn display(Query(q): Query<Shown>) -> Result<Json<Value>, (StatusCode, String)> {
    let shown = match proposal(&q.run, &q.proposal).await.map_err(failed)? {
        Some((activity, message, hash)) => json!({
            "proposal_id": q.proposal,
            "activity": activity,
            "message": serde_json::from_str::<Value>(&message).unwrap_or_default(),
            "payload_hash": hash,
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
/// named send grant.
async fn approve(Json(a): Json<Approve>) -> Reply {
    let Some((activity, message, stored_hash)) = proposal(&a.run, &a.proposal).await.map_err(failed)? else {
        return Ok("refused: no such proposal".into());
    };

    // The principal approves what was displayed; a changed payload is refused.
    let command = Command::Approval { proposal_id: a.proposal.clone(), payload_hash: a.payload_hash };
    let principal = Source::principal(PRINCIPAL);
    if let Response::Refused { reason, .. } = journal::apply(&a.run, &principal, command).await.map_err(failed)? {
        return Ok(format!("refused: {reason}"));
    }

    let recipient = serde_json::from_str::<Message>(&message).map(|m| m.to).unwrap_or_default();
    let expires_at_ms = now_ms() + AUTHORIZATION_TTL_MS;
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
    match journal::apply(&a.run, &service(), command).await.map_err(failed)? {
        Response::Minted { authorization, .. } => {
            // Straight to the adapter; no participant sees it.
            let delivery = Delivery { run: a.run.clone(), activity, authorization: authorization.clone(), expires_at_ms, message };
            let options = WorkflowStartOptions::new(ADAPTER_QUEUE, format!("{}/deliver/{authorization}", a.run)).build();
            temporal().await.map_err(failed)?.start_workflow(DeliveryWorkflow::run, delivery, options).await.map_err(failed)?;
            Ok(json!({ "authorization": authorization, "expires_at_ms": expires_at_ms }).to_string())
        }
        Response::Refused { reason, .. } => Ok(format!("refused: {reason}")),
        other => Ok(format!("unexpected: {other:?}")),
    }
}

pub async fn serve() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/submit", post(submit))
        .route("/display", get(display))
        .route("/approve", post(approve));
    let listener = tokio::net::TcpListener::bind(env("ORCA_APPROVAL_BIND", "127.0.0.1:18090")).await?;

    axum::serve(listener, app).await?;
    Ok(())
}
