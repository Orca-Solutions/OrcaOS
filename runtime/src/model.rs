// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The runtime's state outside the enforcement core, named as the Glossary
//! and the provisional contract name it.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The surfaces the runtime answers, each with the version it implements.
/// A request for an older minor version of the same major is answered too:
/// a minor bump only adds.
pub const SURFACES: [(&str, &str); 10] = [
    ("install", "0.3"),
    ("activities", "0.3"),
    ("grants", "0.3"),
    ("proposals", "0.3"),
    ("ledger", "0.1"),
    ("records", "0.3"),
    ("egress", "0.3"),
    ("members", "0.2"),
    ("schedules", "0.3"),
    ("household", "0.3"),
];

/// Categories that never leave for a provider unless a member loosened them
/// for the matter at hand.
pub const SENSITIVE: [&str; 3] = ["health", "children", "money"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Adult,
    Child,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub role: Role,
    /// Operations this member may approve; `*` for all.
    #[serde(default)]
    pub may_approve: Vec<String>,
    #[serde(default)]
    pub devices: BTreeSet<String>,
}

impl Member {
    pub fn may_approve(&self, operation: &str) -> bool {
        self.may_approve.iter().any(|op| op == "*" || op == operation)
    }
}

fn default_zone() -> String {
    "UTC".into()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Household {
    #[serde(default = "default_zone")]
    pub zone: String,
    #[serde(default)]
    pub members: Vec<Member>,
    /// Child member id to the members who are its guardians.
    #[serde(default)]
    pub guardians: BTreeMap<String, Vec<String>>,
    /// Disclosure categories no member may loosen for any matter.
    #[serde(default)]
    pub locked: BTreeSet<String>,
    #[serde(default)]
    pub approval_rules: Vec<ApprovalRule>,
}

/// Who may approve an operation, optionally only when the record in the
/// activity's context has a given audience (G-15).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRule {
    pub operation: String,
    #[serde(default)]
    pub when_audience: Option<Audience>,
    pub approvers: Approvers,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Approvers {
    /// `"record_audience"`: the members in the context record's audience.
    RecordAudience(String),
    Members { members: Vec<String> },
    Role { role: Role },
}

impl Household {
    pub fn member(&self, id: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.id == id)
    }

    pub fn member_mut(&mut self, id: &str) -> Option<&mut Member> {
        self.members.iter_mut().find(|m| m.id == id)
    }

    pub fn owner(&self) -> Option<&Member> {
        self.members.iter().find(|m| m.role == Role::Owner)
    }
}

/// A connection handle: an adapter the household set up in the shell. Apps
/// refer to it by handle and never see the credential.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub handle: String,
    pub kind: String,
    #[serde(default)]
    pub account: String,
}

// ---- The manifest ----

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    /// Daybook: only the owner switches it, pauses it or grants for it.
    Core,
    #[default]
    AddOn,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub app: String,
    #[serde(default)]
    pub name: Option<String>,
    pub version: String,
    #[serde(default)]
    pub kind: AppKind,
    /// The surface versions the app pins.
    #[serde(default)]
    pub surfaces: BTreeMap<String, String>,
    #[serde(default)]
    pub record_types: Vec<RecordType>,
    #[serde(default)]
    pub routines: Vec<RoutineDecl>,
    #[serde(default)]
    pub operations: Vec<OperationDecl>,
    #[serde(default)]
    pub providers: Vec<ProviderDecl>,
    #[serde(default)]
    pub disclosure_categories: Vec<String>,
    #[serde(default)]
    pub read_scopes: Vec<ReadScope>,
    #[serde(default)]
    pub notifications: NotificationBudget,
    /// Switching the app off revokes its standing grants instead of holding them.
    #[serde(default)]
    pub off_revokes: bool,
    /// Other apps' routines this app may open.
    #[serde(default)]
    pub opens: Vec<RoutineRef>,
    /// Apps that may open this app's routines.
    #[serde(default)]
    pub opened_by: Vec<String>,
    /// The query parameters the app may send to each host it fetches from (G-17).
    #[serde(default)]
    pub fetch_params: Vec<FetchParams>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchParams {
    pub host: String,
    pub params: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutineRef {
    pub app: String,
    pub routine: String,
}

impl Manifest {
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.app.clone())
    }

    pub fn operation(&self, operation: &str) -> Option<&OperationDecl> {
        self.operations.iter().find(|o| o.operation == operation)
    }

    pub fn routine(&self, name: &str) -> Option<&RoutineDecl> {
        self.routines.iter().find(|r| r.name == name)
    }

    pub fn record_type(&self, name: &str) -> Option<&RecordType> {
        self.record_types.iter().find(|t| t.name == name)
    }

    pub fn fetch_params(&self, host: &str) -> Vec<String> {
        self.fetch_params.iter().filter(|f| f.host == host).flat_map(|f| f.params.clone()).collect()
    }

    pub fn may_read(&self, app: &str, record_type: &str) -> bool {
        self.read_scopes
            .iter()
            .any(|s| s.app == app && s.record_types.iter().any(|t| t == record_type || t == "*"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordType {
    pub name: String,
    /// Disclosure categories content of this type always carries.
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    #[default]
    Manual,
    Schedule,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retry {
    #[default]
    None,
    IdempotentOnly,
}

fn default_timeout() -> i64 {
    60
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutineDecl {
    pub name: String,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default = "default_timeout")]
    pub timeout_minutes: i64,
    #[serde(default)]
    pub retry: Retry,
    #[serde(default)]
    pub concurrency_key: Option<String>,
    #[serde(default)]
    pub cost_allowance_cents: Option<u64>,
    /// Shortens the catch-up window; never longer than the cadence.
    #[serde(default)]
    pub catch_up_hours: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantKind {
    #[default]
    Standing,
    OneTime,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDefault {
    #[default]
    None,
    Required,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationDecl {
    pub operation: String,
    pub adapter: String,
    /// The object pattern: exact, or ending in `*` for a prefix.
    pub object: String,
    #[serde(default)]
    pub grant: GrantKind,
    #[serde(default)]
    pub approval: ApprovalDefault,
    #[serde(default)]
    pub idempotent: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderDecl {
    pub name: String,
    #[serde(default)]
    pub memory_spans_activities: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadScope {
    pub app: String,
    pub record_types: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NotificationBudget {
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub per_day: u32,
}

/// The adapter contracts the runtime implements: operation, adapter kind, and
/// whether it is a read (proceeds on the grant alone) or a consequential
/// action (a single-use authorization is minted and consumed for it).
pub const OPERATIONS: [(&str, &str, bool); 7] = [
    ("mail.read", "mail", false),
    ("mail.send", "mail", true),
    ("calendar.read", "calendar", false),
    ("calendar.write", "calendar", true),
    ("web.fetch", "web", false),
    ("provider.disclose", "provider", true),
    ("lan.print", "lan", true),
];

pub fn adapter_of(operation: &str) -> Option<(&'static str, bool)> {
    OPERATIONS
        .iter()
        .find(|(op, _, _)| *op == operation)
        .map(|(_, adapter, consequential)| (*adapter, *consequential))
}

/// Exact match, or a prefix match when the pattern ends in `*`.
pub fn object_matches(pattern: &str, object: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => object.starts_with(prefix),
        None => pattern == object,
    }
}

// ---- Installed apps ----

#[derive(Clone, Debug, Serialize)]
pub struct Allowance {
    pub ceiling_cents: u64,
    pub period: String,
    pub paused: bool,
    pub spent_cents: u64,
    pub period_start: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct App {
    pub manifest: Manifest,
    pub installed: bool,
    pub on: bool,
    pub paused: bool,
    pub switched_on_by: Option<String>,
    pub allowance: Option<Allowance>,
}

// ---- Activities and grants live in the enforcement core ----

pub use crate::core::authority::{Grant, GrantState, Holder};
pub use crate::core::lifecycle::{Activity, ActivityState};

// ---- Proposals ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Pending,
    Approved,
    Declined,
    Withdrawn,
    /// The approved payload changed before the action; a new proposal is needed.
    Voided,
}

#[derive(Clone, Debug, Serialize)]
pub struct Proposal {
    pub id: String,
    pub activity: String,
    /// The app that proposed.
    pub app: String,
    /// The app whose matter this is, when a routine proposes on a matter it did not open.
    pub matter_app: Option<String>,
    pub matter: Option<String>,
    pub operation: String,
    pub object: String,
    pub target: Value,
    pub payload: Value,
    pub payload_hash: String,
    pub state: ProposalState,
    pub created_at: DateTime<Utc>,
    pub decided_by: Option<String>,
    pub authorization: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Receipt {
    pub request_id: String,
    pub activity: String,
    pub app: String,
    pub member: String,
    pub operation: String,
    pub object: String,
    pub authority: Value,
    pub outcome: Value,
    pub evidence: String,
    pub detail: Value,
}

impl Receipt {
    pub fn is_definitive(&self) -> bool {
        !matches!(self.outcome["state"].as_str(), Some("unknown" | "submitted"))
    }
}

// ---- Records ----

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Audience {
    Everyone(String),
    Members { members: Vec<String> },
    Guardians { guardians_of: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub id: String,
    pub app: String,
    pub record_type: String,
    pub version: u64,
    pub content: Value,
    pub provenance: Value,
    pub trust: String,
    pub audience: Audience,
    /// The member whose authority the write was made on.
    pub written_by: String,
    pub at: DateTime<Utc>,
    pub client_key: Option<String>,
    /// Deleted: the content is gone; the id and type stay for receipts.
    pub deleted: bool,
}

// ---- Scheduled runs ----

#[derive(Clone, Debug, Serialize)]
pub struct Schedule {
    pub id: String,
    pub app: String,
    pub routine: String,
    /// Local wall-clock time, `HH:MM`.
    pub at: String,
    pub zone: String,
    /// `daily`, or `weekly:<mon|tue|...>`.
    pub cadence: String,
    pub catch_up_hours: i64,
    pub set_by: String,
}

/// An item an app contributes to the shell. It carries no authority.
#[derive(Clone, Debug, Serialize)]
pub struct Contribution {
    pub id: String,
    pub app: String,
    pub kind: String,
    pub content: Value,
    pub audience: BTreeSet<String>,
    pub activity: Option<String>,
    pub at: DateTime<Utc>,
    pub answered_by: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Notification {
    pub id: String,
    pub member: String,
    pub app: String,
    pub activity: String,
    pub kind: String,
    pub at: DateTime<Utc>,
}
