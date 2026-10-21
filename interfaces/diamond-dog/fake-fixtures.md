<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: MIT
-->

# Test fixtures for the fake Diamond Dog

Revision 2026-10-21. How to write the scripted scenarios the fake runs, each with a fixed expected ledger. Orca Keel owns the eight gating fixtures; Diamond Dog keeps reference versions of them in `examples/diamond-dog-fake/fixtures/`, which Orca Keel may copy, change or replace. When the two disagree about what the contract requires, the contract decides, and the side that is wrong fixes its file.

A fixture is one JSON file. The fake runs each in a fresh, empty fake:

```sh
fake-diamond-dog check path/to/fixtures/          # every .json in the folder
fake-diamond-dog check --ledger one-fixture.json  # also prints the ledger
```

A running fake also takes a fixture over HTTP at `POST /fake/check` and runs it in a separate, fresh fake, leaving its own state alone.

## Shape

```json
{
  "name": "Changed draft voids the approval",
  "about": "One or two sentences on what this proves.",
  "setup": { "now": "2026-11-02T14:00:00Z", "household": { ... }, "connections": [ ... ] },
  "steps": [
    { "call": "proposals/v0.3/propose",
      "body": { "caller": { "routine": "${a1}" }, "operation": "mail.send", "object": "mail:household-mail", "payload": { ... } },
      "save": { "p1": "proposal", "hash": "payload_hash" } },
    { "call": "ledger/v0.1/act",
      "body": { "caller": { "routine": "${a1}" }, "operation": "mail.send", "object": "mail:household-mail", "proposal": "${p1}", "payload": { ... } },
      "expect": { "refused": "payload_changed" } },
    { "control": "clock/advance", "body": { "hours": 2 } }
  ],
  "expect_ledger": [
    { "kind": "approval_voided", "proposal": "${p1}" },
    { "kind": "refusal", "operation": "mail.send", "reason": { "code": "payload_changed" } }
  ],
  "forbid_ledger": [ { "kind": "receipt" } ],
  "expect_mock": { "mail": { "household-mail": { "arrived_count": 0 } } }
}
```

| Field | Meaning |
| --- | --- |
| `setup` | The household, its connections and the mock services' content. Same body as the `setup` test control below |
| `steps` | In order. A `call` is `surface/vX.Y/call`, exactly as in the [HTTP binding](http-binding.md). A `control` is a test control |
| `expect` | `"ok"` (the default), `"refused"`, `{"refused": "<code>"}`, `"error"`, or `{"ok": {...}}`, which must be contained in the result. The first step that does not meet its expectation stops the run |
| `save` | Names for values from the result (or from the refusal, when refused), by dotted path: `"grant"`, `"grants.0"`, `"detail.provider"` |
| `${name}` | A saved value, anywhere in a later step or in the expectations below. `${name.path}` reaches inside it. A string that is only a reference takes the value's own type |
| `expect_ledger` | Ledger lines that must appear, in this order, each after the one before. Other lines may come between |
| `forbid_ledger` | Ledger lines that must not appear anywhere. `{}` matches every line, so `[{}]` asserts an empty ledger |
| `expect_mock` | Must be contained in the mock services' summary at the end (see below) |

"Contained" means every field given is present with an equal value; fields not given are not checked. Arrays must match element by element and have the same length. A `null` matches a field that is absent. So an expected ledger line names only what the test is about, usually `kind`, the ids, `operation`, `object`, `reason.code`, `outcome` and `evidence`. Never match on `plain`: its wording may improve.

Ids are given in order from 1 in a fresh fake (`A1`, `G1`, `P1`, `R1`, `S1`, `auth-1`), but save and reference them instead of writing them in, so a fixture survives a change in how many grants an earlier step draws.

## Test controls

Under `/fake/` over HTTP, or `control` in a fixture. They belong to the fake, not to the contract; the real runtime will not have them, and an app must never call them.

| Control | Body | Does |
| --- | --- | --- |
| `setup` | [`now`], `household`, [`connections`], [`providers`: `{name: {price_cents}}`], [`web`: pages], [`mail`: incoming mail] | Starts over with this household |
| `reset` | — | Starts over, empty |
| `now` | — | The fake's clock |
| `clock/set` | `at`, [`asleep`] | Moves the clock forward to `at` |
| `clock/advance` | [`minutes`], [`hours`], [`days`], [`asleep`] | Moves the clock forward |
| `mail/incoming` | `handle`, `from`, `subject`, `body` | Puts a mail in that mailbox |
| `mail/fault` | `handle`, `fault`, [`count`] | Arms a fault for the next submissions: `never_acknowledge` (accepted and delivered, reply lost), `refuse`, `drop_request` (never arrives), `status_unavailable` (the next status check gets no answer) |
| `web/page` | `host`, [`path`], `body`, [`redirect_to`] | Serves a page to the bounded browser |
| `provider` | `name`, `price_cents` | Sets a provider's price per disclosure |
| `ledger`, `mock`, `state` | — | The whole ledger; the mock services' summary; everything the fake holds |

Moving the clock awake runs every tick, expiry and timeout in between at its own time. With `asleep: true` the fake behaves as if it had not been running: of the ticks in between, the latest runs once, late, if it is inside its catch-up window, and the rest are journaled as skipped.

The household:

```json
{
  "zone": "America/New_York",
  "members": [
    { "id": "dana", "name": "Dana", "role": "owner", "may_approve": ["*"], "devices": ["dana-phone"] },
    { "id": "sam", "name": "Sam", "role": "adult", "may_approve": ["mail.send"], "devices": ["sam-phone"] },
    { "id": "maya", "name": "Maya", "role": "child" }
  ],
  "guardians": { "maya": ["dana"] },
  "locked": [],
  "approval_rules": []
}
```

The mock services' summary, which `expect_mock` is checked against:

```json
{
  "mail": { "household-mail": { "account": "...", "inbox_count": 1, "arrived_count": 0, "delivered_count": 0, "deliveries": [], "armed_faults": [] } },
  "calendar": { "<handle>": [ ... ] },
  "fetches": [ ... ], "fetch_count": 0,
  "providers": { "claude": { "price_cents": 2, "disclosure_count": 0, "disclosures": [] } },
  "lan": { "<target>": [ ... ] }
}
```

`arrived_count` counts submissions that reached the mail service; `delivered_count` counts messages it delivered. "Nothing was sent" is `arrived_count: 0`; "nothing was re-sent" is `delivered_count: 1` after a reconciliation.
