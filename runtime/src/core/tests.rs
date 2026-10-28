// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The enforcement core on its own: the journal's chain and its file, the
//! lifecycle's moves, and consume-if-live against revocation, reuse, expiry
//! and a closing activity.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use super::authority::{Basis, Grant, GrantState, Holder, NotLive};
use super::journal::Journal;
use super::lifecycle::{Activity, ActivityState};
use super::{Core, Names};
use crate::ledger::{Entry, Source};
use crate::model::GrantKind;

fn t(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// A fresh path in the system's temporary directory for one test's journal.
pub fn scratch(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!("diamond-dog-{name}-{}-{n}.journal", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn line(text: &str) -> Entry {
    Entry::new("claim", Source::participant("test"), text)
}

const NAMES: Names = &|app: &str| app.to_string();

/// A core holding one running activity, A1, with a standing grant G1 for
/// mail.send, drawn from the install grant G0 when `drawn` is set.
fn core_with_grant(drawn: bool) -> Core {
    let mut core = Core::new(Journal::in_memory(), t("2026-11-02T14:00:00Z"));
    let now = core.clock.now;
    let deadline = now + Duration::minutes(30);

    let activity = Activity::new("A1".into(), "daybook", "reply", "dana", BTreeSet::from(["dana".into()]), None, json!(null), BTreeSet::new(), now, deadline, None, None);
    core.open_activity(activity, Entry::new("activity", Source::service("test"), "opened").activity("A1").detail(json!({ "to": "created" })));
    core.transition("A1", ActivityState::Running, "it started work", Source::service("test"), NAMES).unwrap();

    let mut from = None;
    if drawn {
        let install = Grant::new("G0".into(), Holder::App("daybook".into()), "daybook", "mail.send", "mail:household-mail", "dana", GrantKind::Standing, None, None, None);
        core.issue_grant(install, Entry::new("grant", Source::principal("dana"), "install grant").grant("G0"));
        from = Some("G0".to_string());
    }
    let grant = Grant::new("G1".into(), Holder::Activity("A1".into()), "daybook", "mail.send", "mail:household-mail", "dana", GrantKind::Standing, None, Some(deadline), from);
    core.issue_grant(grant, Entry::new("grant", Source::principal("dana"), "grant").grant("G1").activity("A1"));
    core
}

fn mint_from_grant(core: &mut Core) -> String {
    let basis = Basis::Grant { grant: "G1".into(), principal: "dana".into() };
    core.mint(basis, "A1", "mail.send", "mail:household-mail", "mail", "sha256:draft", None, None)
}

fn consumption() -> Entry {
    Entry::new("consumption", Source::service("enforcement check"), "consumed")
}

fn kinds(core: &Core, kind: &str) -> usize {
    core.entries().iter().filter(|e| e.kind == kind).count()
}

// ---- The journal ----

#[test]
fn a_journal_reopens_with_its_chain_intact() {
    let path = scratch("reopen");
    {
        let (mut journal, opened) = Journal::open(&path).unwrap();
        assert_eq!(opened.lines, 0);
        for i in 1..=3 {
            assert_eq!(journal.append(line(&format!("line {i}"))), i);
        }
    }

    let (mut journal, opened) = Journal::open(&path).unwrap();
    assert_eq!(opened.lines, 3);
    assert!(!opened.torn_tail_discarded);
    assert_eq!(journal.entries()[2].plain, "line 3");
    assert_eq!(journal.append(line("line 4")), 4);
    assert_eq!(Journal::verify_file(&path).unwrap().lines, 4);
}

#[test]
fn an_edited_line_stops_the_journal_opening() {
    let path = scratch("edited");
    {
        let (mut journal, _) = Journal::open(&path).unwrap();
        journal.append(line("Dana approved P1"));
        journal.append(line("Sam approved P2"));
        journal.append(line("Dana revoked G1"));
    }
    let text = std::fs::read_to_string(&path).unwrap().replace("Sam approved P2", "Sam approved P9");
    std::fs::write(&path, text).unwrap();

    let err = Journal::open(&path).err().unwrap();
    assert!(err.contains("line 2") && err.contains("changed after it was written"), "{err}");
    assert!(Journal::verify_file(&path).is_err());
}

#[test]
fn a_dropped_line_stops_the_journal_opening() {
    let path = scratch("dropped");
    {
        let (mut journal, _) = Journal::open(&path).unwrap();
        for i in 1..=3 {
            journal.append(line(&format!("line {i}")));
        }
    }
    let text = std::fs::read_to_string(&path).unwrap();
    let kept: Vec<&str> = text.lines().enumerate().filter(|(i, _)| *i != 1).map(|(_, l)| l).collect();
    std::fs::write(&path, kept.join("\n") + "\n").unwrap();

    let err = Journal::open(&path).err().unwrap();
    assert!(err.contains("missing or out of order"), "{err}");
}

#[test]
fn a_torn_final_line_never_committed() {
    let path = scratch("torn");
    {
        let (mut journal, _) = Journal::open(&path).unwrap();
        journal.append(line("line 1"));
        journal.append(line("line 2"));
    }
    let committed = std::fs::metadata(&path).unwrap().len();
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(br#"{"seq":3,"prev":"sha256:ab"#).unwrap();

    assert!(Journal::verify_file(&path).unwrap().torn_tail_discarded);
    let (mut journal, opened) = Journal::open(&path).unwrap();
    assert!(opened.torn_tail_discarded);
    assert_eq!(opened.lines, 2);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), committed);

    assert_eq!(journal.append(line("line 3")), 3);
    assert_eq!(Journal::verify_file(&path).unwrap().lines, 3);
}

#[test]
fn a_torn_tail_inside_a_character_is_discarded_too() {
    let path = scratch("torn-utf8");
    let (mut journal, _) = Journal::open(&path).unwrap();
    journal.append(line("line 1"));
    drop(journal);

    let committed = std::fs::metadata(&path).unwrap().len();
    // The first two bytes of a three-byte character, and no newline.
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(&[b'{', b'"', 0xE2, 0x82]).unwrap();

    let (mut journal, opened) = Journal::open(&path).unwrap();
    assert!(opened.torn_tail_discarded);
    assert_eq!(opened.lines, 1);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), committed);
    assert_eq!(journal.append(line("line 2")), 2);
}

#[test]
fn a_wipe_leaves_an_empty_journal_and_keeps_the_id_floors() {
    let path = scratch("wipe");
    let (mut journal, _) = Journal::open(&path).unwrap();
    journal.append(line("line 1"));
    journal.wipe(&BTreeMap::from([("auth-".to_string(), 7), ("A".to_string(), 3)]));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    assert_eq!(journal.append(line("after")), 1);
    assert_eq!(Journal::verify_file(&path).unwrap().lines, 1);

    let (_, opened) = Journal::open(&path).unwrap();
    assert_eq!(opened.id_floors.get("auth-"), Some(&7));
    assert_eq!(opened.id_floors.get("A"), Some(&3));
}

// ---- The lifecycle ----

#[test]
fn the_lifecycle_allows_only_the_contract_moves() {
    use ActivityState::*;
    let all = [Created, Running, Waiting, Suspended, Closing, Closed];
    let allowed = [
        (Created, Running),
        (Running, Waiting),
        (Waiting, Running),
        (Running, Suspended),
        (Suspended, Running),
        (Created, Closing),
        (Running, Closing),
        (Waiting, Closing),
        (Suspended, Closing),
        (Closing, Closed),
    ];
    for from in all {
        for to in all {
            assert_eq!(from.may_become(to), allowed.contains(&(from, to)), "{} to {}", from.word(), to.word());
        }
    }
}

#[test]
fn an_illegal_move_is_not_made_and_leaves_no_line() {
    let mut core = core_with_grant(false);
    let lines = core.entries().len();
    assert!(core.transition("A1", ActivityState::Closed, "skipping Closing", Source::service("test"), NAMES).is_err());
    assert_eq!(core.activity("A1").unwrap().state(), ActivityState::Running);
    assert_eq!(core.entries().len(), lines);
}

// ---- Consume-if-live ----

#[test]
fn a_consumption_commits_before_it_returns() {
    let mut core = core_with_grant(false);
    let auth = mint_from_grant(&mut core);
    let consumed = core.consume_if_live(&auth, consumption()).unwrap();

    assert_eq!(consumed.request_id, format!("A1:mail.send:{auth}"));
    let last = core.entries().last().unwrap();
    assert_eq!(last.kind, "consumption");
    assert_eq!(last.request_id.as_deref(), Some(consumed.request_id.as_str()));
    assert_eq!(last.authority, Some(json!({ "grant": "G1", "principal": "dana" })));
    assert!(!core.authorization(&auth).unwrap().unconsumed);
}

#[test]
fn an_authorization_is_consumed_once() {
    let mut core = core_with_grant(false);
    let auth = mint_from_grant(&mut core);
    core.consume_if_live(&auth, consumption()).unwrap();

    let again = core.consume_if_live(&auth, consumption()).unwrap_err();
    assert_eq!(again.code(), "grant_used");
    assert_eq!(kinds(&core, "consumption"), 1);
}

#[test]
fn a_revocation_committed_first_prevents_the_consumption() {
    let mut core = core_with_grant(false);
    let auth = mint_from_grant(&mut core);
    core.revoke("G1", Source::principal("dana"), "Dana", NAMES);

    let refused = core.consume_if_live(&auth, consumption()).unwrap_err();
    assert_eq!(refused.code(), "grant_ended");
    assert_eq!(kinds(&core, "consumption"), 0);
    assert_eq!(core.activity("A1").unwrap().state(), ActivityState::Suspended);
}

#[test]
fn a_revocation_reaches_what_was_drawn_from_the_grant() {
    let mut core = core_with_grant(true);
    let auth = mint_from_grant(&mut core);
    let stopped = core.revoke("G0", Source::principal("dana"), "Dana", NAMES);

    assert_eq!(stopped, vec!["G0".to_string(), "G1".to_string()]);
    assert_eq!(core.grant("G1").unwrap().state(), GrantState::Revoked);
    assert!(core.consume_if_live(&auth, consumption()).is_err());
    assert_eq!(kinds(&core, "revocation"), 2);
    assert_eq!(core.activity("A1").unwrap().state(), ActivityState::Suspended);
}

#[test]
fn a_parent_grant_revoked_without_the_cascade_still_stops_the_child() {
    // Even if a child grant were somehow left live, consume-if-live walks the
    // chain it was drawn from.
    let mut core = core_with_grant(true);
    let auth = mint_from_grant(&mut core);
    let entry = Entry::new("revocation", Source::principal("dana"), "only the install grant").grant("G0");
    core.change_grant("G0", GrantState::Revoked, entry).unwrap();

    match core.consume_if_live(&auth, consumption()).unwrap_err() {
        NotLive::Grant { code, grant, .. } => assert_eq!((code, grant.as_str()), ("grant_revoked", "G1")),
        other => panic!("expected the grant to be refused, got {other:?}"),
    }
}

#[test]
fn a_closing_activity_consumes_nothing() {
    let mut core = core_with_grant(false);
    let auth = mint_from_grant(&mut core);
    core.transition("A1", ActivityState::Closing, "cancelled", Source::principal("dana"), NAMES).unwrap();

    assert_eq!(core.consume_if_live(&auth, consumption()).unwrap_err().code(), "activity_not_open");
}

#[test]
fn an_expired_approval_is_not_consumed() {
    let mut core = core_with_grant(false);
    let expires = core.clock.now + Duration::minutes(10);
    let basis = Basis::Approval { proposal: "P1".into(), member: "dana".into() };
    let auth = core.mint(basis, "A1", "mail.send", "mail:household-mail", "mail", "sha256:draft", Some(expires), None);

    core.clock.now = expires;
    assert_eq!(core.consume_if_live(&auth, consumption()).unwrap_err().code(), "approval_expired");
}

#[test]
fn the_consumption_line_names_what_the_authorization_covers() {
    let mut core = core_with_grant(false);
    let auth = mint_from_grant(&mut core);
    let misleading = consumption().operation("calendar.write").object("calendar:someone-else");
    core.consume_if_live(&auth, misleading).unwrap();

    let last = core.entries().last().unwrap();
    assert_eq!(last.operation.as_deref(), Some("mail.send"));
    assert_eq!(last.object.as_deref(), Some("mail:household-mail"));
    assert_eq!(last.detail.as_ref().unwrap()["payload_hash"], "sha256:draft");
}

#[test]
fn a_final_grant_state_is_final() {
    let mut core = core_with_grant(false);
    core.revoke("G1", Source::principal("dana"), "Dana", NAMES);
    let entry = Entry::new("grant_confirmed", Source::principal("dana"), "back").grant("G1");
    assert!(core.change_grant("G1", GrantState::Live, entry).is_err());
    assert_eq!(kinds(&core, "grant_confirmed"), 0);
}
