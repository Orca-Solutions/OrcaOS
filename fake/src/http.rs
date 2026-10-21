// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The HTTP binding: `POST /<surface>/v<major.minor>/<call>` with a JSON
//! body, answered one request at a time. It listens on the loopback address
//! only, because the fake trusts its host and checks no identity: whoever
//! can reach the port can claim to be any member.

use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::fake::{CallError, Fake};
use crate::fixture;
use crate::model::SURFACES;

pub fn serve(fake: &mut Fake, port: u16) -> Result<(), String> {
    let address = format!("127.0.0.1:{port}");
    let server = Server::http(&address).map_err(|e| format!("could not listen on {address}: {e}"))?;
    eprintln!("fake Diamond Dog listening on http://{address} (a test double: nothing here is isolated or durable)");

    for mut request in server.incoming_requests() {
        let (status, body) = handle(fake, &mut request);
        let header = Header::from_bytes("content-type", "application/json").unwrap();
        let response = Response::from_string(body.to_string()).with_status_code(status).with_header(header);
        if let Err(e) = request.respond(response) {
            eprintln!("could not answer a request: {e}");
        }
    }
    Ok(())
}

fn handle(fake: &mut Fake, request: &mut Request) -> (u16, Value) {
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
        return (200, json!({ "ok": true, "result": { "surfaces": surfaces, "fake": true } }));
    }

    if let Some(control) = path.strip_prefix("fake/") {
        // Runs a fixture in a fresh fake, leaving this one as it is.
        if control == "check" && method == Method::Post {
            let outcome = fixture::run(&body);
            return (200, json!({ "ok": true, "result": { "name": outcome.name, "passed": outcome.passed(), "failures": outcome.failures, "ledger": outcome.ledger } }));
        }
        return reply(fake.control(control, &body));
    }

    if method != Method::Post {
        return (405, json!({ "ok": false, "error": "surface calls are POST" }));
    }
    match fixture::split_call(&path) {
        Some((surface, version, call)) => reply(fake.call(surface, version, call, &body)),
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
