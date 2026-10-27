<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# 0002: The fake Diamond Dog

- Status: accepted
- Date: 2026-10-21
- Decided by: Jason

## Context

Orca Keel, a suite of household apps built in parallel, needs something to build against before the real runtime exists. The provisional contract (`interfaces/diamond-dog/provisional-contract.md`) asks for a fake that enforces six things from day one and passes eight scripted tests whose expected ledgers Orca Keel supplies. Jason decided on 2026-10-21 that Diamond Dog builds the fake and Orca Keel writes and runs the fixtures. The runtime language is not decided, and the fake is not the runtime.

## Decision

- **A local HTTP service.** One process on 127.0.0.1 answers `POST /<surface>/v<version>/<call>` with JSON. Orca Keel's code talks to it from any language, and the swap to the real runtime is a version bump and a client change, not a rewrite.
- **Written in Rust.** It reuses the receipt vocabulary of the Scenario A build (`experiment/shared/orca-ref`: sources, outcomes and evidence kept apart) and the mock mail contract, and matches the language recommended for the runtime. Orca Keel never links it, so its language does not reach their code.
- **Licensing.** The fake is runtime tooling, in `fake/` under GPL-3.0-or-later. The HTTP binding and the fixture format are interface definitions, in `interfaces/diamond-dog/` under MIT; the reference fixtures, household and manifest are examples, in `examples/diamond-dog-fake/` under MIT.
- **Fixtures as JSON.** A fixture is a script of calls with an expected ledger, run by `fake-diamond-dog check`, so Orca Keel writes them without touching Rust.
- **It says what it does not prove.** Nothing durable, nothing isolated, no identity checks, no spoof-proof approval path. The README leads with this.

## Alternatives considered

- **TypeScript, as a local service.** Easier for web developers to read; no reuse of the Scenario A code.
- **A library inside Orca Keel.** Quickest to wire up, but the swap to the real runtime would become a code change, and the fake would have to be written in Orca Keel's language.

## Consequences

- Orca Keel needs a Rust toolchain to build the fake, or a built binary from us. Publishing binaries is a release decision not yet taken.
- The HTTP binding is a second document to keep in step with the contract; the contract wins where they differ.
- The fake makes some readings where the contract is open; they are listed in `fake/README.md` for the contract thread to confirm or overrule.
