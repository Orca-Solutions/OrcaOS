// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The inspector: answers what happened from the journal, with every answer
//! carrying its source and evidence level, and imports Golem's own denial
//! records from agent oplogs so a store refusal is attributed to the store.
//!
//! Scorecard: counted. Category: inspection. Tests: 1, 3, 4, 6, 7.

use serde_json::{json, Value};

/// Store denials Golem recorded in an agent's oplog: (operation, object,
/// reason) for every key-value call it refused.
pub fn oplog_denials(oplog: &str) -> Vec<(String, String, String)> {
    let mut denials = Vec::new();

    for block in oplog.split("\n#").skip(1) {
        let Some(function) = block.lines().find_map(|l| l.trim().strip_prefix("START keyvalue::")) else { continue };
        let Some(reason) = block.lines().find_map(|l| l.trim().strip_prefix("denied:")) else { continue };
        let input = block.lines().find_map(|l| l.trim().strip_prefix("input:")).unwrap_or_default();

        let quoted = |name: &str| {
            input
                .split(&format!("{name}: \""))
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_default()
                .to_string()
        };
        let operation = match function.rsplit("::").next().unwrap_or_default() {
            "get" | "exists" => "read",
            "set" => "write",
            "delete" => "delete",
            other => other,
        };
        denials.push((operation.to_string(), format!("{}/{}", quoted("bucket"), quoted("key")), reason.trim().to_string()));
    }
    denials
}

fn source_text(source: &Value) -> String {
    match source["kind"].as_str() {
        Some("principal") => format!("principal {}", source["id"].as_str().unwrap_or("?")),
        Some("participant") => format!("participant {} (claim)", source["id"].as_str().unwrap_or("?")),
        Some("service") => format!("service {}", source["name"].as_str().unwrap_or("?")),
        Some("external") => format!(
            "{} via {}",
            source["system"].as_str().unwrap_or("?"),
            source["adapter"].as_str().unwrap_or("?")
        ),
        _ => "unknown source".into(),
    }
}

/// The inspector's answers for one run.
pub fn report(export: &Value) -> Value {
    let journal = &export["journal"];
    let entries = journal["entries"].as_array().cloned().unwrap_or_default();
    let empty = serde_json::Map::new();

    let activities: Value = journal["activities"]
        .as_object()
        .unwrap_or(&empty)
        .iter()
        .map(|(id, a)| (id.clone(), a["state"].clone()))
        .collect::<serde_json::Map<_, _>>()
        .into();

    let outcomes: Vec<Value> = journal["receipts"]
        .as_object()
        .unwrap_or(&empty)
        .values()
        .map(|r| {
            let rid = r["request_id"].as_str().unwrap_or_default();
            let last = entries
                .iter()
                .filter(|e| e["request_id"] == rid && (e["entry"] == "receipt_update" || e["entry"] == "reconciliation"))
                .last();
            json!({
                "request_id": rid,
                "activity": r["activity"],
                "outcome": r["outcome"],
                "evidence": r["evidence"],
                "source": last.map(|e| source_text(&e["source"])).unwrap_or_else(|| "consumption only".into()),
                "submissions": r["submissions"],
                "re_dispatches": r["re_dispatches"],
                "reconciliations": r["reconciliations"],
                "reservation_released": r["reservation_released"],
            })
        })
        .collect();

    let definitive = |outcome: &Value| !matches!(outcome["state"].as_str(), Some("unknown") | Some("submitted"));
    let closed_with_unknown: Vec<Value> = journal["activities"]
        .as_object()
        .unwrap_or(&empty)
        .iter()
        .filter(|(_, a)| a["state"] == "closed")
        .filter_map(|(id, _)| {
            let unknown: Vec<&Value> = outcomes.iter().filter(|o| o["activity"] == id.as_str() && !definitive(&o["outcome"])).collect();
            (!unknown.is_empty()).then(|| json!({ "activity": id, "unknown": unknown.iter().map(|o| o["request_id"].clone()).collect::<Vec<_>>() }))
        })
        .collect();

    // A revocation prevented an effect only if it invalidated an authorization
    // before consumption. One that rests on a revoked grant but was consumed
    // first was not prevented: consumed, then revoked.
    let revocations: Vec<Value> = entries
        .iter()
        .filter(|e| e["entry"] == "revocation")
        .map(|rev| {
            let seq = rev["seq"].as_u64().unwrap_or_default();
            let revoked: Vec<&str> = rev["revoked"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let consumed_before: Vec<Value> = entries
                .iter()
                .filter(|c| c["entry"] == "consumption" && c["seq"].as_u64().unwrap_or_default() < seq)
                .filter(|c| {
                    let auth = &journal["authorizations"][c["authorization"].as_str().unwrap_or_default()];
                    let grant = auth["basis"]["grant_id"].as_str().unwrap_or_default();
                    revoked.contains(&grant)
                })
                .map(|c| json!({ "authorization": c["authorization"], "request_id": c["request_id"], "consumed_seq": c["seq"] }))
                .collect();
            json!({
                "grant_id": rev["grant_id"],
                "seq": seq,
                "source": source_text(&rev["source"]),
                "prevented": rev["invalidated"],
                "consumed_then_revoked": consumed_before,
                "suspended": rev["suspended"],
            })
        })
        .collect();

    let denials: Vec<Value> = entries
        .iter()
        .filter(|e| e["entry"] == "denial")
        .map(|e| json!({ "holder": e["holder"], "operation": e["operation"], "object": e["object"], "reason": e["reason"], "source": source_text(&e["source"]) }))
        .collect();

    let mut used = serde_json::Map::new();
    for r in export["ledger"]["reservations"].as_object().unwrap_or(&empty).values() {
        if r["released"] == false {
            let activity = r["activity"].as_str().unwrap_or_default().to_string();
            let total = used.get(&activity).and_then(Value::as_u64).unwrap_or(0) + r["cpu_ms"].as_u64().unwrap_or(0);
            used.insert(activity, json!(total));
        }
    }

    let outputs: Vec<Value> = entries
        .iter()
        .filter(|e| e["entry"] == "output")
        .map(|e| json!({ "holder": e["holder"], "activity": e["activity"], "space": e["space"], "object": e["object"], "hash": e["hash"] }))
        .collect();

    json!({
        "activities": activities,
        "outcomes": outcomes,
        "closed_with_unknown_outcomes": closed_with_unknown,
        "revocations": revocations,
        "denials": denials,
        "budget_used_cpu_ms": used,
        "budget_allowance_cpu_ms": export["ledger"]["budgets"],
        "outputs": outputs,
    })
}

/// The report as the experimental interface says it, one line per answer.
pub fn say(report: &Value) -> String {
    let mut lines = Vec::new();

    for (id, state) in report["activities"].as_object().into_iter().flatten() {
        lines.push(format!("{id} is {}.", state.as_str().unwrap_or("?")));
    }
    for o in report["outcomes"].as_array().into_iter().flatten() {
        let outcome = &o["outcome"];
        let state = match outcome["state"].as_str() {
            Some("reconciled") => {
                let resolution = &outcome["resolution"];
                let to = match (resolution["to"].as_str(), resolution["reason"]["why"].as_str()) {
                    (Some("failed"), Some(why)) => format!("failed ({})", why.replace('_', " ")),
                    (to, _) => to.unwrap_or("?").to_string(),
                };
                format!("reconciled to {to} ({} evidence)", outcome["evidence"].as_str().unwrap_or("?"))
            }
            Some(s) => s.to_string(),
            None => "no outcome".into(),
        };
        lines.push(format!(
            "{}: {state}, from {}. Submissions {}, re-dispatches {}, status checks {}.",
            o["request_id"].as_str().unwrap_or("?"),
            o["source"].as_str().unwrap_or("?"),
            o["submissions"],
            o["re_dispatches"],
            o["reconciliations"]
        ));
    }
    for c in report["closed_with_unknown_outcomes"].as_array().into_iter().flatten() {
        lines.push(format!("{} closed with unknown outcomes: {}.", c["activity"].as_str().unwrap_or("?"), c["unknown"]));
    }
    for r in report["revocations"].as_array().into_iter().flatten() {
        let prevented = r["prevented"].as_array().map(Vec::len).unwrap_or(0);
        let late = r["consumed_then_revoked"].as_array().map(Vec::len).unwrap_or(0);
        let verdict = match (prevented, late) {
            (0, 0) => "nothing was waiting on it".to_string(),
            (p, 0) => format!("it prevented {p} authorization(s) from being used"),
            (_, l) => format!("{l} authorization(s) had already been consumed; the revocation did not prevent them"),
        };
        lines.push(format!("Revocation of {} by {}: {verdict}.", r["grant_id"].as_str().unwrap_or("?"), r["source"].as_str().unwrap_or("?")));
    }
    for d in report["denials"].as_array().into_iter().flatten() {
        lines.push(format!(
            "Denied: {} {} by {}, recorded by {}.",
            d["operation"].as_str().unwrap_or("?"),
            d["object"].as_str().unwrap_or("?"),
            d["holder"].as_str().unwrap_or("?"),
            d["source"].as_str().unwrap_or("?")
        ));
    }
    for (activity, used) in report["budget_used_cpu_ms"].as_object().into_iter().flatten() {
        lines.push(format!("{activity} holds {used} CPU-ms of the shared budget."));
    }
    lines.join("\n")
}
