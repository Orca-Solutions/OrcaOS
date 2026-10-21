<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# The fake Diamond Dog

A local stand-in for Diamond Dog that Orca Keel's apps and routines can be built against while the real runtime does not exist. It answers the ten surfaces of the [provisional contract](../interfaces/diamond-dog/provisional-contract.md) over HTTP, refuses what the contract says to refuse, and writes the ledger in the format meant to be kept.

## What it does not prove

It is a test double, not a runtime. It keeps everything in memory and forgets it when it stops. It runs nothing isolated: apps are ordinary programs on the same machine, and nothing stops them from reaching the network or the disk directly. It checks no identity: whoever can reach its port can claim to be any member, which is why it listens on 127.0.0.1 only. And it cannot show that an approval screen is genuine.

So passing against the fake shows that an app asks for the right things in the right order and copes with the refusals and outcomes it will meet. It says nothing about isolation, durability across crashes or a spoof-proof approval path. Those are the real runtime's job, and the experiment gate decides what that runtime is.

## Running it

It needs Rust 1.88 or later (`rustup` installs it).

```sh
cd fake
cargo run -- serve --setup ../examples/diamond-dog-fake/household.json
# fake Diamond Dog listening on http://127.0.0.1:7477

cargo run -- check ../examples/diamond-dog-fake/fixtures   # the eight scenarios, plus one on scheduled runs
cargo test                                                  # those, the regressions in tests/fixtures, and unit tests
```

A first call, installing Daybook as the owner:

```sh
curl -s -X POST http://127.0.0.1:7477/install/v0.3/install \
  -d "{\"caller\": {\"shell\": \"dana\", \"device\": \"dana-phone\"}, \"manifest\": $(cat ../examples/diamond-dog-fake/daybook-manifest.json)}"
```

How calls travel is in [interfaces/diamond-dog/http-binding.md](../interfaces/diamond-dog/http-binding.md); how to write a test scenario, and the test controls that set up a household, move the clock and arm the mock services' faults, is in [interfaces/diamond-dog/fake-fixtures.md](../interfaces/diamond-dog/fake-fixtures.md).

## The six enforcements, and where they live

| Contract | Code |
| --- | --- |
| The manifest is enforced: an operation the app did not declare is refused and journaled | `surfaces/install.rs`; the checks in `surfaces/actions.rs` (`action_base`) and `surfaces/grants.rs` (`check_grant`) |
| Grants bound to work state: change the draft, the record or the recipient and the enforcement check refuses | `enforce` in `surfaces/actions.rs` |
| Relayed content is labelled and cannot create a grant, an approval or an operation | Trust labels in `surfaces/records.rs`; only a shell caller grants or approves (`surfaces/grants.rs`, `surfaces/proposals.rs`) |
| A ledger in the format meant to be kept, renderable in plain language | `ledger.rs`; every refusal journaled in `Fake::call` (`fake.rs`) |
| Scheduled runs at local wall-clock time with a catch-up window | `schedule.rs`, `advance_to` in `fake.rs`, `surfaces/schedules.rs` |
| Every surface versioned | `SURFACES` in `model.rs`; checked per call in `fake.rs` |

The outside systems (mail, calendar, web hosts, model providers, a printer) are mocks in `mock.rs`, in the same process. The mail mock follows the Scenario A mock mail service's contract: a request identifier on every submission, a status check by that identifier, and the same faults.

## Where the fake reads the contract

The contract thread confirmed or settled each of these on 2026-10-21 (contract commit c09c154); they are now in the contract itself:

- **An activity's audience** is the principal alone when `open` names none.
- **Install grants** are drawn on by every activity of the app, scheduled or not: each activity gets a child grant per live install grant, expiring with it. An activity uses only the grants it holds.
- **Approval alone** mints the authorization for an operation the policy marks; no grant is needed beside it, and the manifest is still the ceiling.
- **The catch-up window** may be set up to the cadence in the manifest; a schedule may only shorten it.
- **Timeouts** move an activity to Closing and then, once its temporary grants have ended, to Closed on its own.
- **Disclosure categories** are what the routine names plus what the sent records' types carry. The fake does not read free text.
- **Revoke** is open to members through the shell and to an app giving up its own grants; other callers are refused and journaled.

Readings the contract leaves to the binding, not the contract:

- **Malformed requests** (bad JSON, a missing field, an unknown id) get a 400 error and are not journaled; refusals get a 403 and always are.
- **Support sessions** are recorded as time-boxed grants for `ledger.read` and `records.read`; the fake runs no session.
