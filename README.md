# Diamond Dog

**Delegate work, not your identity.**

Diamond Dog is a delegation-first operating environment: an open-source layer on Linux for giving software a specific job. Each job states what it may read, what it may change and what needs your sign-off, and leaves a record of what happened, including the parts nobody could confirm. Apps, scripts and AI participants follow the same rules; AI gets no special status.

A little less blind faith, in other words.

The repository is still named OrcaOS, the project's earlier working name. Folder names, crate names and license headers keep that name until a rename is decided. Project site: [diamonddog.orcasolutions.dev](https://diamonddog.orcasolutions.dev).

## Where it stands

Diamond Dog is in development. The design is written down, a first experiment has run, and nothing has shipped. Nothing in this repository is a runtime to rely on yet. Current scope, decisions and recorded checks are in [WORKING_RECORD.md](WORKING_RECORD.md).

**Built, on `main`:**

| What | Where | What it is, and what it is not |
| --- | --- | --- |
| Provisional contract | [interfaces/diamond-dog/](interfaces/diamond-dog/) | The ten surfaces that products built on top (first, Orca Keel) pin against, each versioned on its own, plus an HTTP binding and a fixture format. Provisional until the experiment's results are in. |
| The fake Diamond Dog | [fake/](fake/README.md) | A local Rust test double that answers the contract over HTTP, refuses what the contract says to refuse and writes the ledger. CI runs its fixtures and lints on every change. It keeps nothing durable, isolates nothing and checks no identity. |
| Scenario A on Golem | [experiment/](experiment/README.md) | The first experiment: one bounded scenario (three contracts, one message, your sign-off) built on a restricted Golem 1.6.0. It passed the seven acceptance tests on 2026-10-20. That is a conformance result, not a security claim. |
| Sensor observation interface 0.1 | [interfaces/sensor-observation/](interfaces/sensor-observation/README.md) | A draft of how a small device publishes readings and coverage. No implementation has shipped. |

**In progress, not on `main`:**

- A conventional baseline for Scenario A, the comparison the Golem build is measured against.
- An experimental runtime skeleton in Rust: the fake's surfaces on top of its own enforcement core.

**Planned, not built:**

- The real runtime: identity, Wasm isolation for untrusted participants, and records that survive a crash. The experiment's results decide its architecture and language, and the core specification waits for them.
- The inspector, where a person reads what an activity did.
- The component SDK. [sdk/](sdk/README.md) is a placeholder.

## A few words

Diamond Dog's documents use a small, fixed vocabulary.

| Word | Meaning |
| --- | --- |
| Activity | A bounded scope of work with its own grants, budget and outputs. It outlives the processes that perform it. |
| Grant | Giving a capability to an activity or participant. It names the object, the operation, the recipient, the duration and the person it acts for. |
| Participant | Any software taking part in an activity: a component, a script, an app or an AI participant. |
| Receipt | A system-written record that a consequential action was attempted and what was observed of its outcome, including "unknown". |

The record keeps what a participant claimed apart from what the system observed, and says "unknown" when the evidence does not settle the outcome.

## Find your starting point

| Task | Start here |
| --- | --- |
| Run or understand the fake runtime | [fake/README.md](fake/README.md) |
| Read the product-facing contracts | [interfaces/diamond-dog/](interfaces/diamond-dog/) |
| Inspect the reference fixtures | [examples/diamond-dog-fake/](examples/diamond-dog-fake/README.md) |
| Read the Scenario A results and findings about Golem | [experiment/README.md](experiment/README.md) |
| See why things are the way they are | [docs/decisions/](docs/decisions/) and the [documentation index](docs/README.md) |
| Follow the delivery rules | [Operating guide](SOFTWARE_OPERATING_GUIDE.md) and [PR workflow](PR_WORKFLOW.md) |

## License

Diamond Dog uses two licenses:

- **The runtime and system services** are licensed under the GNU General
  Public License, version 3 or any later version (SPDX:
  `GPL-3.0-or-later`). See [LICENSE](LICENSE).
- **Everything other software builds against** is licensed under the MIT
  License (SPDX: `MIT`). See [LICENSES/MIT.txt](LICENSES/MIT.txt). Writing a
  component for Diamond Dog does not require licensing that component under
  the GPL.

Which license covers which path:

| Path | License |
|---|---|
| `sdk/` (component SDK) | MIT |
| `interfaces/` (contract and interface definitions) | MIT |
| `examples/` | MIT |
| Everything else (runtime, fake runtime, experiment, system services, docs, tooling) | GPL-3.0-or-later |

Each MIT folder carries its own `LICENSE` file, so a copy of just that folder
keeps its license. [REUSE.toml](REUSE.toml) records the same mapping in a form
tools can check: run `reuse lint` to confirm every file is covered. CI runs
the same check on every pull request. New source files also start with a
license header, which takes precedence over the map (the copyright line keeps
the OrcaOS name for now):

<!-- REUSE-IgnoreStart -->
```text
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: GPL-3.0-or-later   (or MIT under sdk/, interfaces/, examples/)
```
<!-- REUSE-IgnoreEnd -->

The full license texts live in [LICENSES](LICENSES); `LICENSE` at the root is
a copy of the GPL so GitHub detects it.

## Shared project instructions

[AGENTS.md](AGENTS.md) and [CLAUDE.md](CLAUDE.md) remain tracked so local and cloud agents receive the same project instructions. Keep them identical and keep personal overrides in ignored `CLAUDE.local.md`. Private dated infrastructure references remain separate; a source checkout does not reserve capacity or authorize a deployment.
