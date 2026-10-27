<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: MIT
-->

# Examples for the fake Diamond Dog

Data to run the [fake Diamond Dog](../../fake/README.md) with, MIT licensed so Orca Keel can copy any of it.

- `household.json`: a three-member household (Dana the owner, Sam an adult, Maya a child with Dana as guardian), a mail connection and a model provider. Pass it to `fake-diamond-dog serve --setup`.
- `daybook-manifest.json`: a Daybook manifest that pins every surface version.
- `fixtures/`: Diamond Dog's reference versions of the eight gating tests in the contract's section "The fake Diamond Dog", plus a ninth on scheduled runs across a clock change. Orca Keel owns the gating fixtures; these show the fake passes them as Diamond Dog reads the contract.

| Fixture | Contract test |
| --- | --- |
| `1-undeclared-operation-refused.json` | Undeclared operation refused |
| `2-injected-content-does-nothing.json` | Injected content does nothing |
| `3-changed-draft-voids-approval.json` | Changed draft voids the approval |
| `4-revocation-stops-future-use.json` | Revocation stops future use |
| `5-unknown-stays-unknown.json` | Unknown stays unknown |
| `6-app-switched-off.json` | App switched off |
| `7-audience-enforced-on-read.json` | Audience enforced on read |
| `8-disclosure-stays-in-category.json` | Disclosure stays in category |
| `9-scheduled-runs-on-the-wall-clock.json` | Not a gating test: G-1 clock changes and the catch-up window |

The format is in [interfaces/diamond-dog/fake-fixtures.md](../../interfaces/diamond-dog/fake-fixtures.md).
