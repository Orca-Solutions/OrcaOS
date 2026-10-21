// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs every fixture in the repository, and checks that the runner itself
//! catches a wrong expectation.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::fake::version_answers;
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
    assert_all_pass("tests/fixtures");
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
    assert_eq!(crate::fake::hash(&a), crate::fake::hash(&b));
}
