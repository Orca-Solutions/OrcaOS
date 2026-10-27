<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Diamond Dog, the runtime (experimental)

**Experimental skeleton.** The real runtime's language and architecture still wait on the experiment gate (the Golem build and its conventional baseline). This is where the enforcement rules get built and tested in the meantime, in Rust, against the same contract and fixtures as the fake. Nothing should depend on it yet; Orca Keel keeps building against [the fake](../fake/README.md).

It answers the ten surfaces of the [provisional contract](../interfaces/diamond-dog/provisional-contract.md) over the same [HTTP binding](../interfaces/diamond-dog/http-binding.md) as the fake, and passes the same fixtures: the eight gating tests and the scheduled-run scenario in `examples/diamond-dog-fake/fixtures/`, and the fake's regression fixtures in `fake/tests/fixtures/`. What is new is underneath: an enforcement core of its own.

## What the core does

| Rule (Decision log, 2026-10-20) | Where |
| --- | --- |
| The journal writer is the single point of order. Every ledger line is written and flushed to the journal file before the core changes anything, so a revocation takes effect when its record commits | `src/core/journal.rs`; every change in `src/core/mod.rs` journals first |
| The journal is append-only and hash-chained: each line carries the hash of the one before, and the runtime refuses to start on a journal with an edited, missing or reordered line. A final line cut off mid-write never committed and is discarded. If a line cannot be written, the runtime stops rather than act without a record | `Journal::open`, `Journal::append` |
| Authorization is per-action: for every consequential action the core mints a single-use authorization from a grant, or from an approval where the policy marks the operation, bound to the payload hash, operation, adapter, activity and expiry. It never leaves the core | `src/core/authority.rs`; `Core::mint` |
| Consume-if-live: one step checks that the authorization is unused and unexpired, its grant and every grant that one was drawn from are live, and the activity is open, then commits the consumption record with a fresh request identifier. An adapter can only dispatch with the `Consumed` this returns, which nothing else can make | `Core::consume_if_live`; `dispatch` in `src/surfaces/actions.rs` takes it by value |
| Revocation always lands, reaches every grant drawn from the one revoked, and suspends a running activity that held one | `Core::revoke` |
| The six activity states, and only the transitions the contract names. Activity and grant states are set only inside the core | `src/core/lifecycle.rs`; `Core::transition`, `Core::change_grant` |
| Durable work is not durable authority. On start, a consumption with no receipt becomes an unknown outcome (tracked, reconcilable, never re-sent), an approval not acted on is voided, and no grant is restored. Ids are never reused across a restart | `src/core/recovery.rs`; `Runtime::restore_receipts` |

The surfaces in `src/surfaces/` started as the fake's and call the core for everything above. The fixture runner and test controls are the fake's too, so a fixture means the same thing to both.

## What it does not do yet

- **Identity.** Like the fake, it trusts whoever reaches its port to be the member they claim, so it listens on 127.0.0.1 only. There is no spoof-proof approval path.
- **Isolation.** Routines are not yet Wasm participants; apps and routines are whatever calls the HTTP surface.
- **Real adapters.** Mail, calendar, web, providers and the printer are in-process mocks, as in the fake. A mock's memory does not survive a restart.
- **Carrying work across a restart.** The journal survives; the rest of the state (apps, records, schedules, activities) is not yet rebuilt from storage. So a restart closes open activities and says so in the journal, and the household is configured again with `--setup`. The Decision log's target is that activities persist and only authority does not.
- **More than one host.** The journal writer and the adapters run in one process, as the Decision log's first implementation assumes. Revocation across hosts is an open question.
- **Tamper-proofing.** The chain shows an edit; whoever can rewrite the whole file can rewrite the chain.

## Where it reads the contract differently from the fake

- **Suspended.** The contract names only Running to Suspended. The fake also suspends an activity that is Created or Waiting when a grant it holds is revoked; this runtime does not move it, and its next use of that grant is refused. A question for the contract thread.
- **Revoking a held grant.** Revoking an install grant that is held while its app is off revokes it, so switching the app on does not offer it again. The fake reports it as already stopped and leaves it held.

## Running it

It needs Rust 1.88 or later.

```sh
cd runtime
cargo test                                    # the core's tests, restart tests, and every fixture
cargo run -- check ../examples/diamond-dog-fake/fixtures ../fake/tests/fixtures

cargo run -- serve --setup ../examples/diamond-dog-fake/household.json
# Diamond Dog (experimental skeleton) listening on http://127.0.0.1:7478; journaling to disk; wall clock.

cargo run -- verify diamond-dog.journal       # checks a journal's chain without starting anything
```

`serve` journals to `diamond-dog.journal` in the working directory unless given `--journal <path>` or `--in-memory`, and follows the wall clock: scheduled runs, expiries and timeouts happen as they fall due. It listens on port 7478, so it can run beside the fake on 7477. `--test-controls` turns on the fixtures' test controls under `/fake/` and hands the clock to them; `setup` and `reset` then start over in memory, as fixtures expect. `GET /surfaces` answers `"runtime": "diamond-dog", "experimental": true`.
