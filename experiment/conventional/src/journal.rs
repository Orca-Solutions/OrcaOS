// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The journal writer hosted in PostgreSQL. Each command runs in one
//! transaction that locks its run's row, applies the shared reference
//! implementation's step, stores the result and commits before the caller
//! sees the response. So the row lock is the single point of order, and "the
//! record commits" means the PostgreSQL commit.
//!
//! The shared budget is a reservation table taken under the same lock, so a
//! reservation and its refusal are atomic with the journal.
//!
//! Scorecard: counted. Category: grants (hosting the journal, budget
//! reservation, the live-grant check). Tests: all; the budget serves test 6.

use orca_ref::{Command, Journal as Writer, Kind, Response, Source};
use serde_json::{json, Value};
use tokio_postgres::types::Json;
use tokio_postgres::Transaction;

use crate::common::{db, now_ms};

/// Locks the run's journal row until the transaction ends. A service that
/// must commit its own rows with a journal command takes the lock itself.
pub async fn lock(tx: &Transaction<'_>, run: &str) -> anyhow::Result<Writer> {
    tx.execute(
        "INSERT INTO journals (run, writer) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[&run, &Json(Writer::new())],
    )
    .await?;
    let row = tx.query_one("SELECT writer FROM journals WHERE run = $1 FOR UPDATE", &[&run]).await?;

    Ok(row.get::<_, Json<Writer>>(0).0)
}

pub async fn store(tx: &Transaction<'_>, run: &str, writer: &Writer) -> anyhow::Result<()> {
    tx.execute("UPDATE journals SET writer = $2 WHERE run = $1", &[&run, &Json(writer)]).await?;
    Ok(())
}

/// Reads the journal without taking the lock.
pub async fn read(run: &str) -> anyhow::Result<Writer> {
    let row = db().await?.query_opt("SELECT writer FROM journals WHERE run = $1", &[&run]).await?;
    Ok(row.map(|r| r.get::<_, Json<Writer>>(0).0).unwrap_or_default())
}

/// Applies one command and commits it before returning the response.
pub async fn apply(run: &str, source: &Source, command: Command) -> anyhow::Result<Response> {
    let mut client = db().await?;
    let tx = client.transaction().await?;
    let mut writer = lock(&tx, run).await?;

    let before = writer.entries.len();
    let response = writer.apply(source.clone(), now_ms(), command);

    // Reservations the journal released (a certified non-start) are released
    // in the budget too.
    for entry in &writer.entries[before..] {
        if let Kind::ReservationReleased { reservation, .. } = &entry.kind {
            tx.execute(
                "UPDATE reservations SET released = true WHERE run = $1 AND reservation = $2",
                &[&run, reservation],
            )
            .await?;
        }
    }

    store(&tx, run, &writer).await?;
    tx.commit().await?;
    Ok(response)
}

pub async fn set_budget(run: &str, root: &str, cpu_ms: u64) -> anyhow::Result<()> {
    db().await?
        .execute(
            "INSERT INTO budgets (run, root, cpu_ms) VALUES ($1, $2, $3) ON CONFLICT (run, root) DO UPDATE SET cpu_ms = $3",
            &[&run, &root, &(cpu_ms as i64)],
        )
        .await?;
    Ok(())
}

fn root(writer: &Writer, activity: &str) -> String {
    let mut current = activity.to_string();
    while let Some(parent) = writer.activities.get(&current).and_then(|a| a.parent.clone()) {
        current = parent;
    }
    current
}

/// Reserves from the root activity's allowance, atomically. A reservation
/// identifier is never reserved twice; repeating it returns the original
/// result.
pub async fn reserve(run: &str, activity: &str, reservation: &str, cpu_ms: u64) -> anyhow::Result<String> {
    let mut client = db().await?;
    let tx = client.transaction().await?;
    let mut writer = lock(&tx, run).await?;

    let earlier = tx
        .query_opt("SELECT result FROM reservations WHERE run = $1 AND reservation = $2", &[&run, &reservation])
        .await?;
    if let Some(row) = earlier {
        return Ok(row.get(0));
    }

    let now = now_ms();
    let root = root(&writer, activity);
    let allowance: i64 = tx
        .query_opt("SELECT cpu_ms FROM budgets WHERE run = $1 AND root = $2", &[&run, &root])
        .await?
        .map(|r| r.get(0))
        .unwrap_or(0);
    let used: i64 = tx
        .query_one(
            "SELECT COALESCE(SUM(cpu_ms), 0)::bigint FROM reservations
             WHERE run = $1 AND root = $2 AND result = 'reserved' AND NOT released",
            &[&run, &root],
        )
        .await?
        .get(0);

    let refusal = if !writer.activity_open(activity) {
        Some(format!("{activity} is not open"))
    } else if used + cpu_ms as i64 > allowance {
        Some(format!("{root} has {} of {allowance} CPU-ms left", allowance - used))
    } else {
        None
    };

    let result = match refusal {
        Some(reason) => {
            let denial = Command::Denial {
                holder: activity.into(),
                operation: "reserve".into(),
                object: format!("{root} budget, {cpu_ms} CPU-ms"),
                reason: reason.clone(),
            };
            writer.apply(Source::service("budget"), now, denial);
            store(&tx, run, &writer).await?;
            format!("refused: {reason}")
        }
        None => "reserved".to_string(),
    };

    tx.execute(
        "INSERT INTO reservations (run, reservation, activity, root, cpu_ms, result, at_ms) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[&run, &reservation, &activity, &root, &(cpu_ms as i64), &result, &(now as i64)],
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

/// The holder's grants that are live right now, in an open activity.
pub fn live_grants<'w>(writer: &'w Writer, holder: &str) -> Vec<&'w orca_ref::journal::Grant> {
    let now = now_ms();
    writer
        .grants
        .values()
        .filter(|g| g.holder == holder && writer.grant_live(&g.id, now) && writer.activity_open(&g.activity))
        .collect()
}

/// Whether the holder has a live grant for the capability on the object.
pub async fn live(run: &str, holder: &str, capability: &str, object: &str) -> anyhow::Result<bool> {
    let writer = read(run).await?;
    Ok(live_grants(&writer, holder).iter().any(|g| g.capability == capability && g.object == object))
}

/// The journal and the budget, in the shape the inspector reads.
pub async fn export(run: &str) -> anyhow::Result<Value> {
    let writer = read(run).await?;
    let client = db().await?;

    let budgets: serde_json::Map<String, Value> = client
        .query("SELECT root, cpu_ms FROM budgets WHERE run = $1", &[&run])
        .await?
        .iter()
        .map(|r| (r.get::<_, String>(0), json!(r.get::<_, i64>(1))))
        .collect();
    let rows = client
        .query("SELECT reservation, activity, root, cpu_ms, result, released, at_ms FROM reservations WHERE run = $1 ORDER BY at_ms", &[&run])
        .await?;

    let mut reservations = serde_json::Map::new();
    let mut log = Vec::new();
    for r in &rows {
        let (id, result): (String, String) = (r.get(0), r.get(4));
        let (activity, root, cpu_ms): (String, String, i64) = (r.get(1), r.get(2), r.get(3));
        if result == "reserved" {
            reservations.insert(id.clone(), json!({ "activity": activity, "root": root, "cpu_ms": cpu_ms, "released": r.get::<_, bool>(5) }));
        }
        log.push(json!({ "at_ms": r.get::<_, i64>(6), "reservation": id, "activity": activity, "cpu_ms": cpu_ms, "result": result }));
    }

    Ok(json!({ "journal": writer, "ledger": { "budgets": budgets, "reservations": reservations, "log": log } }))
}
