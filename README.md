# OrcaOS

A delegation-first operating system.

## License

OrcaOS uses two licenses:

- **The runtime and system services** are licensed under the GNU General
  Public License, version 3 or any later version (SPDX:
  `GPL-3.0-or-later`). See [LICENSE](LICENSE).
- **Everything other software builds against** (the component SDK,
  contract and interface definitions, and examples) is licensed under the
  MIT License (SPDX: `MIT`). See [LICENSE-MIT](LICENSE-MIT). Writing a
  component for OrcaOS does not require licensing that component under the
  GPL.

Code that is MIT-licensed carries an `SPDX-License-Identifier: MIT` header
or lives in a directory with its own copy of the MIT license. Everything
else falls under the GPL.

## Project scaffold

Practical provider-neutral workflow for Claude and Codex, policy v3.1.4 (October 11, 2026).

Copy into a project, fill in the short brief/current record and project commands, and replace this README with project setup/usage. One delivery owner implements and verifies routine work. Use focused independent review for high-risk behavior or a concrete consequential concern, rather than every PR.

Start with the person using the result, including teammates and maintainers. Deep engineering should support intuitive interfaces, complete approved behavior and clear documentation. Use the optional human-centered prompts in [HANDOFF_TEMPLATES.md](HANDOFF_TEMPLATES.md) within ordinary work; they add no forms or approval gates. Keep approved project identity and audience context in the supplied brief rather than relying on company files outside the copied scaffold.

Welcome the person's taste and imagination, including playful, unconventional and entertaining ideas. Apply the [operating guide](SOFTWARE_OPERATING_GUIDE.md) to realizing their approved goal with care and technical rigor; no added judgment phase or prescribed product aesthetic.

- SOFTWARE_OPERATING_GUIDE.md: delivery, risk-based review, verification and authorization.
- AGENTS.md / CLAUDE.md: identical short instructions and project commands.
- PR_WORKFLOW.md: author checks, selective review, merge brief and follow-through.
- docs/AGENT_ROLES.md: optional specialist contracts.
- HANDOFF_TEMPLATES.md: short optional task/review/handoff prompts.
- docs/DEVOPS.md: applicable deployment agreement; fill relevant parts before building around platform assumptions.
- .claude/agents/: Claude-only role adapters; use Codex's supported delegation separately. Existing adapter model settings are unchanged; invoke specialists only when useful and authorized.

Default to an owned branch/worktree, one coherent increment/PR, proportionate checks and one useful milestone update. Keep shared main clean for reading. Jason normally merges and accepts releases; honor any recorded project-specific standing permission within its exact scope, and do not generalize a one-time exception.

## Adoption and remote workers

This folder is a local non-Git scaffold. Updating it does not rewrite existing project policies, change protections, install providers or grant external-action authority. Apply it deliberately when initializing a project or reconciling its instructions; keep both entry points identical.

The current .gitignore keeps operating documents/tool configuration private. Git clones do not receive ignored files. Supply them through supported project/session instructions or explicitly authorize tracking the needed files. Keep project state accessible to its workers; do not assume policies/adapters/reviewers were delivered or ran.

docs/PLATFORM_SNAPSHOT.md is private, dated context, not reserved capacity or current billing. Verify only the relevant current constraints before admission. The deployment starter's source backup in private DevOps is separately versioned; this local policy update does not synchronize that repository.
