// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What every service in the composition shares: the clock, the connections
//! to PostgreSQL and Temporal, and the names of the task queues.
//!
//! Scorecard: counted. Category: grants (connections the journal and the
//! services use). Tests: all.

use temporalio_client::envconfig::LoadClientConfigProfileOptions;
use temporalio_client::{Client, ClientOptions, Connection};

pub const PRINCIPAL: &str = "principal-1";
pub const ADAPTER: &str = "mail-adapter";
/// The coordinator worker's task queue, and the mail adapter's.
pub const COORDINATOR_QUEUE: &str = "scenario-a";
pub const ADAPTER_QUEUE: &str = "mail-adapter";

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

/// A fresh connection per call, so a restarted PostgreSQL costs one failed
/// call (which Temporal retries) rather than a broken pool.
pub async fn db() -> anyhow::Result<tokio_postgres::Client> {
    let config = env("ORCA_PG", "host=/var/run/postgresql port=5433 user=postgres dbname=orca");
    let (client, connection) = tokio_postgres::connect(&config, tokio_postgres::NoTls).await?;

    tokio::spawn(connection);
    Ok(client)
}

/// The Temporal server, from TEMPORAL_ADDRESS (default localhost:7233).
pub async fn temporal() -> anyhow::Result<Client> {
    let (connection, options) = ClientOptions::load_from_config(LoadClientConfigProfileOptions::default())
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(Client::new(Connection::connect(connection).await?, options)?)
}
