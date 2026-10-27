// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The HTTP binding (interfaces/diamond-dog/http-binding.md):
//! `POST /<surface>/v<major.minor>/<call>` with a JSON body, answered one
//! request at a time, so the journal writer sees every call in one order.
//!
//! It listens on the loopback address only. Like the fake, this skeleton
//! checks no identity yet: whoever can reach the port can claim to be any
//! member. Without test controls the clock is the wall clock, and scheduled
//! runs, expiries and timeouts happen between requests as they fall due.

use std::time::Duration as StdDuration;

use chrono::Utc;
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::fixture;
use crate::model::SURFACES;
use crate::runtime::{CallError, Runtime};

pub fn serve(runtime: &mut Runtime, port: u16, test_controls: bool) -> Result<(), String> {
    let address = format!("127.0.0.1:{port}");
    let server = Server::http(&address).map_err(|e| format!("could not listen on {address}: {e}"))?;
    let journal = if runtime.core.is_durable() { "journaling to disk" } else { "journal in memory only" };
    let clock = if test_controls { "test controls on, clock moved by fixtures" } else { "wall clock" };
    eprintln!("Diamond Dog (experimental skeleton) listening on http://{address}; {journal}; {clock}. It checks no identity: keep it on this machine.");

    loop {
        if !test_controls {
            follow_wall_clock(runtime);
        }
        let mut request = match server.recv_timeout(StdDuration::from_secs(1)) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => return Err(format!("the listener failed: {e}")),
        };
        if !test_controls {
            follow_wall_clock(runtime);
        }

        let (status, body) = handle(runtime, &mut request, test_controls);
        let header = Header::from_bytes("content-type", "application/json").unwrap();
        let response = Response::from_string(body.to_string()).with_status_code(status).with_header(header);
        if let Err(e) = request.respond(response) {
            eprintln!("could not answer a request: {e}");
        }
    }
}

/// Moves the runtime's clock up to now. A clock that went backwards is not
/// followed: the journal's times only move forward.
fn follow_wall_clock(runtime: &mut Runtime) {
    let now = Utc::now();
    if now > runtime.now() {
        runtime.advance_to(now, false);
    }
}

fn handle(runtime: &mut Runtime, request: &mut Request, test_controls: bool) -> (u16, Value) {
    let mut text = String::new();
    if request.as_reader().read_to_string(&mut text).is_err() {
        return (400, json!({ "ok": false, "error": "the body is not UTF-8" }));
    }
    let body: Value = if text.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => return (400, json!({ "ok": false, "error": format!("the body is not JSON: {e}") })),
        }
    };

    let path = request.url().split('?').next().unwrap_or_default().trim_matches('/').to_string();
    let method = request.method().clone();

    if path == "surfaces" {
        let surfaces: serde_json::Map<String, Value> = SURFACES.iter().map(|(s, v)| (s.to_string(), json!(v))).collect();
        return (200, json!({ "ok": true, "result": { "surfaces": surfaces, "runtime": "diamond-dog", "experimental": true, "fake": false } }));
    }

    if let Some(control) = path.strip_prefix("fake/") {
        if !test_controls {
            return (404, json!({ "ok": false, "error": "test controls are off; start with --test-controls to use them" }));
        }
        // Runs a fixture in a fresh runtime, leaving this one as it is.
        if control == "check" && method == Method::Post {
            let outcome = fixture::run(&body);
            return (200, json!({ "ok": true, "result": { "name": outcome.name, "passed": outcome.passed(), "failures": outcome.failures, "ledger": outcome.ledger } }));
        }
        return reply(runtime.control(control, &body));
    }

    if method != Method::Post {
        return (405, json!({ "ok": false, "error": "surface calls are POST" }));
    }
    match fixture::split_call(&path) {
        Some((surface, version, call)) => reply(runtime.call(surface, version, call, &body)),
        None => (404, json!({ "ok": false, "error": format!("`/{path}` is not /<surface>/v<major.minor>/<call>") })),
    }
}

fn reply(result: Result<Value, CallError>) -> (u16, Value) {
    let status = match &result {
        Ok(_) => 200,
        Err(CallError::Refused(_)) => 403,
        Err(CallError::Bad(_)) => 400,
    };
    (status, fixture::response(&result))
}
