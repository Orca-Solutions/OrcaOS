// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The journal writer's ordering rules, checked without any platform.

use orca_ref::journal::Command;
use orca_ref::*;

const ADAPTER: &str = "mail-adapter";
const MINUTE: u64 = 60_000;

fn principal() -> Source {
    Source::principal("principal-1")
}

fn adapter() -> Source {
    Source::external("mock-mail", ADAPTER)
}

struct Fixture {
    journal: Journal,
    authorization: String,
    binding: String,
}

/// A1 with its send grant, a proposal, an approval and a minted authorization.
fn approved() -> Fixture {
    approved_until(60 * MINUTE)
}

fn approved_until(expires_at_ms: u64) -> Fixture {
    let mut journal = Journal::new();
    let p = principal();

    journal.apply(
        p.clone(),
        0,
        Command::OpenActivity {
            activity: "A1".into(),
            parent: None,
            approval_policy: vec!["send".into()],
        },
    );
    journal.apply(
        p.clone(),
        0,
        Command::Grant {
            grant_id: "g-send".into(),
            holder: "A1".into(),
            activity: "A1".into(),
            capability: "send".into(),
            object: "accountant@mock.test".into(),
            parent: None,
            expires_at_ms: Some(24 * 60 * MINUTE),
        },
    );
    journal.apply(
        Source::service("approval-path"),
        1,
        Command::Proposal {
            proposal_id: "prop-1".into(),
            activity: "A1".into(),
            holder: "drafter".into(),
            payload_hash: "h1".into(),
        },
    );
    journal.apply(
        p,
        2,
        Command::Approval {
            proposal_id: "prop-1".into(),
            payload_hash: "h1".into(),
        },
    );
    let Response::Minted {
        authorization,
        binding,
        ..
    } = journal.apply(
        Source::service("approval-path"),
        3,
        Command::MintAuthorization {
            activity: "A1".into(),
            operation: "send".into(),
            target: "accountant@mock.test".into(),
            adapter: ADAPTER.into(),
            payload_hash: "h1".into(),
            expires_at_ms,
            grant_id: "g-send".into(),
            proposal_id: Some("prop-1".into()),
        },
    )
    else {
        panic!("authorization not minted");
    };

    Fixture {
        journal,
        authorization,
        binding,
    }
}

fn consume(f: &mut Fixture, now: u64, delivery: &str) -> Response {
    f.journal.apply(
        adapter(),
        now,
        Command::ConsumeIfLive {
            authorization: f.authorization.clone(),
            binding: f.binding.clone(),
            delivery: delivery.into(),
            validity_ms: 10 * MINUTE,
            reservation: Some("A1:reserve:send".into()),
        },
    )
}

fn revoke(f: &mut Fixture, now: u64) {
    f.journal.apply(
        principal(),
        now,
        Command::Revoke {
            grant_id: "g-send".into(),
        },
    );
}

#[test]
fn request_identifiers_follow_the_protocol_form() {
    let mut f = approved();
    let Response::Consumed { request_id, .. } = consume(&mut f, 4, "d1") else {
        panic!("not consumed");
    };

    assert_eq!(request_id, "A1:send:auth-0001");
}

#[test]
fn sending_needs_the_approval_the_policy_marks() {
    let mut f = approved();
    let response = f.journal.apply(
        Source::service("approval-path"),
        4,
        Command::MintAuthorization {
            activity: "A1".into(),
            operation: "send".into(),
            target: "accountant@mock.test".into(),
            adapter: ADAPTER.into(),
            payload_hash: "h1".into(),
            expires_at_ms: 60 * MINUTE,
            grant_id: "g-send".into(),
            proposal_id: None,
        },
    );

    assert!(matches!(response, Response::Refused { .. }));
}

#[test]
fn an_approval_produces_one_authorization() {
    let mut f = approved();
    let response = f.journal.apply(
        Source::service("approval-path"),
        4,
        Command::MintAuthorization {
            activity: "A1".into(),
            operation: "send".into(),
            target: "accountant@mock.test".into(),
            adapter: ADAPTER.into(),
            payload_hash: "h1".into(),
            expires_at_ms: 60 * MINUTE,
            grant_id: "g-send".into(),
            proposal_id: Some("prop-1".into()),
        },
    );

    assert!(matches!(response, Response::Refused { .. }));
}

#[test]
fn revocation_committed_before_consumption_prevents_it() {
    let mut f = approved();

    // P1: the early check passes, then the revocation commits.
    let check = f.journal.apply(
        adapter(),
        4,
        Command::CheckLive {
            authorization: f.authorization.clone(),
            binding: f.binding.clone(),
            delivery: "d1".into(),
        },
    );
    assert_eq!(check, Response::Live);
    revoke(&mut f, 5);

    assert!(matches!(consume(&mut f, 6, "d1"), Response::Refused { .. }));
    assert_eq!(f.journal.activities["A1"].state, ActivityState::Suspended);
    assert!(matches!(
        f.journal.authorizations[&f.authorization].state,
        AuthorizationState::Invalidated { .. }
    ));
}

#[test]
fn revocation_after_consumption_does_not_undo_the_boundary() {
    let mut f = approved();
    assert!(matches!(consume(&mut f, 4, "d1"), Response::Consumed { .. }));

    // P2: consumed, then revoked. The record order is consumption, revocation.
    revoke(&mut f, 5);
    let kinds: Vec<&str> = f
        .journal
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            Kind::Consumption { .. } => Some("consumption"),
            Kind::Revocation { .. } => Some("revocation"),
            _ => None,
        })
        .collect();

    assert_eq!(kinds, ["consumption", "revocation"]);
    assert!(matches!(
        f.journal.authorizations[&f.authorization].state,
        AuthorizationState::Consumed { .. }
    ));
}

#[test]
fn the_same_delivery_resuming_learns_of_its_earlier_consumption() {
    let mut f = approved();
    consume(&mut f, 4, "d1");

    assert!(matches!(consume(&mut f, 5, "d1"), Response::AlreadyConsumed { .. }));
    assert!(matches!(consume(&mut f, 5, "d2"), Response::Refused { .. }));
}

#[test]
fn a_changed_payload_does_not_match_the_binding() {
    let mut f = approved();
    f.binding = authorization_binding("h2", "send", ADAPTER, "A1", 60 * MINUTE);

    assert!(matches!(consume(&mut f, 4, "d1"), Response::Refused { .. }));
}

#[test]
fn re_dispatch_needs_an_open_activity_and_a_live_grant() {
    let mut f = approved();
    let Response::Consumed { request_id, .. } = consume(&mut f, 4, "d1") else {
        panic!("not consumed");
    };
    let re_dispatch = |journal: &mut Journal, now| {
        journal.apply(
            adapter(),
            now,
            Command::ReDispatchIfLive {
                request_id: request_id.clone(),
                delivery: "d1".into(),
                idempotent: true,
            },
        )
    };

    assert!(matches!(re_dispatch(&mut f.journal, 5), Response::ReDispatchAllowed { .. }));

    f.journal.apply(principal(), 6, Command::Cancel { activity: "A1".into() });
    assert!(matches!(re_dispatch(&mut f.journal, 7), Response::Refused { .. }));
}

#[test]
fn reconciliation_survives_close_and_never_reopens() {
    let mut f = approved();
    let Response::Consumed { request_id, .. } = consume(&mut f, 4, "d1") else {
        panic!("not consumed");
    };
    f.journal.apply(principal(), 5, Command::Cancel { activity: "A1".into() });
    f.journal.apply(principal(), 6, Command::Close { activity: "A1".into() });

    let close_listed_unknown = f.journal.entries.iter().any(|e| {
        matches!(&e.kind, Kind::Close { unknown_outcomes, .. } if unknown_outcomes.contains(&request_id))
    });
    assert!(close_listed_unknown);

    // Not certified while the window is open: the outcome stays Unknown.
    f.journal.apply(
        adapter(),
        7,
        Command::Reconciliation {
            request_id: request_id.clone(),
            observation: StatusObservation::NotReceived { certified: false },
        },
    );
    assert_eq!(f.journal.receipts[&request_id].outcome, Outcome::Unknown);

    // Certified after the window closes: Failed (certified not started).
    f.journal.apply(
        adapter(),
        11 * MINUTE,
        Command::Reconciliation {
            request_id: request_id.clone(),
            observation: StatusObservation::NotReceived { certified: true },
        },
    );
    assert_eq!(
        f.journal.receipts[&request_id].outcome,
        Outcome::Reconciled {
            resolution: Resolution::Failed {
                reason: FailureReason::CertifiedNotStarted,
            },
            evidence: Evidence::Acknowledged,
        }
    );
    assert!(f.journal.receipts[&request_id].reservation_released);
    assert_eq!(f.journal.activities["A1"].state, ActivityState::Closed);
}

#[test]
fn only_the_issuing_adapter_updates_a_receipt() {
    let mut f = approved();
    let Response::Consumed { request_id, .. } = consume(&mut f, 4, "d1") else {
        panic!("not consumed");
    };
    let response = f.journal.apply(
        Source::external("mock-mail", "other-adapter"),
        5,
        Command::Reconciliation {
            request_id,
            observation: StatusObservation::Accepted {
                message_id: "m1".into(),
            },
        },
    );

    assert!(matches!(response, Response::Refused { .. }));
}

#[test]
fn a_new_grant_does_not_revive_an_invalidated_authorization() {
    let mut f = approved();
    revoke(&mut f, 4);
    f.journal.apply(
        principal(),
        5,
        Command::Grant {
            grant_id: "g-send-2".into(),
            holder: "A1".into(),
            activity: "A1".into(),
            capability: "send".into(),
            object: "accountant@mock.test".into(),
            parent: None,
            expires_at_ms: Some(24 * 60 * MINUTE),
        },
    );

    assert_eq!(f.journal.activities["A1"].state, ActivityState::Running);
    assert!(matches!(consume(&mut f, 6, "d1"), Response::Refused { .. }));
}

#[test]
fn revocation_cascades_to_derived_grants_and_child_activities() {
    let mut f = approved();
    f.journal.apply(
        principal(),
        4,
        Command::OpenActivity {
            activity: "A1.1".into(),
            parent: Some("A1".into()),
            approval_policy: vec![],
        },
    );
    f.journal.apply(
        principal(),
        4,
        Command::Grant {
            grant_id: "g-read".into(),
            holder: "A1".into(),
            activity: "A1".into(),
            capability: "read".into(),
            object: "c1@v1".into(),
            parent: None,
            expires_at_ms: None,
        },
    );
    f.journal.apply(
        Source::service("activity-supervisor"),
        4,
        Command::Grant {
            grant_id: "g-read-1".into(),
            holder: "processor-1".into(),
            activity: "A1.1".into(),
            capability: "read".into(),
            object: "c1@v1".into(),
            parent: Some("g-read".into()),
            expires_at_ms: Some(24 * 60 * MINUTE),
        },
    );
    f.journal.apply(principal(), 5, Command::Revoke { grant_id: "g-read".into() });

    assert!(!f.journal.grant_live("g-read-1", 6));
    assert_eq!(f.journal.activities["A1.1"].state, ActivityState::Suspended);
}

#[test]
fn an_approval_never_widens_the_grant() {
    let mut journal = approved().journal;
    let p = principal();
    for (seq, command) in [
        Command::Proposal {
            proposal_id: "prop-2".into(),
            activity: "A1".into(),
            holder: "drafter".into(),
            payload_hash: "h2".into(),
        },
        Command::Approval {
            proposal_id: "prop-2".into(),
            payload_hash: "h2".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        journal.apply(p.clone(), 10 + seq as u64, command);
    }

    // Approved, but addressed outside the send grant's object.
    let response = journal.apply(
        Source::service("approval-path"),
        12,
        Command::MintAuthorization {
            activity: "A1".into(),
            operation: "send".into(),
            target: "other@mock.test".into(),
            adapter: ADAPTER.into(),
            payload_hash: "h2".into(),
            expires_at_ms: 60 * MINUTE,
            grant_id: "g-send".into(),
            proposal_id: Some("prop-2".into()),
        },
    );

    assert!(matches!(response, Response::Refused { .. }));
}

#[test]
fn a_certificate_before_the_window_closes_is_not_trusted() {
    let mut f = approved();
    let Response::Consumed { request_id, .. } = consume(&mut f, 4, "d1") else {
        panic!("not consumed");
    };
    f.journal.apply(
        adapter(),
        5,
        Command::Reconciliation {
            request_id: request_id.clone(),
            observation: StatusObservation::NotReceived { certified: true },
        },
    );

    assert_eq!(f.journal.receipts[&request_id].outcome, Outcome::Unknown);
    assert!(!f.journal.receipts[&request_id].reservation_released);
}

#[test]
fn an_expired_authorization_is_never_re_dispatched() {
    let mut f = approved_until(5);
    let Response::Consumed { request_id, .. } = consume(&mut f, 4, "d1") else {
        panic!("not consumed");
    };
    let response = f.journal.apply(
        adapter(),
        6,
        Command::ReDispatchIfLive {
            request_id,
            delivery: "d1".into(),
            idempotent: true,
        },
    );

    assert!(matches!(response, Response::Refused { .. }));
}
