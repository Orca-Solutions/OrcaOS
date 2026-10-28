<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# 0003: An experimental runtime skeleton beside the fake

- Status: proposed, experimental (accepted when Jason merges it; the experiment gate can still replace it)
- Date: 2026-10-27
- Asked for by: Jason ("push forward on development, as many fronts as we can")

## Context

The fake (ADR 0002) lets Orca Keel build against the provisional contract, but it is a test double: everything in memory, any state change allowed, the ledger a plain list. The real runtime's language and architecture wait on the experiment gate, which the Golem build and the conventional baseline are working toward. Meanwhile the Decision log's enforcement rules (per-action single-use authorizations, consume-if-live at the journal writer, revocation ordering, durable work without durable authority) exist only on paper and in the Golem build's reference code.

## Decision

- **Start the runtime now, marked experimental,** in `runtime/` (GPL-3.0-or-later), as the crate `diamond-dog`. It answers the same contract over the same HTTP binding and must pass the same fixtures as the fake, so the two stay interchangeable for Orca Keel.
- **Rust,** as recommended for the runtime and already used by the fake. The gate can still change it; the fixtures are JSON and would carry over.
- **A separate enforcement core** (`runtime/src/core/`) that owns the journal, activities, grants and authorizations, keeps their states private, and journals before it changes anything. Consume-if-live is one call on it, and the proof of consumption is a type only it can make, which an adapter needs to dispatch.
- **A durable journal from the start:** one JSON line per entry, hash-chained, flushed per line, verified on open, fail-stop when it cannot be written. Restart applies the Decision log's rules to it rather than rebuilding authority.
- **Copy, then diverge.** The surfaces, fixture runner, test controls and mocks start as copies of the fake's and are rewired onto the core. A shared crate would couple the fake Orca Keel depends on to a runtime that is meant to change; once the two have diverged enough to show what is truly shared, that can be extracted.

## Alternatives considered

- **Wait for the gate.** Keeps effort low, but leaves the enforcement rules untested in code outside Golem, and gives the gate nothing of our own to compare.
- **Turn the fake into the runtime.** One codebase, but every runtime change would risk the stand-in Orca Keel builds against today.
- **Build on Golem directly.** That is the experiment itself (PR #5); its result decides whether a separate runtime is justified at all.

## Consequences

- Two implementations of the surfaces to keep in step with the contract until one is retired. The shared fixtures are what keeps them honest; CI runs them for both.
- The skeleton does not yet rebuild work state after a restart, check identity, isolate participants or talk to real services; `runtime/README.md` says so first.
- If the gate concludes no separate runtime is needed for the tested semantics, this crate becomes the reference for an interface or integration layer instead, or is retired.
