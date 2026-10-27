<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: MIT
-->

# Diamond Dog provisional contract over HTTP

Revision 2026-10-21. The HTTP binding the fake Diamond Dog (`fake/` in this repository) answers today. It carries the surfaces of the [provisional contract](provisional-contract.md) at the versions that contract names on 2026-10-21, fourth pass: surfaces 1, 2, 3, 4, 6, 7, 9 and 10 at 0.3; 8 at 0.2; 5 at 0.1. The contract decides what each call means and when it is refused; this file decides only how a call travels. Where the two disagree, the contract wins and this file is wrong.

The real runtime may bind the contract differently. Code that keeps its HTTP calls behind one small client module will swap with a version bump and a new client, not a rewrite.

## Requests

Every call is `POST /<surface>/v<major.minor>/<call>` with a JSON body. Surface names: `install`, `activities`, `grants`, `proposals`, `ledger`, `records`, `egress`, `members`, `schedules`, `household`. `GET /surfaces` lists the version each surface answers.

A surface answers its own version and every older minor of the same major, because a minor bump only adds. Asking for a newer minor or another major is refused with `surface_version_unsupported`. An app's manifest pins the version it uses per surface, and install refuses a pin the runtime does not answer.

The body always names its caller, in one of three shapes:

| Caller | JSON | Who it is |
| --- | --- | --- |
| A member, through the shell | `{"shell": "dana", "device": "dana-phone"}` | A principal. Only this caller grants, approves, attests or switches apps. Approvals and attestations also need `device` to be one the member paired |
| An app, in a member's session | `{"app": "daybook", "member": "dana"}` | A tenant outside the runtime, acting while that member uses it. `member` may be left out only where no member is involved |
| A routine | `{"routine": "A3"}` | A participant inside activity A3. It acts for that activity's member and holds only that activity's grants |

Software is never a principal: no app or routine caller can grant, approve or attest, whatever it sends.

## Responses

| Status | Body | Meaning |
| --- | --- | --- |
| 200 | `{"ok": true, "result": {...}}` | Carried out as asked |
| 403 | `{"ok": false, "refused": {"code": "...", "reason": "...", "ledger_seq": 12}}` | Refused. Every refusal is journaled; `ledger_seq` is its line |
| 400 | `{"ok": false, "error": "..."}` | The request could not be read: bad JSON, a missing field, an unknown id. Not a refusal and not journaled |

Codes are stable snake_case words; the reason is plain language and may change. The codes in use: `activity_not_open`, `activity_suspended`, `already_closed`, `already_closing`, `already_installed`, `app_not_installed`, `app_off`, `approval_expired`, `approval_needed`, `audience_too_wide`, `beyond_member_authority`, `beyond_parent_authority`, `caller_not_principal`, `catch_up_too_long`, `category_locked`, `category_not_allowed`, `concurrency_key_held`, `cost_allowance_spent`, `device_not_paired`, `expiry_beyond_activity`, `grant_ended`, `grant_expired`, `grant_revoked`, `grant_used`, `hosted_model_rule`, `needs_proposal`, `no_adapter_contract`, `no_grant`, `no_grant_could_cover`, `not_a_member`, `not_adult`, `not_allowed_to_approve`, `not_owner`, `not_through_shell`, `not_writer_or_owner`, `notification_budget_spent`, `notification_kind_undeclared`, `object_outside_grant`, `object_outside_manifest`, `open_not_declared`, `operation_not_in_manifest`, `operation_not_standing`, `outside_audience`, `payload_changed`, `principal_may_not_grant`, `principal_may_not_use_app`, `proposal_not_pending`, `provider_not_declared`, `query_parameter_undeclared`, `read_scope_missing`, `record_type_undeclared`, `redirect_off_host`, `routine_not_in_manifest`, `routine_not_scheduled`, `second_settings_panel`, `spend_ceiling_reached`, `surface_version_unsupported`, `temporary_grants_live`, `trust_label_not_allowed`, `unknown_activity`, `unknown_member`, `work_state_changed`, `work_state_missing`.

## Operations and objects

An operation is `<adapter kind>.<verb>`; an object is `<adapter kind>:<handle, host, provider or target>`. A manifest's object pattern is exact, or ends in `*` for a prefix.

| Operation | Object | Kind |
| --- | --- | --- |
| `mail.read` | `mail:<handle>` | Read: proceeds on the grant alone |
| `mail.send` | `mail:<handle>` | Consequential: a single-use authorization is minted and consumed |
| `calendar.read` | `calendar:<handle>` | Read |
| `calendar.write` | `calendar:<handle>` | Consequential |
| `web.fetch` | `web:<host>` | Read, journaled as egress |
| `provider.disclose` | `provider:<name>` | Consequential, journaled as a disclosure |
| `lan.print` | `lan:<target>` | Consequential |

Keeping a record inside the household is not an operation (G-5); it is surface 6.

## The manifest

```json
{
  "app": "daybook", "name": "Daybook", "version": "0.1.0",
  "kind": "core",
  "surfaces": { "install": "0.3", "activities": "0.3", "ledger": "0.1" },
  "record_types": [ { "name": "health_note", "categories": ["health"] } ],
  "routines": [
    { "name": "morning", "trigger": "schedule", "timeout_minutes": 30,
      "retry": "none", "concurrency_key": "morning", "cost_allowance_cents": 50, "catch_up_hours": 6 }
  ],
  "operations": [
    { "operation": "mail.send", "adapter": "mail", "object": "mail:household-mail",
      "grant": "standing", "approval": "required", "idempotent": false }
  ],
  "providers": [ { "name": "claude", "memory_spans_activities": false } ],
  "disclosure_categories": ["general", "health"],
  "read_scopes": [ { "app": "lists", "record_types": ["item"] } ],
  "notifications": { "kinds": ["digest"], "per_day": 3 },
  "off_revokes": false,
  "opens": [ { "app": "lists", "routine": "groceries" } ],
  "opened_by": ["the-week"],
  "fetch_params": [ { "host": "api.recalls.example", "params": ["year", "make", "model"] } ]
}
```

`kind` is `core` (Daybook: the owner alone switches, pauses and grants for it, and its activities act only for the owner) or `add_on` (any adult member switches it; any member may be an activity's principal). A full example is in `examples/diamond-dog-fake/daybook-manifest.json`.

## Calls

Fields in brackets are optional. "Routine" means the call is made as `{"routine": ...}`, or as an app naming `activity` in the body.

### install (0.3)

| Call | Body | Result |
| --- | --- | --- |
| `install` | shell (owner); `manifest` | `app`, `on: false`, `read_scopes_granted` |
| `switch_on` | shell; `app`; [`standing_grants`: `[{operation, object, [expires_at]}]`]; [`confirm_held`: grant ids or `["all"]`] | `grants`, `held_for_confirmation`, `standing_operations` |
| `switch_off` | shell; `app` | Off: activities closed, pending proposals withdrawn, install grants held (revoked when `off_revokes`) |
| `uninstall` | shell (owner); `app` | Grants revoked; records and ledger kept |
| `read_app` | any; [`app`] | Manifest, on, paused, who switched it on, allowance |

### activities (0.3)

| Call | Body | Result |
| --- | --- | --- |
| `open` | app or routine; `routine`; [`app` to open another app's routine]; [`principal`]; [`audience`: member ids]; [`context`]; [`parent`]; [`approval_policy`: `{require: [operations]}`] | `activity`, `state`, `acts_for`, `audience`, `deadline` |
| `read_state` | anyone in the audience; `activity` | State, open proposals, live grants, unknown outcomes, cost |
| `cancel` | `activity` | Closing |
| `close` | `activity` | Closed; `unknown_outcomes_still_tracked` |
| `watch` | [`app`]; [`activities`]; [`states`, default `["closed"]`]; [`after`: ledger line] | `events` (the journal entries that changed state), `cursor` |

`principal` defaults to the caller's member; `audience` defaults to the principal alone, since a default never widens who sees work, and always includes the principal. A timed-out activity goes to Closing and then Closed on its own. A routine that opens an activity of its own app opens a child. Every new activity draws a child grant from each live install grant of its app, expiring with it. `watch` answers at once with what happened after `after`; pass back `cursor` next time.

### grants (0.3)

| Call | Body | Result |
| --- | --- | --- |
| `add_connection` | shell (adult); `handle`, `kind`, [`account`] | A connection handle |
| `grant` | shell; `holder`: `{activity}` or `{app}` (an install grant); `operation`, `object`; [`kind`: `standing` or `one_time`]; [`expires_at`]; one-time: `bound_to` (the work state) or `bound_hash` | `grant`, `bound_hash`, `expires_at` |
| `revoke` | shell, or the grant's own app giving it up; `grant` | Revoked, with everything drawn from it; a live activity holding it goes Suspended. Never refused for the grant's state |
| `list` | any; [`app`], [`activity`], [`object`] | Live grants in the caller's audience |
| `use` | routine; `operation`, `object`; [`grant`] | `items`, each with provenance and trust `untrusted` |

### proposals (0.3)

| Call | Body | Result |
| --- | --- | --- |
| `propose` | routine; `operation`, `object`, `payload`; [`target`]; [`matter`: another app's activity] | `proposal`, `payload_hash`, `needs_approval`, `approvers` |
| `show` | shell; `proposal` | What the paired device shows, and the hash it binds |
| `approve` | shell on a paired device; `proposal`, `payload_hash` as shown | Approved. The authorization goes to the adapter, never to the app |
| `decline` | shell on a paired device; `proposal` | Declined |
| `read_policy` | any; `activity` | Per operation: whether it needs approval, and who may give it |
| `list_pending` | the member, or an app or routine acting for them; `member` | `count`, `pending` with app, activity and age |
| `set_rule` | shell (owner); `rule`: `{operation, [when_audience], approvers}` | Household approval rules. `approvers` is `{"members": [...]}`, `{"role": "owner"}` or `"record_audience"`; `when_audience` matches the record named by the activity's `context.record` |

### ledger (0.1)

| Call | Body | Result |
| --- | --- | --- |
| `act` | routine; `operation` (`mail.send`, `calendar.write`, `lan.print`), `object`, `payload`; [`proposal`] when the policy needs approval; [`grant`] | The receipt: `request_id`, `outcome`, `evidence`, `authority` |
| `reconcile` | `request_id` | The receipt after a status check. Never re-sends |
| `read` | [`activity`], [`app`], [`member`], [`object`], [`since`], [`until`], [`limit`] | `entries`, newest first, inside the caller's audience |
| `claim` | routine; `statement` | A ledger line with evidence `claimed` |
| `attest` | shell on a paired device; `activity`, `statement` | A ledger line with evidence `attested` |

### records (0.3)

| Call | Body | Result |
| --- | --- | --- |
| `write` | app (with member) or routine; `type`, `content`, `provenance`: `{source, [time], [relayed_by], trust}`, `audience`; [`client_key`]; [`object` to write a new version] | `object`, `version`; `repeat: true` when the client key was seen |
| `read` | `object`, or a query by [`type`] and [`app`] | `records` the reader may read |
| `promote` | `object`, `audience` | The audience widened inside the household, journaled; no grant |
| `delete` | the writing app or the owner; `object` | Content gone; id and type kept |
| `allow_app_read` | shell; `app` | That app's runs may read records in this member's audience (G-2) |

`audience` is `"household"`, `{"members": [...]}` or `{"guardians_of": "<child>"}`. Trust is `untrusted` for anything relayed from outside, `member` only for a member's own words, and never `service` from an app. A routine's audience may not reach beyond its activity's.

### egress (0.3)

| Call | Body | Result |
| --- | --- | --- |
| `fetch` | routine; `host`, [`path`]; [`api`: true for an API request (G-17)]; [`query`: `{name: value}`] | `status`, `content_type`, `body` (a page through the bounded browser, or the raw body through the fetch adapter), provenance `untrusted` |
| `disclose` | routine; `provider`, `categories`, `content`, [`records`] | The receipt, naming provider, categories, conversation and grant |
| `loosen` | shell; `activity`, `categories` | Those categories may leave for that activity only |
| `lock_category` | shell (owner); `category`, [`unlock`] | Locked for every matter |
| `lan` | routine; `target`, `job` | The receipt |

Every `query` parameter must be one the manifest's `fetch_params` declares for that host, page or API alike; any other is refused with `query_parameter_undeclared` and journaled, so "never the VIN" is enforced. A disclosure's categories are the ones named plus every category the named records' types carry. The fake does not read free text.

### members (0.2)

| Call | Body | Result |
| --- | --- | --- |
| `list` | any member | Members, roles, approvals, devices, guardians |
| `pair_device` | shell; `device` | Paired |
| `set_guardians` | shell (owner); `child`, `guardians` (empty removes) | The relation |
| `list_guardians` | any member | The relations |

### schedules (0.3)

| Call | Body | Result |
| --- | --- | --- |
| `schedule` | app or shell; `routine`, `at` (`HH:MM`), `cadence` (`daily` or `weekly:sun`), [`zone`, default the household's], [`catch_up_hours`] | `schedule`, `next`, `catch_up_hours` |
| `list` | [`app`] | Schedules |
| `unschedule` | `schedule` | Removed |

### household (0.3)

| Call | Body | Result |
| --- | --- | --- |
| `notify` | routine; `member`, `kind` | A content-free push. `live_notice` is outside the budget |
| `list_notifications` | shell | The member's pushes |
| `pause_app` | shell; `app` | As switch off, journaled as a pause |
| `set_spend_allowance` | shell (owner); `app`, `ceiling_cents`, [`period`: `day`, `week`, `month`], [`paused`] | The allowance |
| `grant_support` | shell; `support`, [`minutes`], `operations` (`ledger.read`, `records.read`) | Time-boxed grants |
| `contribute` | app or routine; `kind` (`matter`, `question`, `review`, `notice`, `offer`, `settings_panel`), `content`, [`audience`], [`activity`] | `item` |
| `list_contributions` | any | Items in the caller's audience |
| `answer` | shell; `item`, `response` | A record with source principal, never a grant |
| `wipe` | shell (owner) | Everything gone, with no ledger line after |

## The ledger line

```json
{
  "seq": 13, "at": "2026-11-02T14:00:00Z", "local": "2026-11-02 09:00 EST",
  "kind": "receipt",
  "source": { "kind": "external", "system": "mail:family@example.test", "adapter": "mail adapter (household-mail)" },
  "app": "daybook", "activity": "A1", "member": "dana",
  "operation": "mail.send", "object": "mail:household-mail", "request_id": "A1:mail.send:auth-1",
  "outcome": { "state": "acknowledged", "message_id": "msg-1" },
  "evidence": "acknowledged",
  "authority": { "approval": "P1", "member": "dana" },
  "plain": "Daybook's mail.send on mail:household-mail for Dana, approved by Dana (P1): the other side acknowledged it."
}
```

Fields that do not apply are left out. Hashes (`payload_hash`, `bound_hash`) are `sha256:` and the hex SHA-256 of the JSON value with object keys sorted, so the order a client sends keys in never changes them. `source.kind` is `principal`, `participant`, `service` or `external`. `outcome.state` is `submitted`, `acknowledged`, `unknown`, `failed` (with `why`: `refused` or `certified_not_started`) or `reconciled` (with `to` and `evidence`). `evidence` is `claimed`, `attested`, `observed` or `acknowledged`. A refusal carries `reason: {code, text}`. `visible_to` lists members who may read a line beyond the usual audience: the guardians, on every read of their child's record.

Kinds in use: `install`, `switch_on`, `switch_off`, `pause`, `uninstall`, `connection`, `activity`, `grant`, `grant_held`, `grant_confirmed`, `grant_ended`, `revocation`, `read`, `fetch`, `proposal`, `approval`, `authorization`, `approval_voided`, `decline`, `withdrawal`, `approval_rule`, `consumption`, `receipt`, `disclosure`, `status_check`, `reconciliation`, `claim`, `attestation`, `record`, `record_read`, `promotion`, `deletion`, `read_allowed`, `loosened`, `category_lock`, `pairing`, `guardians`, `schedule`, `tick`, `notification`, `allowance`, `contribution`, `answer`, `refusal`.
