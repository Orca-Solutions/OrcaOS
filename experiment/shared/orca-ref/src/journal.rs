// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The journal writer: the single point of order for grants, revocations,
//! cancellations, approvals, authorizations, consumption and receipts.
//!
//! Every command is applied as one step. A platform hosts one `Journal` per
//! principal and must apply commands one at a time, persisting each entry
//! before it returns the response, so that "the record commits" means the same
//! thing on every platform.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::binding::authorization_binding;
use crate::model::*;
use crate::request_id;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    OpenActivity {
        activity: String,
        parent: Option<String>,
        /// Operations that need a fresh approval beyond sufficient authority.
        approval_policy: Vec<String>,
    },
    /// Running and Waiting only; the other states have their own commands.
    Transition { activity: String, to: ActivityState },
    Grant {
        grant_id: String,
        holder: String,
        activity: String,
        capability: String,
        object: String,
        parent: Option<String>,
        expires_at_ms: Option<u64>,
    },
    Revoke { grant_id: String },
    Cancel { activity: String },
    Close { activity: String },
    Denial {
        holder: String,
        operation: String,
        object: String,
        reason: String,
    },
    Claim { holder: String, activity: String, statement: String },
    Output {
        holder: String,
        activity: String,
        space: String,
        object: String,
        hash: String,
    },
    Proposal {
        proposal_id: String,
        activity: String,
        holder: String,
        payload_hash: String,
    },
    Approval { proposal_id: String, payload_hash: String },
    MintAuthorization {
        activity: String,
        operation: String,
        /// What the operation acts on, such as the recipient of a send. It
        /// must be the grant's object.
        target: String,
        adapter: String,
        payload_hash: String,
        expires_at_ms: u64,
        grant_id: String,
        proposal_id: Option<String>,
    },
    /// The adapter's early enforcement check. Refuses early; never consumes.
    CheckLive {
        authorization: String,
        binding: String,
        delivery: String,
    },
    /// Commits consumption only if the authorization, its grant and the
    /// activity are all live at this moment.
    ConsumeIfLive {
        authorization: String,
        binding: String,
        /// Identifies one delivery of the authorization to the adapter. The same
        /// delivery resuming after a crash gets `AlreadyConsumed`; any other
        /// delivery of a consumed authorization is refused.
        delivery: String,
        /// The request's validity window at the external service.
        validity_ms: u64,
        reservation: Option<String>,
    },
    Submission { request_id: String, delivery: String },
    ReceiptUpdate { request_id: String, result: SubmitResult },
    /// A new submission reusing a consumed request identifier. Allowed only
    /// while the activity is open, the grant live and the operation idempotent.
    ReDispatchIfLive {
        request_id: String,
        delivery: String,
        idempotent: bool,
    },
    /// A status check and its result. Allowed after cancellation, revocation
    /// and close, under the adapter's standing reconciliation grant.
    Reconciliation {
        request_id: String,
        observation: StatusObservation,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "snake_case")]
pub enum Response {
    Recorded { seq: u64 },
    Minted {
        seq: u64,
        authorization: String,
        binding: String,
    },
    Live,
    Consumed {
        seq: u64,
        request_id: String,
        issued_at_ms: u64,
        not_after_ms: u64,
    },
    AlreadyConsumed {
        request_id: String,
        issued_at_ms: u64,
        not_after_ms: u64,
    },
    ReDispatchAllowed { seq: u64 },
    Refused { reason: String, seq: Option<u64> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    pub at_ms: u64,
    pub source: Source,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entry", rename_all = "snake_case")]
pub enum Kind {
    ActivityCreated {
        activity: String,
        parent: Option<String>,
        approval_policy: Vec<String>,
    },
    Transition {
        activity: String,
        from: ActivityState,
        to: ActivityState,
        reason: String,
    },
    Grant {
        grant_id: String,
        holder: String,
        activity: String,
        capability: String,
        object: String,
        parent: Option<String>,
        expires_at_ms: Option<u64>,
    },
    Revocation {
        grant_id: String,
        revoked: Vec<String>,
        invalidated: Vec<String>,
        suspended: Vec<String>,
    },
    Cancellation {
        activity: String,
        closing: Vec<String>,
        invalidated: Vec<String>,
    },
    Close {
        activity: String,
        closed: Vec<String>,
        expired_grants: Vec<String>,
        unknown_outcomes: Vec<String>,
    },
    Denial {
        holder: String,
        operation: String,
        object: String,
        reason: String,
    },
    Claim {
        holder: String,
        activity: String,
        statement: String,
    },
    Output {
        holder: String,
        activity: String,
        space: String,
        object: String,
        hash: String,
    },
    Proposal {
        proposal_id: String,
        activity: String,
        holder: String,
        payload_hash: String,
    },
    Approval {
        proposal_id: String,
        payload_hash: String,
    },
    AuthorizationMinted {
        authorization: String,
        activity: String,
        operation: String,
        adapter: String,
        payload_hash: String,
        binding: String,
        expires_at_ms: u64,
        basis: Basis,
    },
    Consumption {
        authorization: String,
        request_id: String,
        delivery: String,
        issued_at_ms: u64,
        not_after_ms: u64,
        reservation: Option<String>,
    },
    Submission {
        request_id: String,
        delivery: String,
        attempt: u32,
    },
    ReceiptUpdate {
        request_id: String,
        evidence: Evidence,
        outcome: Outcome,
    },
    ReDispatch {
        request_id: String,
        delivery: String,
    },
    Reconciliation {
        request_id: String,
        observation: StatusObservation,
        outcome: Outcome,
    },
    ReservationReleased {
        reservation: String,
        request_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub id: String,
    pub parent: Option<String>,
    pub state: ActivityState,
    pub approval_policy: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub holder: String,
    pub activity: String,
    pub capability: String,
    pub object: String,
    pub parent: Option<String>,
    pub expires_at_ms: Option<u64>,
    pub revoked: bool,
    pub expired_at_close: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub activity: String,
    pub holder: String,
    pub payload_hash: String,
    pub approved_hash: Option<String>,
    pub authorized: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authorization {
    pub id: String,
    pub activity: String,
    pub operation: String,
    pub adapter: String,
    pub payload_hash: String,
    pub binding: String,
    pub expires_at_ms: u64,
    pub basis: Basis,
    pub state: AuthorizationState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub request_id: String,
    pub activity: String,
    pub authorization: String,
    pub adapter: String,
    pub operation: String,
    pub delivery: String,
    pub issued_at_ms: u64,
    pub not_after_ms: u64,
    pub reservation: Option<String>,
    pub reservation_released: bool,
    pub evidence: Evidence,
    pub outcome: Outcome,
    pub submissions: u32,
    pub re_dispatches: u32,
    pub reconciliations: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub entries: Vec<Entry>,
    pub activities: BTreeMap<String, Activity>,
    pub grants: BTreeMap<String, Grant>,
    pub proposals: BTreeMap<String, Proposal>,
    pub authorizations: BTreeMap<String, Authorization>,
    pub receipts: BTreeMap<String, Receipt>,
    next_authorization: u64,
}

impl Journal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one command as one step and returns its response. A refusal of a
    /// consequential step is itself journaled as a denial.
    pub fn apply(&mut self, source: Source, now_ms: u64, command: Command) -> Response {
        match command {
            Command::OpenActivity {
                activity,
                parent,
                approval_policy,
            } => self.open_activity(source, now_ms, activity, parent, approval_policy),
            Command::Transition { activity, to } => self.transition(source, now_ms, &activity, to),
            Command::Grant {
                grant_id,
                holder,
                activity,
                capability,
                object,
                parent,
                expires_at_ms,
            } => self.grant(
                source,
                now_ms,
                Grant {
                    id: grant_id,
                    holder,
                    activity,
                    capability,
                    object,
                    parent,
                    expires_at_ms,
                    revoked: false,
                    expired_at_close: false,
                },
            ),
            Command::Revoke { grant_id } => self.revoke(source, now_ms, &grant_id),
            Command::Cancel { activity } => self.cancel(source, now_ms, &activity),
            Command::Close { activity } => self.close(source, now_ms, &activity),
            Command::Denial {
                holder,
                operation,
                object,
                reason,
            } => {
                let seq = self.push(
                    source,
                    now_ms,
                    Kind::Denial {
                        holder,
                        operation,
                        object,
                        reason,
                    },
                );
                Response::Recorded { seq }
            }
            Command::Claim {
                holder,
                activity,
                statement,
            } => {
                let seq = self.push(
                    source,
                    now_ms,
                    Kind::Claim {
                        holder,
                        activity,
                        statement,
                    },
                );
                Response::Recorded { seq }
            }
            Command::Output {
                holder,
                activity,
                space,
                object,
                hash,
            } => {
                let seq = self.push(
                    source,
                    now_ms,
                    Kind::Output {
                        holder,
                        activity,
                        space,
                        object,
                        hash,
                    },
                );
                Response::Recorded { seq }
            }
            Command::Proposal {
                proposal_id,
                activity,
                holder,
                payload_hash,
            } => self.proposal(source, now_ms, proposal_id, activity, holder, payload_hash),
            Command::Approval {
                proposal_id,
                payload_hash,
            } => self.approval(source, now_ms, proposal_id, payload_hash),
            Command::MintAuthorization {
                activity,
                operation,
                target,
                adapter,
                payload_hash,
                expires_at_ms,
                grant_id,
                proposal_id,
            } => self.mint(
                source,
                now_ms,
                activity,
                operation,
                target,
                adapter,
                payload_hash,
                expires_at_ms,
                grant_id,
                proposal_id,
            ),
            Command::CheckLive {
                authorization,
                binding,
                delivery,
            } => {
                if let Some(earlier) = self.already_consumed(&authorization, &binding, &delivery) {
                    return earlier;
                }
                match self.liveness_refusal(now_ms, &authorization, &binding) {
                    Some(reason) => self.refuse(source, now_ms, &authorization, "enforcement check", reason),
                    None => Response::Live,
                }
            }
            Command::ConsumeIfLive {
                authorization,
                binding,
                delivery,
                validity_ms,
                reservation,
            } => self.consume_if_live(
                source,
                now_ms,
                authorization,
                binding,
                delivery,
                validity_ms,
                reservation,
            ),
            Command::Submission {
                request_id,
                delivery,
            } => self.submission(source, now_ms, request_id, delivery),
            Command::ReceiptUpdate { request_id, result } => {
                self.receipt_update(source, now_ms, request_id, result)
            }
            Command::ReDispatchIfLive {
                request_id,
                delivery,
                idempotent,
            } => self.re_dispatch_if_live(source, now_ms, request_id, delivery, idempotent),
            Command::Reconciliation {
                request_id,
                observation,
            } => self.reconciliation(source, now_ms, request_id, observation),
        }
    }

    fn push(&mut self, source: Source, at_ms: u64, kind: Kind) -> u64 {
        let seq = self.entries.len() as u64 + 1;
        self.entries.push(Entry {
            seq,
            at_ms,
            source,
            kind,
        });
        seq
    }

    fn refuse(
        &mut self,
        source: Source,
        now_ms: u64,
        object: &str,
        operation: &str,
        reason: String,
    ) -> Response {
        let holder = match &source {
            Source::Principal { id } | Source::Participant { id } => id.clone(),
            Source::Service { name } => name.clone(),
            Source::External { adapter, .. } => adapter.clone(),
        };
        let seq = self.push(
            Source::service("journal-writer"),
            now_ms,
            Kind::Denial {
                holder,
                operation: operation.into(),
                object: object.into(),
                reason: reason.clone(),
            },
        );

        Response::Refused {
            reason,
            seq: Some(seq),
        }
    }

    fn set_state(&mut self, source: &Source, now_ms: u64, activity: &str, to: ActivityState, reason: &str) {
        let Some(current) = self.activities.get(activity).map(|a| a.state) else {
            return;
        };
        if current == to {
            return;
        }

        self.activities.get_mut(activity).unwrap().state = to;
        self.push(
            source.clone(),
            now_ms,
            Kind::Transition {
                activity: activity.into(),
                from: current,
                to,
                reason: reason.into(),
            },
        );
    }

    /// The activity and every activity created inside it, parents first.
    pub fn descendants(&self, activity: &str) -> Vec<String> {
        let mut found = vec![activity.to_string()];
        let mut index = 0;

        while index < found.len() {
            let parent = found[index].clone();
            for child in self.activities.values() {
                if child.parent.as_deref() == Some(parent.as_str()) && !found.contains(&child.id) {
                    found.push(child.id.clone());
                }
            }
            index += 1;
        }

        found
    }

    /// A grant is live when neither it nor any grant it was derived from is
    /// revoked, expired, or ended by its activity's close.
    pub fn grant_live(&self, grant_id: &str, now_ms: u64) -> bool {
        let mut next = Some(grant_id.to_string());

        while let Some(id) = next {
            let Some(grant) = self.grants.get(&id) else {
                return false;
            };
            if grant.revoked || grant.expired_at_close {
                return false;
            }
            if grant.expires_at_ms.is_some_and(|expiry| expiry <= now_ms) {
                return false;
            }
            next = grant.parent.clone();
        }

        true
    }

    pub fn activity_open(&self, activity: &str) -> bool {
        self.activities.get(activity).is_some_and(|a| a.state.is_open())
    }

    fn open_activity(
        &mut self,
        source: Source,
        now_ms: u64,
        activity: String,
        parent: Option<String>,
        approval_policy: Vec<String>,
    ) -> Response {
        if self.activities.contains_key(&activity) {
            return self.refuse(source, now_ms, &activity, "open activity", "activity already exists".into());
        }
        if let Some(parent) = &parent {
            if !self.activity_open(parent) {
                return self.refuse(source, now_ms, &activity, "open activity", format!("parent {parent} is not open"));
            }
        }

        self.activities.insert(
            activity.clone(),
            Activity {
                id: activity.clone(),
                parent: parent.clone(),
                state: ActivityState::Created,
                approval_policy: approval_policy.clone(),
            },
        );
        let seq = self.push(
            source,
            now_ms,
            Kind::ActivityCreated {
                activity,
                parent,
                approval_policy,
            },
        );

        Response::Recorded { seq }
    }

    fn transition(&mut self, source: Source, now_ms: u64, activity: &str, to: ActivityState) -> Response {
        if !matches!(to, ActivityState::Running | ActivityState::Waiting) {
            return self.refuse(source, now_ms, activity, "transition", format!("{to:?} has its own command"));
        }
        if !self.activity_open(activity) {
            return self.refuse(source, now_ms, activity, "transition", "activity is not open".into());
        }

        self.set_state(&source, now_ms, activity, to, "participant progress");
        Response::Recorded {
            seq: self.entries.len() as u64,
        }
    }

    fn grant(&mut self, source: Source, now_ms: u64, grant: Grant) -> Response {
        if self.grants.contains_key(&grant.id) {
            return self.refuse(source, now_ms, &grant.id, "grant", "grant id already used".into());
        }
        if let Some(parent) = &grant.parent {
            if !self.grant_live(parent, now_ms) {
                return self.refuse(source, now_ms, &grant.id, "grant", format!("parent grant {parent} is not live"));
            }
        }
        let Some(state) = self.activities.get(&grant.activity).map(|a| a.state) else {
            return self.refuse(source, now_ms, &grant.id, "grant", "unknown activity".into());
        };
        if !(state.is_open() || state == ActivityState::Suspended) {
            return self.refuse(source, now_ms, &grant.id, "grant", "activity is not open".into());
        }

        let activity = grant.activity.clone();
        let seq = self.push(
            source.clone(),
            now_ms,
            Kind::Grant {
                grant_id: grant.id.clone(),
                holder: grant.holder.clone(),
                activity: grant.activity.clone(),
                capability: grant.capability.clone(),
                object: grant.object.clone(),
                parent: grant.parent.clone(),
                expires_at_ms: grant.expires_at_ms,
            },
        );
        self.grants.insert(grant.id.clone(), grant);

        // A new grant returns a suspended activity to Running. It never revives
        // an authorization that the revocation invalidated.
        if state == ActivityState::Suspended {
            self.set_state(&source, now_ms, &activity, ActivityState::Running, "re-granted");
        }

        Response::Recorded { seq }
    }

    fn revoke(&mut self, source: Source, now_ms: u64, grant_id: &str) -> Response {
        if !self.grants.contains_key(grant_id) {
            return self.refuse(source, now_ms, grant_id, "revoke", "unknown grant".into());
        }

        // Revoking a grant revokes everything delegated from it.
        let mut revoked = vec![grant_id.to_string()];
        let mut index = 0;
        while index < revoked.len() {
            let parent = revoked[index].clone();
            for grant in self.grants.values() {
                if grant.parent.as_deref() == Some(parent.as_str()) && !revoked.contains(&grant.id) {
                    revoked.push(grant.id.clone());
                }
            }
            index += 1;
        }

        for id in &revoked {
            self.grants.get_mut(id).unwrap().revoked = true;
        }

        let mut invalidated = Vec::new();
        for authorization in self.authorizations.values_mut() {
            let rests_on_revoked = revoked.iter().any(|id| id == authorization.basis.grant_id());
            if rests_on_revoked && authorization.state == AuthorizationState::Unconsumed {
                authorization.state = AuthorizationState::Invalidated {
                    reason: format!("grant {grant_id} revoked"),
                };
                invalidated.push(authorization.id.clone());
            }
        }

        let mut suspended: Vec<String> = Vec::new();
        for id in &revoked {
            let activity = self.grants[id].activity.clone();
            if self.activity_open(&activity) && !suspended.contains(&activity) {
                suspended.push(activity);
            }
        }

        let seq = self.push(
            source.clone(),
            now_ms,
            Kind::Revocation {
                grant_id: grant_id.into(),
                revoked,
                invalidated,
                suspended: suspended.clone(),
            },
        );
        for activity in suspended {
            self.set_state(&source, now_ms, &activity, ActivityState::Suspended, "revoked");
        }

        Response::Recorded { seq }
    }

    fn invalidate_unconsumed(&mut self, activities: &[String], reason: &str) -> Vec<String> {
        let mut invalidated = Vec::new();

        for authorization in self.authorizations.values_mut() {
            if activities.contains(&authorization.activity) && authorization.state == AuthorizationState::Unconsumed {
                authorization.state = AuthorizationState::Invalidated { reason: reason.into() };
                invalidated.push(authorization.id.clone());
            }
        }

        invalidated
    }

    fn cancel(&mut self, source: Source, now_ms: u64, activity: &str) -> Response {
        if !self.activities.contains_key(activity) {
            return self.refuse(source, now_ms, activity, "cancel", "unknown activity".into());
        }

        let closing: Vec<String> = self
            .descendants(activity)
            .into_iter()
            .filter(|id| {
                let state = self.activities[id].state;
                state.is_open() || state == ActivityState::Suspended
            })
            .collect();
        let invalidated = self.invalidate_unconsumed(&closing, &format!("activity {activity} cancelled"));

        let seq = self.push(
            source.clone(),
            now_ms,
            Kind::Cancellation {
                activity: activity.into(),
                closing: closing.clone(),
                invalidated,
            },
        );
        for id in closing {
            self.set_state(&source, now_ms, &id, ActivityState::Closing, "cancelled");
        }

        Response::Recorded { seq }
    }

    fn close(&mut self, source: Source, now_ms: u64, activity: &str) -> Response {
        let Some(state) = self.activities.get(activity).map(|a| a.state) else {
            return self.refuse(source, now_ms, activity, "close", "unknown activity".into());
        };
        if state == ActivityState::Closed {
            return self.refuse(source, now_ms, activity, "close", "activity already closed".into());
        }

        // A child activity closes no later than its parent.
        let closed: Vec<String> = self
            .descendants(activity)
            .into_iter()
            .filter(|id| self.activities[id].state != ActivityState::Closed)
            .collect();
        self.invalidate_unconsumed(&closed, &format!("activity {activity} closed"));

        let mut expired_grants = Vec::new();
        for grant in self.grants.values_mut() {
            if closed.contains(&grant.activity) && grant.expires_at_ms.is_some() && !grant.revoked {
                grant.expired_at_close = true;
                expired_grants.push(grant.id.clone());
            }
        }

        // Close never waits on, and never resolves, an external outcome.
        let unknown_outcomes: Vec<String> = self
            .receipts
            .values()
            .filter(|r| closed.contains(&r.activity) && !r.outcome.is_definitive())
            .map(|r| r.request_id.clone())
            .collect();

        for id in &closed {
            if self.activities[id].state.is_open() || self.activities[id].state == ActivityState::Suspended {
                self.set_state(&source, now_ms, id, ActivityState::Closing, "finished");
            }
        }
        let seq = self.push(
            source.clone(),
            now_ms,
            Kind::Close {
                activity: activity.into(),
                closed: closed.clone(),
                expired_grants,
                unknown_outcomes,
            },
        );
        for id in closed {
            self.set_state(&source, now_ms, &id, ActivityState::Closed, "closed");
        }

        Response::Recorded { seq }
    }

    fn proposal(
        &mut self,
        source: Source,
        now_ms: u64,
        proposal_id: String,
        activity: String,
        holder: String,
        payload_hash: String,
    ) -> Response {
        if self.proposals.contains_key(&proposal_id) {
            return self.refuse(source, now_ms, &proposal_id, "proposal", "proposal id already used".into());
        }
        if !self.activity_open(&activity) {
            return self.refuse(source, now_ms, &proposal_id, "proposal", "activity is not open".into());
        }

        self.proposals.insert(
            proposal_id.clone(),
            Proposal {
                id: proposal_id.clone(),
                activity: activity.clone(),
                holder: holder.clone(),
                payload_hash: payload_hash.clone(),
                approved_hash: None,
                authorized: false,
            },
        );
        let seq = self.push(
            source.clone(),
            now_ms,
            Kind::Proposal {
                proposal_id,
                activity: activity.clone(),
                holder,
                payload_hash,
            },
        );
        self.set_state(&source, now_ms, &activity, ActivityState::Waiting, "needs approval");

        Response::Recorded { seq }
    }

    fn approval(&mut self, source: Source, now_ms: u64, proposal_id: String, payload_hash: String) -> Response {
        let Some(proposal) = self.proposals.get(&proposal_id).cloned() else {
            return self.refuse(source, now_ms, &proposal_id, "approve", "unknown proposal".into());
        };
        if proposal.payload_hash != payload_hash {
            return self.refuse(
                source,
                now_ms,
                &proposal_id,
                "approve",
                "the approved hash differs from the proposal's payload".into(),
            );
        }
        if proposal.approved_hash.is_some() {
            return self.refuse(source, now_ms, &proposal_id, "approve", "proposal already approved".into());
        }
        if !self.activity_open(&proposal.activity) {
            return self.refuse(source, now_ms, &proposal_id, "approve", "activity is not open".into());
        }

        self.proposals.get_mut(&proposal_id).unwrap().approved_hash = Some(payload_hash.clone());
        let seq = self.push(
            source,
            now_ms,
            Kind::Approval {
                proposal_id,
                payload_hash,
            },
        );

        Response::Recorded { seq }
    }

    #[allow(clippy::too_many_arguments)]
    fn mint(
        &mut self,
        source: Source,
        now_ms: u64,
        activity: String,
        operation: String,
        target: String,
        adapter: String,
        payload_hash: String,
        expires_at_ms: u64,
        grant_id: String,
        proposal_id: Option<String>,
    ) -> Response {
        let object = format!("{activity}:{operation}");

        if !self.activity_open(&activity) {
            return self.refuse(source, now_ms, &object, "mint authorization", "activity is not open".into());
        }
        if !self.grant_live(&grant_id, now_ms) {
            return self.refuse(source, now_ms, &object, "mint authorization", format!("grant {grant_id} is not live"));
        }
        let grant = &self.grants[&grant_id];
        if grant.capability != operation || grant.activity != activity {
            return self.refuse(
                source,
                now_ms,
                &object,
                "mint authorization",
                format!("grant {grant_id} does not cover {operation} in {activity}"),
            );
        }
        // An approval adds to sufficient authority; it never stands in for it.
        if grant.object != target {
            return self.refuse(
                source,
                now_ms,
                &object,
                "mint authorization",
                format!("grant {grant_id} does not cover {target}"),
            );
        }
        if expires_at_ms <= now_ms {
            return self.refuse(source, now_ms, &object, "mint authorization", "expiry is in the past".into());
        }

        let needs_approval = self.activities[&activity].approval_policy.contains(&operation);
        let basis = match (needs_approval, proposal_id) {
            (false, None) => Basis::Grant { grant_id },
            (_, Some(proposal_id)) => {
                let Some(proposal) = self.proposals.get(&proposal_id) else {
                    return self.refuse(source, now_ms, &object, "mint authorization", "unknown proposal".into());
                };
                if proposal.activity != activity {
                    return self.refuse(source, now_ms, &object, "mint authorization", "proposal belongs to another activity".into());
                }
                if proposal.approved_hash.as_deref() != Some(payload_hash.as_str()) {
                    return self.refuse(source, now_ms, &object, "mint authorization", "no approval for this payload".into());
                }
                if proposal.authorized {
                    return self.refuse(source, now_ms, &object, "mint authorization", "approval already used".into());
                }
                self.proposals.get_mut(&proposal_id).unwrap().authorized = true;
                Basis::Approval { grant_id, proposal_id }
            }
            (true, None) => {
                return self.refuse(
                    source,
                    now_ms,
                    &object,
                    "mint authorization",
                    format!("the approval policy marks {operation} as needing fresh approval"),
                );
            }
        };

        self.next_authorization += 1;
        let authorization = format!("auth-{:04}", self.next_authorization);
        let binding = authorization_binding(&payload_hash, &operation, &adapter, &activity, expires_at_ms);

        self.authorizations.insert(
            authorization.clone(),
            Authorization {
                id: authorization.clone(),
                activity: activity.clone(),
                operation: operation.clone(),
                adapter: adapter.clone(),
                payload_hash: payload_hash.clone(),
                binding: binding.clone(),
                expires_at_ms,
                basis: basis.clone(),
                state: AuthorizationState::Unconsumed,
            },
        );
        let seq = self.push(
            source,
            now_ms,
            Kind::AuthorizationMinted {
                authorization: authorization.clone(),
                activity,
                operation,
                adapter,
                payload_hash,
                binding: binding.clone(),
                expires_at_ms,
                basis,
            },
        );

        Response::Minted {
            seq,
            authorization,
            binding,
        }
    }

    /// Why an authorization may not be used right now, if it may not.
    fn liveness_refusal(&self, now_ms: u64, authorization: &str, binding: &str) -> Option<String> {
        let Some(auth) = self.authorizations.get(authorization) else {
            return Some("unknown authorization".into());
        };
        if auth.binding != binding {
            return Some("the payload does not match the authorization's hash".into());
        }

        match &auth.state {
            AuthorizationState::Unconsumed => {}
            AuthorizationState::Consumed { .. } => return Some("authorization already consumed".into()),
            AuthorizationState::Invalidated { reason } => return Some(format!("authorization invalidated: {reason}")),
        }

        if auth.expires_at_ms <= now_ms {
            return Some("authorization expired".into());
        }
        if !self.activity_open(&auth.activity) {
            let state = self.activities.get(&auth.activity).map(|a| a.state);
            return Some(format!("activity {} is {state:?}", auth.activity));
        }
        if !self.grant_live(auth.basis.grant_id(), now_ms) {
            return Some(format!("grant {} is not live", auth.basis.grant_id()));
        }

        None
    }

    /// The same delivery resuming after a crash: report the earlier consumption
    /// so the adapter follows the recovery rule instead of dispatching.
    fn already_consumed(&self, authorization: &str, binding: &str, delivery: &str) -> Option<Response> {
        let auth = self.authorizations.get(authorization)?;
        let AuthorizationState::Consumed { request_id, delivery: by } = &auth.state else {
            return None;
        };
        if by != delivery || auth.binding != binding {
            return None;
        }

        let receipt = &self.receipts[request_id];
        Some(Response::AlreadyConsumed {
            request_id: request_id.clone(),
            issued_at_ms: receipt.issued_at_ms,
            not_after_ms: receipt.not_after_ms,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn consume_if_live(
        &mut self,
        source: Source,
        now_ms: u64,
        authorization: String,
        binding: String,
        delivery: String,
        validity_ms: u64,
        reservation: Option<String>,
    ) -> Response {
        if let Some(earlier) = self.already_consumed(&authorization, &binding, &delivery) {
            return earlier;
        }
        if let Some(reason) = self.liveness_refusal(now_ms, &authorization, &binding) {
            return self.refuse(source, now_ms, &authorization, "consume authorization", reason);
        }

        let auth = self.authorizations.get_mut(&authorization).unwrap();
        let request_id = request_id(&auth.activity, &auth.operation, &authorization);
        auth.state = AuthorizationState::Consumed {
            request_id: request_id.clone(),
            delivery: delivery.clone(),
        };
        let receipt = Receipt {
            request_id: request_id.clone(),
            activity: auth.activity.clone(),
            authorization: authorization.clone(),
            adapter: auth.adapter.clone(),
            operation: auth.operation.clone(),
            delivery: delivery.clone(),
            issued_at_ms: now_ms,
            not_after_ms: now_ms + validity_ms,
            reservation: reservation.clone(),
            reservation_released: false,
            evidence: Evidence::Observed,
            outcome: Outcome::Unknown,
            submissions: 0,
            re_dispatches: 0,
            reconciliations: 0,
        };
        self.receipts.insert(request_id.clone(), receipt);

        let seq = self.push(
            source,
            now_ms,
            Kind::Consumption {
                authorization,
                request_id: request_id.clone(),
                delivery,
                issued_at_ms: now_ms,
                not_after_ms: now_ms + validity_ms,
                reservation,
            },
        );

        Response::Consumed {
            seq,
            request_id,
            issued_at_ms: now_ms,
            not_after_ms: now_ms + validity_ms,
        }
    }

    fn own_receipt(&self, source: &Source, request_id: &str) -> Result<Receipt, String> {
        let Some(receipt) = self.receipts.get(request_id) else {
            return Err("unknown request identifier".into());
        };
        let adapter = match source {
            Source::External { adapter, .. } => adapter.as_str(),
            Source::Service { name } => name.as_str(),
            _ => return Err("only the issuing adapter may update this receipt".into()),
        };
        if adapter != receipt.adapter {
            return Err("not this adapter's request identifier".into());
        }

        Ok(receipt.clone())
    }

    fn submission(&mut self, source: Source, now_ms: u64, request_id: String, delivery: String) -> Response {
        let receipt = match self.own_receipt(&source, &request_id) {
            Ok(receipt) => receipt,
            Err(reason) => return self.refuse(source, now_ms, &request_id, "record submission", reason),
        };
        if receipt.delivery != delivery {
            return self.refuse(source, now_ms, &request_id, "record submission", "another delivery's request".into());
        }

        let receipt = self.receipts.get_mut(&request_id).unwrap();
        receipt.submissions += 1;
        let attempt = receipt.submissions;
        if !receipt.outcome.is_definitive() {
            receipt.outcome = Outcome::Submitted;
            receipt.evidence = Evidence::Observed;
        }
        let seq = self.push(
            source,
            now_ms,
            Kind::Submission {
                request_id,
                delivery,
                attempt,
            },
        );

        Response::Recorded { seq }
    }

    fn receipt_update(&mut self, source: Source, now_ms: u64, request_id: String, result: SubmitResult) -> Response {
        if let Err(reason) = self.own_receipt(&source, &request_id) {
            return self.refuse(source, now_ms, &request_id, "update receipt", reason);
        }

        let (evidence, outcome) = match result {
            SubmitResult::Accepted { message_id } => (Evidence::Acknowledged, Outcome::Acknowledged { message_id }),
            SubmitResult::Refused { reason } => (
                Evidence::Acknowledged,
                Outcome::Failed {
                    reason: FailureReason::Refused { reason },
                },
            ),
            SubmitResult::NoReply { .. } => (Evidence::Observed, Outcome::Unknown),
        };

        let receipt = self.receipts.get_mut(&request_id).unwrap();
        if receipt.outcome.is_definitive() {
            // A late reply never overwrites a definitive outcome; it is still recorded.
        } else {
            receipt.evidence = evidence;
            receipt.outcome = outcome.clone();
        }
        let seq = self.push(
            source,
            now_ms,
            Kind::ReceiptUpdate {
                request_id,
                evidence,
                outcome,
            },
        );

        Response::Recorded { seq }
    }

    fn re_dispatch_if_live(
        &mut self,
        source: Source,
        now_ms: u64,
        request_id: String,
        delivery: String,
        idempotent: bool,
    ) -> Response {
        let receipt = match self.own_receipt(&source, &request_id) {
            Ok(receipt) => receipt,
            Err(reason) => return self.refuse(source, now_ms, &request_id, "re-dispatch", reason),
        };

        let refusal = if !idempotent {
            Some("the contract does not declare the operation idempotent".to_string())
        } else if receipt.delivery != delivery {
            Some("another delivery's request".to_string())
        } else if receipt.outcome.is_definitive() {
            Some("the outcome is already definitive".to_string())
        } else if now_ms >= receipt.not_after_ms {
            Some("the request's validity window has closed".to_string())
        } else {
            let auth = &self.authorizations[&receipt.authorization];
            if now_ms >= auth.expires_at_ms {
                Some(format!("authorization {} has expired", auth.id))
            } else if !self.activity_open(&receipt.activity) {
                let state = self.activities.get(&receipt.activity).map(|a| a.state);
                Some(format!("activity {} is {state:?}", receipt.activity))
            } else if !self.grant_live(auth.basis.grant_id(), now_ms) {
                Some(format!("grant {} is not live", auth.basis.grant_id()))
            } else {
                None
            }
        };
        if let Some(reason) = refusal {
            return self.refuse(source, now_ms, &request_id, "re-dispatch", reason);
        }

        self.receipts.get_mut(&request_id).unwrap().re_dispatches += 1;
        let seq = self.push(source, now_ms, Kind::ReDispatch { request_id, delivery });

        Response::ReDispatchAllowed { seq }
    }

    fn reconciliation(
        &mut self,
        source: Source,
        now_ms: u64,
        request_id: String,
        observation: StatusObservation,
    ) -> Response {
        let receipt = match self.own_receipt(&source, &request_id) {
            Ok(receipt) => receipt,
            Err(reason) => return self.refuse(source, now_ms, &request_id, "reconcile", reason),
        };

        let resolved = match &observation {
            StatusObservation::Accepted { message_id } => Some(Resolution::Acknowledged {
                message_id: message_id.clone(),
            }),
            StatusObservation::Refused { reason } => Some(Resolution::Failed {
                reason: FailureReason::Refused { reason: reason.clone() },
            }),
            // A certificate counts only once the request's validity window
            // has closed by the journal's clock too.
            StatusObservation::NotReceived { certified: true } if now_ms >= receipt.not_after_ms => Some(Resolution::Failed {
                reason: FailureReason::CertifiedNotStarted,
            }),
            StatusObservation::NotReceived { .. } | StatusObservation::Unavailable { .. } => None,
        };

        // A status result never overwrites a definitive outcome.
        let outcome = match (receipt.outcome.is_definitive(), resolved) {
            (true, _) => receipt.outcome.clone(),
            (false, Some(resolution)) => Outcome::Reconciled {
                resolution,
                evidence: Evidence::Acknowledged,
            },
            (false, None) => Outcome::Unknown,
        };

        let receipt = self.receipts.get_mut(&request_id).unwrap();
        receipt.reconciliations += 1;
        receipt.outcome = outcome.clone();
        if let Outcome::Reconciled { evidence, .. } = &outcome {
            receipt.evidence = *evidence;
        }
        let release = match (&outcome, &receipt.reservation, receipt.reservation_released) {
            (
                Outcome::Reconciled {
                    resolution:
                        Resolution::Failed {
                            reason: FailureReason::CertifiedNotStarted,
                        },
                    ..
                },
                Some(reservation),
                false,
            ) => {
                receipt.reservation_released = true;
                Some(reservation.clone())
            }
            _ => None,
        };

        let seq = self.push(
            source,
            now_ms,
            Kind::Reconciliation {
                request_id: request_id.clone(),
                observation,
                outcome,
            },
        );

        // Only a certified non-start makes a retry safe and releases the budget.
        if let Some(reservation) = release {
            self.push(
                Source::service("journal-writer"),
                now_ms,
                Kind::ReservationReleased {
                    reservation,
                    request_id,
                },
            );
        }

        Response::Recorded { seq }
    }
}
