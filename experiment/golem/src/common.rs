// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Helpers every agent in the component uses: the clock, calls to the journal
//! writer, and access to Golem's key-value stores.
//!
//! Scorecard: counted. Category: grants (journal client) and none (store
//! helpers are task I/O). Tests: all.

use golem_rust::bindings::wasi::keyvalue::{eventual, types::{Bucket, OutgoingValue}};
use orca_ref::{Command, Response, Source};

use crate::journal_agent::JournalClient;

pub const PRINCIPAL: &str = "principal-1";
pub const ADAPTER: &str = "mail-adapter";

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Applies one command through the run's journal writer and waits until it
/// has committed.
pub async fn journal(run: &str, source: &Source, command: Command) -> Response {
    let source = serde_json::to_string(source).unwrap();
    let command = serde_json::to_string(&command).unwrap();
    let response = JournalClient::get(run.to_string()).apply(source, command).await;

    serde_json::from_str(&response).expect("journal response")
}

/// Golem's key-value store. Every call is checked against the caller's cards;
/// a denial comes back as an error string carrying Golem's own message.
pub mod store {
    use super::*;

    pub fn get(bucket: &str, key: &str) -> Result<Option<String>, String> {
        let bucket = Bucket::open_bucket(bucket).map_err(|e| e.trace())?;
        let value = eventual::get(&bucket, &key.to_string()).map_err(|e| e.trace())?;

        Ok(value.map(|v| String::from_utf8_lossy(&v.incoming_value_consume_sync().unwrap()).into_owned()))
    }

    pub fn set(bucket: &str, key: &str, value: &str) -> Result<(), String> {
        let bucket = Bucket::open_bucket(bucket).map_err(|e| e.trace())?;
        let outgoing = OutgoingValue::new_outgoing_value();
        outgoing.outgoing_value_write_body_sync(value.as_bytes()).map_err(|_| "write body".to_string())?;

        eventual::set(&bucket, &key.to_string(), &outgoing).map_err(|e| e.trace())
    }
}
