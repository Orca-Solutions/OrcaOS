# 0001: Licensing

- Status: accepted
- Date: 2026-10-20
- Decided by: Jason

## Context

OrcaOS needs a license before anyone else can use, copy or modify it. Jason
wanted improvements to OrcaOS itself to stay open, while still letting
companies use and sell it. Components written for OrcaOS should not be forced
into a particular license, or developers will avoid the platform. The Rust
and Wasm ecosystem OrcaOS builds on is mostly MIT and Apache-2.0.

## Decision

- The runtime, system services, docs and tooling are licensed under
  GPL-3.0-or-later.
- What other software builds against is licensed under MIT: the component
  SDK (`sdk/`), contract and interface definitions (`interfaces/`) and
  examples (`examples/`).
- REUSE.toml maps paths to licenses, each MIT folder carries its own
  `LICENSE`, and new source files carry SPDX headers. `reuse lint` checks
  coverage.

## Alternatives considered

- **MIT everywhere.** Lowest friction for adopters, but a fork can become
  fully proprietary.
- **GPLv2-only, as Linux uses.** Rejected because Apache-2.0-only code cannot
  be combined with GPLv2-only code, and it has no explicit patent grant.
- **Separate repositories for the GPL and MIT parts.** Makes the split
  obvious, but costs coordinated changes across repos while the interfaces are
  still moving. One repository with a path map gives the same clarity; the SDK
  can move to its own repository later if it needs an independent release
  cycle.
- **AGPL.** Would also cover hosted use, but deters adopters.

## Consequences

- Anyone distributing a modified OrcaOS runtime must publish its source under
  the GPL, and devices shipping it must let owners install modified versions.
- Component authors can use any license, provided their components only use
  the MIT-licensed SDK and interfaces.
- Code never moves from a GPL path into an MIT path without every author's
  agreement. Once outside contributions arrive, relicensing needs their
  agreement or a contributor license agreement.
