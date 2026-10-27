// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What happens when Diamond Dog starts on a journal it wrote before.
//!
//! The rules are the Decision log's: durable work is not durable authority,
//! replay never exercises authority, and a crash after consumption is an
//! unknown outcome, never a new effect. So on start, recovery reads the
//! journal and only ever appends to it:
//!
//! - a consumption with no receipt after it gets a receipt with outcome
//!   unknown: the request may or may not have left, and it stays tracked
//!   until a status check settles it. Nothing is re-sent;
//! - an approval's authorization that was never consumed is voided: a new
//!   attempt needs a new proposal;
//! - every grant still live or held ends: a member gives it again;
//! - every open activity closes.
//!
//! The last two are this skeleton's limit, not the target. The Decision log
//! says activities persist across a restart and only authority does not;
//! until the runtime rebuilds its work state (apps, records, schedules,
//! activities) from storage, it closes what it cannot carry, and says so in
//! the journal.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::json;

use super::Core;
use super::journal::Opened;
use crate::ledger::{Entry, Source};

/// What recovery found and wrote.
#[derive(Debug, Default)]
pub struct Recovery {
    pub lines: u64,
    pub torn_tail_discarded: bool,
    /// Request identifiers consumed before the restart with no receipt.
    pub unknown: Vec<String>,
    /// Proposals whose approved authorization was never consumed.
    pub approvals_voided: Vec<String>,
    pub grants_ended: Vec<String>,
    pub activities_closed: Vec<String>,
}

impl Core {
    /// Applies the restart rules to the journal just opened. Returns what it did.
    pub fn recover(&mut self, opened: &Opened) -> Recovery {
        let entries: Vec<Entry> = self.entries().to_vec();
        let mut report = Recovery {
            lines: opened.lines,
            torn_tail_discarded: opened.torn_tail_discarded,
            ..Default::default()
        };

        // Ids are never reused across a restart, or across a wipe: a request
        // identifier that names a new attempt must not name an old one.
        for (prefix, n) in &opened.id_floors {
            self.seed_counter(prefix, *n);
        }
        for (prefix, n) in entries.iter().filter_map(minted_id) {
            self.seed_counter(prefix, n);
        }
        if entries.is_empty() {
            return report;
        }

        let restart = "Diamond Dog restarted";
        let source = || Source::service("recovery");

        // Consumed with no receipt: unknown, tracked, never re-sent.
        let receipted: BTreeSet<&str> = entries
            .iter()
            .filter(|e| matches!(e.kind.as_str(), "receipt" | "disclosure"))
            .filter_map(|e| e.request_id.as_deref())
            .collect();
        for e in entries.iter().filter(|e| e.kind == "consumption") {
            let Some(rid) = e.request_id.as_deref() else { continue };
            if receipted.contains(rid) {
                continue;
            }
            let plain = format!(
                "{restart} after the enforcement check consumed {rid} and before a receipt was written. It may or may not have left; it stays unknown until a status check settles it, and nothing is re-sent."
            );
            let mut line = Entry::new("receipt", source(), plain)
                .outcome(json!({ "state": "unknown" }))
                .evidence("observed")
                .request_id(rid)
                .detail(json!({ "recovered_from": e.seq }));
            line.app = e.app.clone();
            line.activity = e.activity.clone();
            line.member = e.member.clone();
            line.operation = e.operation.clone();
            line.object = e.object.clone();
            line.authority = e.authority.clone();
            line.grant = e.grant.clone();
            self.journal(line);
            report.unknown.push(rid.into());
        }

        // An approval's authorization not consumed before the restart is void.
        let settled: BTreeSet<String> = entries
            .iter()
            .filter_map(|e| match e.kind.as_str() {
                "consumption" => e.authority.as_ref().and_then(|a| a["approval"].as_str()).map(str::to_string),
                "approval_voided" | "withdrawal" => e.proposal.clone(),
                _ => None,
            })
            .collect();
        for e in entries.iter().filter(|e| e.kind == "authorization") {
            let Some(pid) = e.proposal.clone() else { continue };
            if settled.contains(&pid) || report.approvals_voided.contains(&pid) {
                continue;
            }
            let plain = format!("The approval of {pid} was voided: {restart}, and an authorization is never restored from the journal. A new proposal is needed.");
            let mut line = Entry::new("approval_voided", source(), plain).proposal(&pid).detail(json!({ "why": "restart" }));
            line.app = e.app.clone();
            line.activity = e.activity.clone();
            line.operation = e.operation.clone();
            line.object = e.object.clone();
            self.journal(line);
            report.approvals_voided.push(pid);
        }

        // Grants still live or held end: restart never resurrects authority.
        let mut grants: BTreeMap<String, (bool, &Entry)> = BTreeMap::new();
        for e in &entries {
            let Some(gid) = e.grant.clone() else { continue };
            match e.kind.as_str() {
                "grant" => {
                    grants.insert(gid, (true, e));
                }
                "revocation" | "grant_ended" => {
                    grants.entry(gid).and_modify(|g| g.0 = false);
                }
                "consumption" | "read" | "fetch" => {
                    let one_time = grants.get(&gid).is_some_and(|(_, line)| line.detail.as_ref().is_some_and(|d| d["kind"] == "one_time"));
                    if one_time {
                        grants.entry(gid).and_modify(|g| g.0 = false);
                    }
                }
                _ => {}
            }
        }
        for (gid, (live, e)) in grants {
            if !live {
                continue;
            }
            let plain = format!(
                "Grant {gid} ({} on {}) ended: {restart}, and a grant is never restored from the journal. A member gives it again.",
                e.operation.clone().unwrap_or_default(),
                e.object.clone().unwrap_or_default()
            );
            let mut line = Entry::new("grant_ended", source(), plain).grant(&gid).detail(json!({ "why": "restart" }));
            line.app = e.app.clone();
            line.member = e.member.clone();
            line.holder = e.holder.clone();
            line.operation = e.operation.clone();
            line.object = e.object.clone();
            self.journal(line);
            report.grants_ended.push(gid);
        }

        // Open activities close, through Closing, as the lifecycle allows.
        let mut last: BTreeMap<String, (&str, &Entry)> = BTreeMap::new();
        for e in entries.iter().filter(|e| e.kind == "activity") {
            let (Some(aid), Some(to)) = (e.activity.clone(), e.detail.as_ref().and_then(|d| d["to"].as_str())) else { continue };
            last.insert(aid, (to, e));
        }
        for (aid, (state, e)) in last {
            if state == "closed" {
                continue;
            }
            let why = format!("{restart}, and this skeleton does not yet carry work across a restart. Its receipts stay, and an unknown outcome stays tracked");
            let steps: &[(&str, &str)] = if state == "closing" { &[("closing", "closed")] } else { &[(state, "closing"), ("closing", "closed")] };
            for (from, to) in steps {
                let plain = format!("Activity {aid} is now {to}: {why}.");
                let mut line = Entry::new("activity", source(), plain).activity(&aid).detail(json!({ "from": from, "to": to, "why": why }));
                line.app = e.app.clone();
                line.member = e.member.clone();
                self.journal(line);
            }
            report.activities_closed.push(aid);
        }

        report
    }

    fn seed_counter(&mut self, prefix: &str, n: u64) {
        let counter = self.counters.entry(prefix.into()).or_insert(0);
        *counter = (*counter).max(n);
    }
}

/// The id a line records the minting of, as its prefix and number. Only the
/// field the minting call writes counts: an id named anywhere else (a
/// caller's text, a payload, a reference to something minted earlier) never
/// moves a counter.
fn minted_id(e: &Entry) -> Option<(&'static str, u64)> {
    let detail = |key: &str| e.detail.as_ref().and_then(|d| d[key].as_str());
    let (prefix, id) = match e.kind.as_str() {
        "activity" if detail("to") == Some("created") => ("A", e.activity.as_deref()?),
        "grant" => ("G", e.grant.as_deref()?),
        "proposal" => ("P", e.proposal.as_deref()?),
        "authorization" | "consumption" => ("auth-", detail("authorization")?),
        "record" | "answer" => ("R", e.object.as_deref()?.strip_prefix("record:")?),
        "schedule" => ("S", detail("schedule")?),
        "contribution" => ("C", detail("item")?),
        "notification" => ("N", detail("notification")?),
        _ => return None,
    };
    Some((prefix, id.strip_prefix(prefix)?.parse().ok()?))
}
