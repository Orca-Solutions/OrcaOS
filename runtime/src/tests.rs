// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs every fixture in the repository, and checks that the runner itself
//! catches a wrong expectation.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::runtime::version_answers;
use crate::fixture;
use crate::model::Schedule;
use crate::schedule::ticks_between;

fn load(dir: &str) -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files
        .into_iter()
        .map(|p| (p.display().to_string(), serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()))
        .collect()
}

fn assert_all_pass(dir: &str) {
    let fixtures = load(dir);
    assert!(!fixtures.is_empty(), "no fixtures in {dir}");
    for (path, f) in fixtures {
        let outcome = fixture::run(&f);
        assert!(outcome.passed(), "{path} failed:\n{}\n{}", outcome.failures.join("\n"), outcome.ledger.join("\n"));
    }
}

#[test]
fn the_reference_scenarios_pass() {
    assert_all_pass("../examples/diamond-dog-fake/fixtures");
}

#[test]
fn the_regression_scenarios_pass() {
    assert_all_pass("../fake/tests/fixtures");
}

#[test]
fn a_wrong_refusal_code_fails_the_fixture() {
    let (_, mut f) = load("../examples/diamond-dog-fake/fixtures").remove(0);
    let steps = f["steps"].as_array_mut().unwrap();
    let last = steps.last_mut().unwrap();
    last["expect"] = json!({ "refused": "no_grant" });
    assert!(!fixture::run(&f).passed());
}

#[test]
fn a_missing_ledger_entry_fails_the_fixture() {
    let (_, mut f) = load("../examples/diamond-dog-fake/fixtures").remove(0);
    f["expect_ledger"].as_array_mut().unwrap().push(json!({ "kind": "receipt" }));
    assert!(!fixture::run(&f).passed());
}

#[test]
fn a_forbidden_entry_fails_the_fixture() {
    let (_, mut f) = load("../examples/diamond-dog-fake/fixtures").remove(0);
    f["forbid_ledger"] = json!([{ "kind": "refusal" }]);
    assert!(!fixture::run(&f).passed());
}

#[test]
fn ledger_entries_must_appear_in_order() {
    let (_, mut f) = load("../examples/diamond-dog-fake/fixtures").remove(2);
    f["expect_ledger"].as_array_mut().unwrap().reverse();
    assert!(!fixture::run(&f).passed());
}

#[test]
fn an_older_minor_version_is_answered_and_a_newer_one_is_not() {
    assert!(version_answers("0.3", "0.1"));
    assert!(version_answers("0.3", "0.3"));
    assert!(!version_answers("0.3", "0.4"));
    assert!(!version_answers("0.3", "1.0"));
}

#[test]
fn a_repeated_hour_fires_once() {
    // Clocks in New York go back on 1 November 2026: 01:30 happens twice.
    let s = Schedule {
        id: "S1".into(),
        app: "a".into(),
        routine: "r".into(),
        at: "01:30".into(),
        zone: "America/New_York".into(),
        cadence: "daily".into(),
        catch_up_hours: 12,
        set_by: "dana".into(),
    };
    let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
    let ticks = ticks_between(&s, t("2026-10-31T12:00:00Z"), t("2026-11-02T12:00:00Z"));

    assert_eq!(ticks.len(), 2);
    assert_eq!(ticks[0].at, t("2026-11-01T05:30:00Z"));
    assert!(ticks[0].adjustment.is_some());
    assert_eq!(ticks[1].at, t("2026-11-02T06:30:00Z"));
}

#[test]
fn a_weekly_run_keeps_its_wall_clock_time_across_the_change() {
    let s = Schedule {
        id: "S1".into(),
        app: "a".into(),
        routine: "r".into(),
        at: "18:00".into(),
        zone: "America/New_York".into(),
        cadence: "weekly:sun".into(),
        catch_up_hours: 72,
        set_by: "dana".into(),
    };
    let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
    let ticks = ticks_between(&s, t("2026-10-24T00:00:00Z"), t("2026-11-02T00:00:00Z"));

    let at: Vec<DateTime<Utc>> = ticks.iter().map(|x| x.at).collect();
    assert_eq!(at, vec![t("2026-10-25T22:00:00Z"), t("2026-11-01T23:00:00Z")]);
}

#[test]
fn the_payload_hash_ignores_key_order() {
    let a: Value = serde_json::from_str(r#"{"to": "x", "body": {"b": 1, "a": 2}}"#).unwrap();
    let b: Value = serde_json::from_str(r#"{"body": {"a": 2, "b": 1}, "to": "x"}"#).unwrap();
    assert_eq!(crate::runtime::hash(&a), crate::runtime::hash(&b));
}

// ---- Restart: durable work is not durable authority ----

mod restart {
    use serde_json::{Value, json};

    use crate::core::journal::Journal;
    use crate::core::tests::scratch;
    use crate::runtime::Runtime;

    fn household() -> Value {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/diamond-dog-fake/household.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn daybook() -> Value {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/diamond-dog-fake/daybook-manifest.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn start(path: &std::path::Path) -> (Runtime, crate::core::recovery::Recovery) {
        let (journal, opened) = Journal::open(path).unwrap();
        let setup = household();
        let now = chrono::DateTime::parse_from_rfc3339(setup["now"].as_str().unwrap()).unwrap().with_timezone(&chrono::Utc);
        let mut rt = Runtime::with_journal(journal, now);
        let report = rt.core.recover(&opened);
        rt.restore_receipts();
        rt.configure(&setup).unwrap();
        (rt, report)
    }

    fn call(rt: &mut Runtime, path: &str, body: Value) -> Value {
        let (surface, version, name) = crate::fixture::split_call(path).unwrap();
        match rt.call(surface, version, name, &body) {
            Ok(v) => v,
            Err(e) => panic!("{path} failed: {}", crate::fixture::response(&Err(e))),
        }
    }

    const DANA: &str = r#"{"shell": "dana", "device": "dana-phone"}"#;

    fn dana() -> Value {
        serde_json::from_str(DANA).unwrap()
    }

    /// Installs Daybook, opens a reply activity, proposes a mail and has Dana
    /// approve it. Returns the activity, the proposal and the payload.
    fn approved_send(rt: &mut Runtime) -> (String, String, Value) {
        call(rt, "install/v0.3/install", json!({ "caller": dana(), "manifest": daybook() }));
        call(rt, "install/v0.3/switch_on", json!({ "caller": dana(), "app": "daybook" }));
        let a = call(rt, "activities/v0.3/open", json!({ "caller": { "app": "daybook", "member": "dana" }, "routine": "reply" }));
        let aid = a["activity"].as_str().unwrap().to_string();

        let payload = json!({ "to": "school@example.test", "subject": "Field trip", "body": "Signed." });
        let p = call(rt, "proposals/v0.3/propose", json!({ "caller": { "routine": aid }, "operation": "mail.send", "object": "mail:household-mail", "payload": payload }));
        let pid = p["proposal"].as_str().unwrap().to_string();
        call(rt, "proposals/v0.3/approve", json!({ "caller": dana(), "proposal": pid, "payload_hash": p["payload_hash"] }));
        (aid, pid, payload)
    }

    #[test]
    fn a_crash_after_consumption_leaves_the_outcome_unknown_and_nothing_is_resent() {
        let path = scratch("crash-after-consumption");
        let request_id = {
            let (mut rt, _) = start(&path);
            let (aid, pid, payload) = approved_send(&mut rt);
            // The enforcement check commits the consumption; the process dies
            // before the adapter writes a receipt.
            let consumed = rt.enforce(&aid, "mail.send", "mail:household-mail", &payload, Some(pid), None).unwrap();
            consumed.request_id.clone()
        };

        let (mut rt, report) = start(&path);
        assert_eq!(report.unknown, vec![request_id.clone()]);
        assert_eq!(report.activities_closed.len(), 1);

        let receipt = rt.ledger().iter().rev().find(|e| e.kind == "receipt").unwrap().clone();
        assert_eq!(receipt.request_id.as_deref(), Some(request_id.as_str()));
        assert_eq!(receipt.outcome, Some(json!({ "state": "unknown" })));
        assert_eq!(receipt.evidence.as_deref(), Some("observed"));

        // Still tracked: a status check by request identifier finds it, and
        // with no answer it stays unknown. Nothing is submitted again.
        rt.control("mail/fault", &json!({ "handle": "household-mail", "fault": "status_unavailable" })).unwrap();
        let after = call(&mut rt, "ledger/v0.1/reconcile", json!({ "caller": dana(), "request_id": request_id }));
        assert_eq!(after["outcome"], json!({ "state": "unknown" }));
        assert_eq!(rt.mock.summary()["mail"]["household-mail"]["arrived_count"], 0);
        assert!(rt.ledger().iter().all(|e| e.kind != "consumption" || e.seq <= receipt.seq));
    }

    #[test]
    fn an_approval_not_acted_on_is_void_after_a_restart() {
        let path = scratch("approval-voided");
        let pid = {
            let (mut rt, _) = start(&path);
            approved_send(&mut rt).1
        };

        let (rt, report) = start(&path);
        assert_eq!(report.approvals_voided, vec![pid.clone()]);
        let voided = rt.ledger().iter().find(|e| e.kind == "approval_voided").unwrap();
        assert_eq!(voided.proposal.as_deref(), Some(pid.as_str()));
        assert!(Journal::verify_file(&path).is_ok());
    }

    #[test]
    fn a_restart_never_resurrects_a_grant() {
        let path = scratch("grants-end");
        {
            let (mut rt, _) = start(&path);
            call(&mut rt, "install/v0.3/install", json!({ "caller": dana(), "manifest": daybook() }));
            call(
                &mut rt,
                "install/v0.3/switch_on",
                json!({ "caller": dana(), "app": "daybook", "standing_grants": [{ "operation": "mail.read", "object": "mail:household-mail" }] }),
            );
            call(&mut rt, "activities/v0.3/open", json!({ "caller": { "app": "daybook", "member": "dana" }, "routine": "intake" }));
        }

        let (rt, report) = start(&path);
        // The install grant and the child the activity drew from it.
        assert_eq!(report.grants_ended.len(), 2);
        assert!(rt.core.grants().next().is_none());
        let ended = rt.ledger().iter().filter(|e| e.kind == "grant_ended" && e.detail.as_ref().is_some_and(|d| d["why"] == "restart")).count();
        assert_eq!(ended, 2);
    }

    #[test]
    fn ids_are_not_reused_after_a_restart() {
        let path = scratch("ids");
        let (first_activity, first_request) = {
            let (mut rt, _) = start(&path);
            let (aid, pid, payload) = approved_send(&mut rt);
            let receipt = call(&mut rt, "ledger/v0.1/act", json!({ "caller": { "routine": aid }, "operation": "mail.send", "object": "mail:household-mail", "proposal": pid, "payload": payload }));
            (aid, receipt["request_id"].as_str().unwrap().to_string())
        };

        let (mut rt, _) = start(&path);
        let (aid, pid, payload) = approved_send(&mut rt);
        let receipt = call(&mut rt, "ledger/v0.1/act", json!({ "caller": { "routine": aid }, "operation": "mail.send", "object": "mail:household-mail", "proposal": pid, "payload": payload }));

        assert_ne!(aid, first_activity);
        assert_ne!(receipt["request_id"].as_str().unwrap(), first_request);
        assert_eq!(receipt["outcome"]["state"], "acknowledged");
    }
}
