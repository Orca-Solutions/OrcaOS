// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Imports the store's own denial records on the conventional build. Wasmtime
//! traces every WASI call a participant makes and what it returned; a file
//! the container does not mount, or mounts read-only, comes back as an error
//! from `open-at`. The inspector journals each one with the container's file
//! system as its source, as it does Golem's oplog denials.
//!
//! The trace shares the container's stderr with the participant, so a
//! participant could print lines that look like it. It cannot remove
//! Wasmtime's own record of it opening stderr, so a trace in which the
//! participant opened stderr is not trusted and yields no denials.
//!
//! Scorecard: counted. Category: inspection (conventional build only).
//! Tests: 1.

/// Refused opens in a Wasmtime trace: (operation, object, reason) each.
pub fn denials(trace: &str) -> Vec<(String, String, String)> {
    if trace.lines().any(|l| l.contains("function=\"get-stderr\"")) {
        return Vec::new();
    }
    let mut denials = Vec::new();
    let mut open: Option<(String, String)> = None;
    let mut cause = String::new();

    for line in trace.lines().filter(|l| l.contains("function=\"[method]descriptor.open-at\"")) {
        if let Some(call) = line.split(": call ").nth(1) {
            let field = |name: &str| call.split(&format!(" {name}=")).nth(1).and_then(|rest| rest.split(' ').next()).unwrap_or_default();
            let operation = if field("flags").contains("WRITE") { "write" } else { "read" };
            open = Some((operation.to_string(), field("path").trim_matches('"').to_string()));
            cause.clear();
        } else if let Some(os) = line.split("unknown raw os error: ").nth(1) {
            // Wasmtime names the host's error on a line of its own first.
            cause = os.to_string();
        } else if let Some(result) = line.split("return result=Err(").nth(1) {
            if let Some((operation, object)) = open.take() {
                let error = result.strip_suffix(')').unwrap_or(result);
                let reason = if cause.is_empty() { error.to_string() } else { format!("{error}: {cause}") };
                denials.push((operation, object, reason));
            }
        } else if line.contains("return result=Ok") {
            open = None;
        }
    }
    denials
}
