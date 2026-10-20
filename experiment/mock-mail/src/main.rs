// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The mock mail service for the Scenario A experiment.
//!
//! The service port speaks the mail contract: `POST /submit` and
//! `GET /status/{request_id}?not_after_ms=`. The control port belongs to the
//! harness: logs, fault switches, and the checkpoints that hold the adapter at
//! its crash and pause points. Participants reach neither.

mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use state::{CheckpointRule, Fault, State, Submit, SubmitOutcome};

#[derive(Clone)]
struct Shared {
    state: Arc<Mutex<State>>,
    released: Arc<Notify>,
}

/// Returned by a handler to abort the connection without a reply.
#[derive(Debug)]
struct Abort;

impl std::fmt::Display for Abort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("connection aborted by fault switch")
    }
}

impl std::error::Error for Abort {}

type Reply = Result<Response<Full<Bytes>>, Abort>;

fn reply(status: StatusCode, body: Value) -> Reply {
    Ok(Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap())
}

fn query(req: &Request<Incoming>, name: &str) -> Option<String> {
    req.uri().query()?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

async fn body_json<T: serde::de::DeserializeOwned>(req: Request<Incoming>) -> Result<T, String> {
    let bytes = req.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// Test runs share one mock service. The harness's run identifier, sent by
/// the adapter in `x-orca-run`, keeps their request identifiers apart.
fn run_of(req: &Request<Incoming>) -> String {
    req.headers().get("x-orca-run").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string()
}

fn namespaced(run: &str, request_id: &str) -> String {
    if run.is_empty() { request_id.to_string() } else { format!("{run}/{request_id}") }
}

async fn service(shared: Shared, req: Request<Incoming>) -> Reply {
    let path = req.uri().path().to_string();

    match (req.method().clone(), path.as_str()) {
        (Method::POST, "/submit") => {
            let token = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.trim_start_matches("Bearer ").to_string());
            let run = run_of(&req);
            let transport_key = req.headers().get("idempotency-key").and_then(|v| v.to_str().ok()).map(str::to_string);
            let mut submit: Submit = match body_json(req).await {
                Ok(submit) => submit,
                Err(e) => return reply(StatusCode::BAD_REQUEST, json!({ "error": e })),
            };
            submit.request_id = namespaced(&run, &submit.request_id);

            let outcome = {
                let mut state = shared.state.lock().unwrap();
                let authorized = token.as_deref() == Some(state.credential.as_str());
                state.submit(authorized, submit, transport_key)
            };

            match outcome {
                SubmitOutcome::DropRequest | SubmitOutcome::DropReply => Err(Abort),
                SubmitOutcome::Unauthenticated => reply(StatusCode::UNAUTHORIZED, json!({ "error": "bad credential" })),
                SubmitOutcome::Reply { reply: r, delay_ms } => {
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    reply(StatusCode::OK, serde_json::to_value(r).unwrap())
                }
            }
        }
        (Method::GET, p) if p.starts_with("/status/") => {
            let rid = namespaced(&run_of(&req), &p.trim_start_matches("/status/").replace("%3A", ":"));
            let not_after_ms = query(&req, "not_after_ms").and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
            let status = shared.state.lock().unwrap().status(&rid, not_after_ms);

            match status {
                Some(status) => reply(StatusCode::OK, status),
                None => reply(StatusCode::SERVICE_UNAVAILABLE, json!({ "error": "status endpoint offline" })),
            }
        }
        _ => reply(StatusCode::NOT_FOUND, json!({ "error": "no such endpoint" })),
    }
}

async fn control(shared: Shared, req: Request<Incoming>) -> Reply {
    let path = req.uri().path().to_string();

    match (req.method().clone(), path.as_str()) {
        (Method::GET, "/control/logs") => {
            let state = shared.state.lock().unwrap();
            reply(StatusCode::OK, json!(state.logs.entries))
        }
        (Method::POST, "/control/reset") => {
            shared.state.lock().unwrap().reset();
            shared.released.notify_waiters();
            reply(StatusCode::OK, json!({ "reset": true }))
        }
        (Method::POST, "/control/fault") => match body_json::<Fault>(req).await {
            Ok(fault) => {
                shared.state.lock().unwrap().add_fault(fault);
                reply(StatusCode::OK, json!({ "armed": true }))
            }
            Err(e) => reply(StatusCode::BAD_REQUEST, json!({ "error": e })),
        },
        (Method::POST, "/control/clear-faults") => {
            let mut state = shared.state.lock().unwrap();
            state.clear_faults();
            state.clear_checkpoint_rules();
            reply(StatusCode::OK, json!({ "cleared": true }))
        }
        (Method::POST, "/control/checkpoint-rule") => match body_json::<CheckpointRule>(req).await {
            Ok(rule) => {
                shared.state.lock().unwrap().add_checkpoint_rule(rule);
                reply(StatusCode::OK, json!({ "armed": true }))
            }
            Err(e) => reply(StatusCode::BAD_REQUEST, json!({ "error": e })),
        },
        (Method::GET, "/control/held") => {
            let state = shared.state.lock().unwrap();
            let held: Vec<_> = state.held.values().filter(|h| !state.released.contains(&h.id)).cloned().collect();
            reply(StatusCode::OK, json!(held))
        }
        (Method::POST, "/control/release") => {
            let id = query(&req, "id").and_then(|v| v.parse().ok());
            let mut state = shared.state.lock().unwrap();
            let ids: Vec<u64> = match id {
                Some(id) => vec![id],
                None => state.held.keys().copied().collect(),
            };
            state.released.extend(ids.iter().copied());
            shared.released.notify_waiters();
            reply(StatusCode::OK, json!({ "released": ids }))
        }
        // Called by the adapter at each crash and pause point.
        (Method::POST, "/control/checkpoint") => {
            let point = query(&req, "point").unwrap_or_default();
            let rid = query(&req, "request_id").unwrap_or_default().replace("%3A", ":");
            let held = shared.state.lock().unwrap().checkpoint(&point, &rid);

            if let Some(id) = held {
                loop {
                    let notified = shared.released.notified();
                    if shared.state.lock().unwrap().released.contains(&id) {
                        break;
                    }
                    notified.await;
                }
            }
            reply(StatusCode::OK, json!({ "point": point, "held": held.is_some() }))
        }
        _ => reply(StatusCode::NOT_FOUND, json!({ "error": "no such endpoint" })),
    }
}

async fn serve<F, Fut>(addr: SocketAddr, shared: Shared, handler: F)
where
    F: Fn(Shared, Request<Incoming>) -> Fut + Copy + Send + 'static,
    Fut: std::future::Future<Output = Reply> + Send + 'static,
{
    let listener = TcpListener::bind(addr).await.expect("bind");

    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let shared = shared.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req| handler(shared.clone(), req));
            // An aborted request ends its connection without a reply.
            let _ = http1::Builder::new().serve_connection(TokioIo::new(stream), service).await;
        });
    }
}

fn arg(name: &str, default: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| default.to_string())
}

#[tokio::main]
async fn main() {
    let service_addr: SocketAddr = arg("--service", "127.0.0.1:18081").parse().expect("service address");
    let control_addr: SocketAddr = arg("--control", "127.0.0.1:18082").parse().expect("control address");
    let log_dir = PathBuf::from(arg("--log-dir", "mock-mail-logs"));
    let credential = std::env::var("MOCK_MAIL_CREDENTIAL").expect("MOCK_MAIL_CREDENTIAL must be set");

    let shared = Shared {
        state: Arc::new(Mutex::new(State::new(credential, Some(&log_dir)))),
        released: Arc::new(Notify::new()),
    };

    println!("mock-mail service on {service_addr}, control on {control_addr}, logs in {}", log_dir.display());
    let control_task = tokio::spawn(serve(control_addr, shared.clone(), control));
    serve(service_addr, shared, service).await;
    let _ = control_task.await;
}
