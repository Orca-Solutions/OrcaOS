// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The seven acceptance tests, the test 7 variants and the test 3 pause-point
//! variant, each as one repetition that returns its checks. A test passes
//! only if every check holds in every repetition.
//!
//! Scorecard: excluded (test harness).

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use crate::golem::{lit, Golem};
use crate::inspector;
use crate::mock::Mock;
use crate::scenario::{activity_state, entries, outcome_state, receipt, Run};

#[derive(Serialize)]
pub struct Check {
    pub name: String,
    pub pass: bool,
    pub detail: Value,
}

#[derive(Serialize)]
pub struct Rep {
    pub test: String,
    pub rep: u32,
    pub run: String,
    pub seed: u64,
    pub started_ms: u64,
    pub elapsed_ms: u64,
    pub checks: Vec<Check>,
    pub notes: Value,
    pub inspector: Value,
    pub said: String,
}

impl Rep {
    pub fn passed(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.pass)
    }
}

fn check(name: &str, pass: bool, detail: Value) -> Check {
    Check { name: name.into(), pass, detail }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64
}

/// A small deterministic generator, so a repetition's timing can be replayed
/// from its seed.
pub fn jitter(seed: u64, max_ms: u64) -> Duration {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    Duration::from_millis(x % max_ms.max(1))
}

const RID: &str = "A1:send:auth-0001";
const SETTLE: Duration = Duration::from_secs(30);
/// Golem resumes interrupted agents some seconds after a restart.
const RECOVERY: Duration = Duration::from_secs(120);

pub struct Ctx<'a> {
    pub golem: &'a Golem,
    pub mock: &'a Mock,
}

type Body = fn(&Run, u64, &mut Vec<Check>, &mut Value) -> Result<(), String>;

pub fn body(test: &str) -> Option<Body> {
    Some(match test {
        "1" => t1,
        "2" => t2,
        "3" => t3,
        "3-p1" => t3_p1,
        "3-p2" => t3_p2,
        "4" => t4,
        "4-offline" => t4_offline,
        "5" => t5,
        "6" => t6,
        "7a" => t7a,
        "7b" => t7b,
        "7c" => t7c,
        "7d" => t7d,
        "7e" => t7e,
        _ => return None,
    })
}

pub fn repeat(ctx: &Ctx, test: &str, run: String, rep: u32, seed: u64) -> Rep {
    let body = body(test).expect("known test");
    let started = Instant::now();
    let started_ms = now_ms();
    let r = Run::new(run.clone(), ctx.golem, ctx.mock);
    let mut checks = Vec::new();
    let mut notes = json!({});

    if let Err(e) = body(&r, seed, &mut checks, &mut notes) {
        checks.push(check("the repetition ran to the end", false, json!(e)));
    }
    r.release_all();

    let report = inspector::report(&r.journal());
    Rep {
        test: test.into(),
        rep,
        run,
        seed,
        started_ms,
        elapsed_ms: started.elapsed().as_millis() as u64,
        checks,
        notes,
        said: inspector::say(&report),
        inspector: report,
    }
}

fn attempts(r: &Run) -> Vec<Value> {
    r.mock.entries("attempts", &r.id)
}

fn deliveries(r: &Run) -> Vec<Value> {
    r.mock.entries("deliveries", &r.id)
}

fn definitive(journal: &Value, rid: &str) -> bool {
    !matches!(outcome_state(journal, rid).as_str(), "unknown" | "submitted" | "none")
}

fn ready(r: &Run) -> Result<String, String> {
    r.setup()?;
    r.process_and_draft()
}

// Test 1: an unrelated read is denied.
fn t1(r: &Run, _seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    let loaded = r.setup()?;
    let attempts: Value = serde_json::from_str(&r.call(&r.agent2("Processor1", "processor-1"), "misbehave", &[])?).map_err(|e| e.to_string())?;
    let all_refused = attempts.as_array().is_some_and(|a| a.len() == 4 && a.iter().all(|x| x["allowed"] == false));
    checks.push(check("the store refused all four", all_refused, attempts.clone()));

    // Golem recorded each refusal in the processor's oplog; the inspector
    // journals them with the store as their source.
    let oplog = r.golem.oplog(&r.agent2("Processor1", "processor-1"))?;
    let denials = inspector::oplog_denials(&oplog);
    let rows: Vec<(String, String, String, String)> =
        denials.iter().map(|(op, object, reason)| ("processor-1".into(), op.clone(), object.clone(), reason.clone())).collect();
    r.supervisor("record_denials", &[&serde_json::to_string(&rows).unwrap()])?;
    checks.push(check("the processor made the attempts (Golem's oplog shows four refused calls)", denials.len() == 4, json!(denials)));

    let journal = r.journal();
    let journaled: Vec<&Value> = entries(&journal, "denial").into_iter().filter(|d| d["source"]["name"] == "golem-kv").collect();
    let objects: Vec<&str> = journaled.iter().filter_map(|d| d["object"].as_str()).collect();
    let expected = ["docs/tax-return-2025", "docs/doc-0005", "docs/c1@v2", "docs/c1@v1"];
    checks.push(check(
        "the journal records each denial with the store as its source",
        expected.iter().all(|o| objects.contains(o)),
        json!(objects),
    ));

    let mut unchanged = true;
    let mut hashes = Vec::new();
    for item in loaded.as_array().into_iter().flatten() {
        let object = item["object"].as_str().unwrap_or_default();
        let now: Value = serde_json::from_str(&r.call(&r.agent("Seeder"), "read", &[&lit("docs"), &lit(object)])?).unwrap_or_default();
        unchanged &= now["hash"] == item["hash"];
        hashes.push(json!({ "object": object, "before": item["hash"], "after": now["hash"] }));
    }
    checks.push(check("snapshot hashes are unchanged", unchanged, json!(hashes)));
    notes["oplog_denials"] = json!(denials.len());
    Ok(())
}

// Test 2: a changed payload is not authorized.
fn t2(r: &Run, _seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    let proposal = ready(r)?;
    let approved = r.approve(&proposal, "g-send")?;
    let approved_hash = r.display(&proposal)?["payload_hash"].clone();
    let (journal, sent) = r.wait_journal(SETTLE, |j| definitive(j, RID));
    checks.push(check("the approved message was sent", sent && outcome_state(&journal, RID) == "acknowledged", receipt(&journal, RID).clone()));

    let mut recipient = approved.message.clone();
    recipient["to"] = json!("other@mock.test");
    let mut attachment = approved.message.clone();
    attachment["attachment"]["content"] = json!("A different attachment (synthetic test data)\n");

    for (name, message, reason) in [
        ("a changed recipient is refused on hash mismatch", &recipient, "does not match"),
        ("a swapped attachment is refused on hash mismatch", &attachment, "does not match"),
        ("the sent message resubmitted is refused as consumed", &approved.message, "already consumed"),
    ] {
        let out = r.deliver_directly(&approved.authorization, approved.expires_at_ms, message)?;
        checks.push(check(name, out.contains("refused") && out.contains(reason), json!(out)));
    }

    // An approval adds to the send grant; it never widens it. A proposal
    // addressed outside the grant is refused at mint even once approved.
    let proposal = r.call(&r.agent("ApprovalPath"), "submit", &[&lit("A1"), &lit("drafter"), &lit(&recipient.to_string())])?;
    let minted = r.approve(&proposal, "g-send");
    let refused = matches!(&minted, Err(e) if e.contains("does not cover other@mock.test"));
    checks.push(check("an approved proposal to a recipient outside the grant is refused at mint", refused, json!(minted.err())));

    let delivered = deliveries(r);
    let one = delivered.len() == 1 && delivered[0]["payload_hash"] == approved_hash;
    checks.push(check("the delivery log shows one delivery carrying the approved hash", one, json!(delivered)));
    notes["attempts"] = json!(attempts(r).len());
    Ok(())
}

const A1_GRANTS: [&str; 5] = ["g-read-c1", "g-read-c2", "g-read-c3", "g-out", "g-send"];

// Test 3: revoke, then restart.
fn t3(r: &Run, seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    let proposal = ready(r)?;
    r.mock.hold_at("pre", &r.id)?;
    r.approve(&proposal, "g-send")?;
    r.wait_held("pre", SETTLE).ok_or("the adapter never reached the hold")?;

    // A1 is Waiting after approval and before dispatch.
    for grant in A1_GRANTS {
        r.supervisor("revoke", &[grant])?;
    }
    std::thread::sleep(jitter(seed, 500));
    let restart = r.golem.kill_and_restart()?;
    r.release_all();
    notes["restart_ms"] = json!(restart.as_millis() as u64);

    let (journal, settled) = r.wait_journal(RECOVERY, |j| {
        entries(j, "denial").iter().any(|d| d["object"] == "auth-0001" && d["operation"] == "enforcement check")
    });
    checks.push(check("after the restart the adapter's check is refused", settled, json!(entries(&journal, "denial"))));
    checks.push(check("A1 resumes Suspended", activity_state(&journal, "A1") == "suspended", json!(activity_state(&journal, "A1"))));
    checks.push(check(
        "the authorization stays invalid",
        journal["journal"]["authorizations"]["auth-0001"]["state"]["state"] == "invalidated",
        journal["journal"]["authorizations"]["auth-0001"]["state"].clone(),
    ));
    let replayed = entries(&journal, "submission").len() + entries(&journal, "re_dispatch").len();
    checks.push(check("replay writes no submission or re-dispatch entry", replayed == 0, json!(replayed)));
    checks.push(check("the attempt log is empty", attempts(r).is_empty(), json!(attempts(r))));

    // A new grant and approval use a new request identifier.
    r.supervisor("grant", &["g-send-2", "A1", "A1", "send", "accountant@mock.test"])?;
    r.supervisor("grant", &["g-d-propose-2", "drafter", "A1", "submit-proposal", "approval-path"])?;
    let draft: Value = serde_json::from_str(&r.call(&r.agent2("Activity", "A1"), "draft", &[])?).map_err(|e| e.to_string())?;
    let second = draft["proposal"].as_str().ok_or(format!("no second proposal: {draft}"))?.to_string();
    let approved = r.approve(&second, "g-send-2")?;
    let rid = format!("A1:send:{}", approved.authorization);
    let (journal, sent) = r.wait_journal(SETTLE, |j| definitive(j, &rid));

    let attempted: Vec<String> = attempts(r).iter().filter_map(|a| a["request_id"].as_str().map(str::to_string)).collect();
    checks.push(check(
        "the only attempt uses the new request identifier",
        sent && rid != RID && attempted == [format!("{}/{rid}", r.id)],
        json!({ "new": rid, "attempts": attempted, "outcome": receipt(&journal, &rid)["outcome"] }),
    ));
    Ok(())
}

fn pause_and_revoke(r: &Run, point: &str, seed: u64) -> Result<String, String> {
    let proposal = ready(r)?;
    r.mock.hold_at(point, &r.id)?;
    r.approve(&proposal, "g-send")?;
    let hold = r.wait_held(point, SETTLE).ok_or("the adapter never reached the pause point")?;

    r.supervisor("revoke", &["g-send"])?;
    std::thread::sleep(jitter(seed, 300));
    r.mock.release(hold)?;
    Ok(proposal)
}

// Test 3 variant, P1: revoked after the enforcement check, before consumption.
fn t3_p1(r: &Run, seed: u64, checks: &mut Vec<Check>, _notes: &mut Value) -> Result<(), String> {
    pause_and_revoke(r, "P1", seed)?;
    let (journal, settled) = r.wait_journal(SETTLE, |j| {
        entries(j, "denial").iter().any(|d| d["object"] == "auth-0001" && d["operation"] == "consume authorization")
    });

    checks.push(check("consumption is refused", settled, json!(entries(&journal, "denial"))));
    checks.push(check("no consumption entry", entries(&journal, "consumption").is_empty(), json!(entries(&journal, "consumption"))));
    checks.push(check("the attempt log shows no attempt", attempts(r).is_empty(), json!(attempts(r))));

    let report = inspector::report(&journal);
    let revocation = &report["revocations"][0];
    let prevented = revocation["prevented"].as_array().is_some_and(|p| p.iter().any(|a| a == "auth-0001"));
    let not_claimed_late = revocation["consumed_then_revoked"].as_array().is_some_and(Vec::is_empty);
    checks.push(check("the inspector says the revocation prevented the effect", prevented && not_claimed_late, revocation.clone()));
    Ok(())
}

// Test 3 variant, P2: revoked after consumption commits, before the send.
fn t3_p2(r: &Run, seed: u64, checks: &mut Vec<Check>, _notes: &mut Value) -> Result<(), String> {
    pause_and_revoke(r, "P2", seed)?;
    let (journal, _) = r.wait_journal(SETTLE, |j| definitive(j, RID));

    checks.push(check("at most one dispatch", attempts(r).len() <= 1 && deliveries(r).len() <= 1, json!({ "attempts": attempts(r), "deliveries": deliveries(r) })));
    checks.push(check("no re-dispatch", entries(&journal, "re_dispatch").is_empty(), json!(entries(&journal, "re_dispatch"))));

    let consumed = entries(&journal, "consumption").first().and_then(|e| e["seq"].as_u64());
    let revoked = entries(&journal, "revocation").first().and_then(|e| e["seq"].as_u64());
    checks.push(check(
        "the journal shows consumed, then revoked",
        matches!((consumed, revoked), (Some(c), Some(v)) if c < v),
        json!({ "consumption_seq": consumed, "revocation_seq": revoked }),
    ));

    let report = inspector::report(&journal);
    let revocation = &report["revocations"][0];
    let late = revocation["consumed_then_revoked"].as_array().is_some_and(|l| l.iter().any(|a| a["authorization"] == "auth-0001"));
    let no_claim = revocation["prevented"].as_array().is_some_and(Vec::is_empty);
    checks.push(check("the inspector does not claim the revocation prevented anything", late && no_claim, revocation.clone()));
    Ok(())
}

fn lost_ack(r: &Run, offline: bool, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    let proposal = ready(r)?;
    r.mock.fault("submit", "drop_reply", &r.id, Some(1), 0)?;
    if offline {
        r.mock.fault("status", "offline", &r.id, None, 0)?;
    }
    r.approve(&proposal, "g-send")?;
    let (journal, _) = r.wait_journal(SETTLE, |j| !entries(j, "reconciliation").is_empty());

    let first = entries(&journal, "receipt_update").first().map(|e| e["outcome"]["state"].clone());
    checks.push(check("the lost reply is recorded as Unknown, not success", first == Some(json!("unknown")), json!(first)));
    checks.push(check("the adapter checks status by request identifier", !entries(&journal, "reconciliation").is_empty(), json!(entries(&journal, "reconciliation"))));

    if offline {
        std::thread::sleep(Duration::from_secs(8));
        let journal = r.journal();
        let report = inspector::report(&journal);
        checks.push(check("with the status endpoint offline the outcome stays Unknown", outcome_state(&journal, RID) == "unknown", report["outcomes"].clone()));
        checks.push(check("no re-dispatch while the service may hold the request", entries(&journal, "re_dispatch").is_empty() && attempts(r).len() == 1, json!(attempts(r))));

        // Once the endpoint answers again, the scheduled check reconciles.
        r.mock.clear()?;
        let (journal, settled) = r.wait_journal(SETTLE, |j| definitive(j, RID));
        notes["after_endpoint_returns"] = json!({ "settled": settled, "outcome": receipt(&journal, RID)["outcome"] });
    } else {
        let (journal, _) = r.wait_journal(SETTLE, |j| definitive(j, RID));
        let reconciled = &receipt(&journal, RID)["outcome"];
        let source = entries(&journal, "reconciliation").last().map(|e| e["source"].clone()).unwrap_or_default();
        checks.push(check(
            "a reconciliation to Acknowledged, with the mail service as its source",
            reconciled["state"] == "reconciled" && reconciled["resolution"]["to"] == "acknowledged" && source["system"] == "mock-mail",
            json!({ "outcome": reconciled, "source": source }),
        ));
        checks.push(check("the delivery log shows one delivery", deliveries(r).len() == 1 && attempts(r).len() == 1, json!(deliveries(r))));
    }
    Ok(())
}

// Test 4: a lost acknowledgment.
fn t4(r: &Run, _seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    lost_ack(r, false, checks, notes)
}

fn t4_offline(r: &Run, _seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    lost_ack(r, true, checks, notes)
}

/// The grants an agent's cards actually give it, with the recipient
/// normalised so different instances compare equal.
fn card_grants(card: &Value) -> Vec<String> {
    let mut grants: Vec<String> = card["lowerPositive"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|g| {
            let (left, right) = g.split_once(" : ").unwrap_or((g, ""));
            let class = left.split(" @ ").next().unwrap_or(left);
            format!("{class} : {right}")
        })
        .collect();
    grants.sort();
    grants
}

// Test 5: a participant is replaced.
fn t5(r: &Run, _seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    r.setup()?;
    let a12 = r.agent2("Activity", "A1.2");
    r.call(&r.agent2("Activity", "A1.1"), "run_processor_1", &[])?;
    let extracted = r.call(&a12, "step_processor_2", &[&lit("processor-2"), &lit("extract")])?;
    notes["extract"] = json!(extracted);

    let old_agent = r.agent2("Processor2", "processor-2");
    let replaced = r.supervisor("replace", &["A1.2", "processor-2", &old_agent, "processor-2b"])?;
    let replaced: Value = serde_json::from_str(&replaced).unwrap_or(json!(replaced));
    checks.push(check("the old participant's store access is narrowed natively", replaced["native"] == "Ok(())", replaced.clone()));

    let direct = r.call(&old_agent, "finish", &[&lit("c3@v1")])?;
    checks.push(check("the old processor, called directly, is refused by the store", direct.contains("permission denied"), json!(direct)));
    let via = r.call(&a12, "step_processor_2", &[&lit("processor-2"), &lit("finish")])?;
    checks.push(check("the coordinator refuses to run the old processor", via.starts_with("refused"), json!(via)));
    let finished = r.call(&a12, "step_processor_2", &[&lit("processor-2b"), &lit("finish")])?;
    checks.push(check("the replacement finishes from the retained progress", finished.contains("\"written\""), json!(finished)));

    let journal = r.journal();
    let revoked: Vec<String> = entries(&journal, "revocation").iter().filter_map(|e| e["grant_id"].as_str().map(str::to_string)).collect();
    checks.push(check(
        "the journal shows the old participant's grants revoked",
        revoked == ["g-p2-read-c3", "g-p2-append"],
        json!(revoked),
    ));
    let new_grants: Vec<String> = journal["journal"]["grants"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, g)| g["holder"] == "processor-2b")
        .map(|(_, g)| format!("{} {}", g["capability"].as_str().unwrap_or(""), g["object"].as_str().unwrap_or("")))
        .collect();
    checks.push(check("the journal gives the new participant equivalent grants", new_grants.len() == 2, json!(new_grants)));

    let outputs: Vec<(String, String)> = entries(&journal, "output")
        .iter()
        .filter(|o| o["activity"] == "A1.2")
        .map(|o| (o["object"].as_str().unwrap_or("").to_string(), o["holder"].as_str().unwrap_or("").to_string()))
        .collect();
    let attributed = outputs.contains(&("c3@v1-notes".into(), "processor-2".into())) && outputs.contains(&("c3@v1".into(), "processor-2b".into()));
    checks.push(check("every retained output stays attributed to its producer", attributed, json!(outputs)));
    checks.push(check("A1.2 keeps its identity and is running", activity_state(&journal, "A1.2") == "running", json!(activity_state(&journal, "A1.2"))));

    let old_cards = r.golem.cards(&old_agent)?;
    let new_cards = r.golem.cards(&r.agent2("Processor2Alt", "processor-2b"))?;
    let initial = |cards: &Value| cards["cards"].as_array().and_then(|c| c.iter().find(|c| c["type"] == "Polymorphic")).map(card_grants);
    let narrowing = old_cards["cards"].as_array().is_some_and(|c| c.iter().any(|c| c["managedBy"]["type"] == "RuntimeDerived" && card_grants(c).is_empty()));
    checks.push(check(
        "the card list shows the new participant holding only equivalent grants",
        initial(&new_cards).is_some() && initial(&new_cards) == initial(&old_cards) && new_cards["cards"].as_array().map(Vec::len) == Some(1),
        json!({ "old": initial(&old_cards), "new": initial(&new_cards) }),
    ));
    checks.push(check("the card list shows the old participant's narrowing card", narrowing, old_cards));
    Ok(())
}

// Test 6: two children share one budget.
fn t6(r: &Run, _seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    r.supervisor("setup", &[])?;
    let whole = (10 * 60 * 1000).to_string();
    let barrier = std::sync::Barrier::new(2);

    let results: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = ["A1.1", "A1.2"]
            .into_iter()
            .map(|child| {
                let (barrier, whole) = (&barrier, &whole);
                s.spawn(move || {
                    let agent = r.agent2("Activity", child);
                    barrier.wait();
                    r.call(&agent, "reserve", &[&lit(&format!("{child}:reserve:race")), whole]).unwrap_or_else(|e| e)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let reserved = results.iter().filter(|r| *r == "reserved").count();
    let refused = results.iter().filter(|r| r.starts_with("refused")).count();
    checks.push(check("one child reserves, the other is refused", reserved == 1 && refused == 1, json!(results)));

    let journal = r.journal();
    let report = inspector::report(&journal);
    let total: u64 = report["budget_used_cpu_ms"].as_object().into_iter().flatten().filter_map(|(_, v)| v.as_u64()).sum();
    checks.push(check("the children's total never exceeds A1's budget", total <= 600_000, report["budget_used_cpu_ms"].clone()));
    let journaled = entries(&journal, "denial").iter().any(|d| d["operation"] == "reserve" && d["source"]["name"] == "budget");
    checks.push(check("the refusal is journaled", journaled, json!(entries(&journal, "denial"))));
    notes["winner"] = json!(results);
    Ok(())
}

/// Test 7: crash after consumption, act, then let the adapter recover.
fn crash_then(r: &Run, seed: u64, point: &str, lose_request: bool, action: &dyn Fn(&Run) -> Result<(), String>, notes: &mut Value) -> Result<u64, String> {
    let proposal = ready(r)?;
    if lose_request {
        r.mock.fault("submit", "drop_request", &r.id, Some(1), 0)?;
    }
    r.mock.hold_at(point, &r.id)?;
    r.mock.hold_at("recover", &r.id)?;
    r.approve(&proposal, "g-send")?;
    r.wait_held(point, SETTLE).ok_or("the adapter never reached the crash point")?;

    std::thread::sleep(jitter(seed, 300));
    let crashed_at = now_ms();
    let restart = r.golem.kill_and_restart()?;
    // The crashed process's hold is stale. The "recover" hold stays, so the
    // adapter cannot act on recovery before the action below commits.
    r.release_at(point);

    action(r)?;

    let resumed = r.wait_held("recover", RECOVERY);
    notes["crashed_at_ms"] = json!(crashed_at);
    notes["restart_ms"] = json!(restart.as_millis() as u64);
    notes["resumed_into_recovery"] = json!(resumed.is_some());
    if let Some(id) = resumed {
        r.mock.release(id)?;
    }
    Ok(crashed_at)
}

/// The cancellation or revocation committed before the adapter's first
/// status check, so the recovery ran after it.
fn acted_before_recovery(journal: &Value, kind: &str, checks: &mut Vec<Check>) {
    let acted = entries(journal, kind).first().and_then(|e| e["seq"].as_u64());
    let first_check = entries(journal, "reconciliation").first().and_then(|e| e["seq"].as_u64());
    let ordered = matches!((acted, first_check), (Some(a), Some(c)) if a < c);
    checks.push(check(
        &format!("the {kind} committed before the adapter's first status check"),
        ordered,
        json!({ kind: acted, "first_reconciliation": first_check }),
    ));
}

fn after(list: &[Value], at_ms: u64) -> Vec<Value> {
    list.iter().filter(|e| e["at_ms"].as_u64().unwrap_or(0) >= at_ms).cloned().collect()
}

/// The checks every variant after a cancellation or revocation shares. The
/// window starts at the crash, before the action, so it is stricter than
/// "after the cancellation or revocation" and catches a platform resend too.
fn no_resend(r: &Run, crashed_at: u64, journal: &Value, checks: &mut Vec<Check>) {
    let late = after(&attempts(r), crashed_at);
    checks.push(check("zero submission attempts after the crash, so none after the cancellation or revocation", late.is_empty(), json!(late)));
    checks.push(check("nothing is re-dispatched", entries(journal, "re_dispatch").is_empty(), json!(entries(journal, "re_dispatch"))));
    let wire_late = after(&r.mock.entries("wire", &r.id), crashed_at);
    checks.push(check("Golem did not resend the request on its own", wire_late.is_empty(), json!(wire_late)));
}

fn status_checked(journal: &Value) -> bool {
    !entries(journal, "reconciliation").is_empty()
}

fn t7a(r: &Run, seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    let acted = crash_then(r, seed, "C1", false, &|r| r.supervisor("cancel", &["A1"]).map(|_| ()), notes)?;
    let (journal, _) = r.wait_journal(SETTLE, status_checked);

    let first = entries(&journal, "reconciliation").first().map(|e| e["observation"].clone()).unwrap_or_default();
    checks.push(check("status checks report not received", first["status"] == "not_received", first));
    checks.push(check("no submission entry", entries(&journal, "submission").is_empty(), json!(entries(&journal, "submission"))));
    checks.push(check("while the window is open the outcome stays Unknown", outcome_state(&journal, RID) == "unknown", receipt(&journal, RID)["outcome"].clone()));
    acted_before_recovery(&journal, "cancellation", checks);
    no_resend(r, acted, &journal, checks);
    notes["not_after_ms"] = receipt(&journal, RID)["not_after_ms"].clone();
    Ok(())
}

fn t7b(r: &Run, seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    let acted = crash_then(r, seed, "C1", false, &|r| r.supervisor("revoke", &["g-send"]).map(|_| ()), notes)?;
    let (journal, _) = r.wait_journal(SETTLE, status_checked);

    let first = entries(&journal, "reconciliation").first().map(|e| e["observation"].clone()).unwrap_or_default();
    checks.push(check("status checks report not received", first["status"] == "not_received", first));
    checks.push(check("A1 is Suspended", activity_state(&journal, "A1") == "suspended", json!(activity_state(&journal, "A1"))));
    checks.push(check("while the window is open the outcome stays Unknown", outcome_state(&journal, RID) == "unknown", receipt(&journal, RID)["outcome"].clone()));

    // Re-granting later does not revive this authorization.
    r.supervisor("grant", &["g-send-2", "A1", "A1", "send", "accountant@mock.test"])?;
    std::thread::sleep(Duration::from_secs(3));
    let journal = r.journal();
    checks.push(check("re-granting revives nothing", activity_state(&journal, "A1") == "running" && entries(&journal, "submission").is_empty(), json!(activity_state(&journal, "A1"))));
    acted_before_recovery(&journal, "revocation", checks);
    no_resend(r, acted, &journal, checks);
    notes["not_after_ms"] = receipt(&journal, RID)["not_after_ms"].clone();
    Ok(())
}

fn t7c(r: &Run, seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    let acted = crash_then(r, seed, "C2", false, &|r| r.supervisor("cancel", &["A1"]).map(|_| ()), notes)?;
    let (journal, _) = r.wait_journal(SETTLE, |j| definitive(j, RID));

    let outcome = receipt(&journal, RID)["outcome"].clone();
    let source = entries(&journal, "reconciliation").last().map(|e| e["source"].clone()).unwrap_or_default();
    checks.push(check(
        "a status check finds the message accepted: reconciled to Acknowledged, from the mail service",
        outcome["state"] == "reconciled" && outcome["resolution"]["to"] == "acknowledged" && source["system"] == "mock-mail",
        json!({ "outcome": outcome, "source": source }),
    ));
    let before = attempts(r).iter().filter(|a| a["at_ms"].as_u64().unwrap_or(u64::MAX) < acted).count();
    checks.push(check("one attempt, made before the crash and the cancellation", before == 1, json!(attempts(r))));
    acted_before_recovery(&journal, "cancellation", checks);
    no_resend(r, acted, &journal, checks);
    Ok(())
}

fn t7d(r: &Run, seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    let cancel_and_close = |r: &Run| {
        r.supervisor("cancel", &["A1"])?;
        r.supervisor("close", &["A1"]).map(|_| ())
    };
    let acted = crash_then(r, seed, "C2", true, &cancel_and_close, notes)?;
    let (journal, _) = r.wait_journal(SETTLE, status_checked);

    checks.push(check("A1 closes", activity_state(&journal, "A1") == "closed", json!(activity_state(&journal, "A1"))));
    let close_lists = entries(&journal, "close").iter().any(|c| c["unknown_outcomes"].as_array().is_some_and(|u| u.iter().any(|x| x == RID)));
    checks.push(check("A1 closes with the outcome Unknown", close_lists && outcome_state(&journal, RID) == "unknown", receipt(&journal, RID)["outcome"].clone()));
    let report = inspector::report(&journal);
    let listed = report["closed_with_unknown_outcomes"].as_array().is_some_and(|l| l.iter().any(|c| c["activity"] == "A1"));
    checks.push(check("the inspector lists A1 among closed activities with unknown outcomes", listed, report["closed_with_unknown_outcomes"].clone()));
    let lost = r.mock.entries("wire", &r.id).iter().filter(|w| w["lost_in_transit"] == true).count();
    checks.push(check("the original request was lost in transit, not delivered", lost == 1 && deliveries(r).is_empty(), json!(r.mock.entries("wire", &r.id))));
    acted_before_recovery(&journal, "cancellation", checks);
    no_resend(r, acted, &journal, checks);
    notes["not_after_ms"] = receipt(&journal, RID)["not_after_ms"].clone();
    Ok(())
}

fn t7e(r: &Run, seed: u64, checks: &mut Vec<Check>, notes: &mut Value) -> Result<(), String> {
    crash_then(r, seed, "C1", false, &|_| Ok(()), notes)?;
    let (journal, _) = r.wait_journal(SETTLE, |j| definitive(j, RID));

    let first = entries(&journal, "reconciliation").first().map(|e| e["observation"].clone()).unwrap_or_default();
    checks.push(check("a status check reports not received", first["status"] == "not_received", first));
    let re = entries(&journal, "re_dispatch");
    checks.push(check("the adapter re-dispatches, journaled as a re-dispatch", re.len() == 1, json!(re)));
    checks.push(check(
        "one attempt for this identifier, the re-dispatch, and one delivery",
        attempts(r).len() == 1 && deliveries(r).len() == 1,
        json!({ "attempts": attempts(r), "deliveries": deliveries(r) }),
    ));
    checks.push(check("the outcome is Acknowledged", outcome_state(&journal, RID) == "acknowledged", receipt(&journal, RID)["outcome"].clone()));
    Ok(())
}

/// After the validity windows close: 7a, 7b and 7d end certified not started,
/// recorded without reopening anything.
pub fn after_window(ctx: &Ctx, rep: &Rep) -> Vec<Check> {
    let r = Run::new(rep.run.clone(), ctx.golem, ctx.mock);
    let journal = r.journal();
    let outcome = receipt(&journal, RID)["outcome"].clone();
    let certified = outcome["state"] == "reconciled"
        && outcome["resolution"]["to"] == "failed"
        && outcome["resolution"]["reason"]["why"] == "certified_not_started";
    let mut checks = vec![check("after the window closes: reconciled to Failed (certified not started)", certified, outcome)];

    if rep.test == "7a" {
        checks.push(check("the budget reservation is released", receipt(&journal, RID)["reservation_released"] == true, receipt(&journal, RID).clone()));
    }
    if rep.test == "7d" {
        checks.push(check("the later status result is recorded against A1 without reopening it", activity_state(&journal, "A1") == "closed", json!(activity_state(&journal, "A1"))));
    }
    let late = attempts(&r);
    checks.push(check("still zero submission attempts for the identifier", late.is_empty() || rep.test == "7c", json!(late)));
    checks
}
