# OrcaOS

A delegation-first operating system in development.

This repository contains the provisional Diamond Dog interfaces, a local fake runtime, SDK material and examples. The fake supports integration exercises; it does not prove production isolation, durability or identity. Current scope, decisions and recorded checks are in [WORKING_RECORD.md](WORKING_RECORD.md).

## Find your starting point

| Task | Start here |
| --- | --- |
| Run or understand the fake runtime | [fake/README.md](fake/README.md) |
| Read the product-facing contracts | [interfaces/diamond-dog/](interfaces/diamond-dog/) |
| Inspect the reference fixtures | [examples/diamond-dog-fake/](examples/diamond-dog-fake/) |
| Understand the fake's scope | [Fake runtime decision](docs/decisions/0002-fake-diamond-dog.md) |
| Follow the existing delivery rules | [Operating guide](SOFTWARE_OPERATING_GUIDE.md) and [PR workflow](PR_WORKFLOW.md) |

## License

OrcaOS uses two licenses:

- **The runtime and system services** are licensed under the GNU General
  Public License, version 3 or any later version (SPDX:
  `GPL-3.0-or-later`). See [LICENSE](LICENSE).
- **Everything other software builds against** is licensed under the MIT
  License (SPDX: `MIT`). See [LICENSES/MIT.txt](LICENSES/MIT.txt). Writing a
  component for OrcaOS does not require licensing that component under the
  GPL.

Which license covers which path:

| Path | License |
|---|---|
| `sdk/` (component SDK) | MIT |
| `interfaces/` (contract and interface definitions) | MIT |
| `examples/` | MIT |
| Everything else (runtime, system services, docs, tooling) | GPL-3.0-or-later |

Each MIT folder carries its own `LICENSE` file, so a copy of just that folder
keeps its license. [REUSE.toml](REUSE.toml) records the same mapping in a form
tools can check: run `reuse lint` to confirm every file is covered. CI runs
the same check on every pull request. New source
files also start with a license header, which takes precedence over the map:

<!-- REUSE-IgnoreStart -->
```text
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: GPL-3.0-or-later   (or MIT under sdk/, interfaces/, examples/)
```
<!-- REUSE-IgnoreEnd -->

The full license texts live in [LICENSES/](LICENSES); `LICENSE` at the root is
a copy of the GPL so GitHub detects it.

## Shared project instructions

[AGENTS.md](AGENTS.md) and [CLAUDE.md](CLAUDE.md) remain tracked so local and cloud agents receive the same project instructions. Keep them identical and keep personal overrides in ignored `CLAUDE.local.md`. Private dated infrastructure references remain separate; a source checkout does not reserve capacity or authorize a deployment.
