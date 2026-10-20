# Practical PR and release workflow

Companion to policy v3.1.4 | October 11, 2026

The author checks every change. Independent review is required for high-risk behavior, not every PR. Follow SOFTWARE_OPERATING_GUIDE.md section 3 for classification. Existing repository-required checks/protections still apply.

## 1. Own and build a useful increment

Use an owned branch/worktree from the appropriate base. Inspect real state, preserve unrelated work and name dependencies. Default to one PR for a coherent outcome, including affected documentation. Open a draft while incomplete; commit/push only within authorization and use configured signing. Keep shared main a clean reading baseline.

A bounded fix needs a short outcome and acceptance check, not a new planning ceremony. The author inspects the diff and verifies affected behavior during implementation: inspect a typo, exercise an affected UI flow, or reproduce a bug and check relevant regressions. No separate exhaustive self-review, report, subagent or full suite by default. Record unrun checks and reasons.

## 2. Choose the review needed

Record one line in the PR: `Review: author verification sufficient - [reason]` or `Review: focused independent review required - [risk]`.

- Routine reversible copy/layout, ordinary docs, maintenance and standard behavior within existing controls: author verification is sufficient unless a concrete concern or repository protection requires more.
- Auth/access, secrets, payments, destructive migrations, recovery/deletion/retention, sensitive-data/tenant boundaries and recovery-critical production changes: independent review of the affected risky behavior before its use/release.
- Consequential uncertainty or repeated failed fixes: run a small reproduction, then consult a relevant specialist if still unresolved. This does not automatically require a project-wide audit.

Give the independent reviewer the exact candidate, acceptance criteria, risky areas and existing checks. It reviews relevant code and evidence, runs targeted checks that add confidence and reports actionable findings. It need not repeat the author's entire suite. Reuse evidence honestly and identify checks actually rerun. Self-review remains self-review.

Fix findings in the existing PR. Recheck repaired behavior and affected dependencies. Later pushes require a delta assessment: routine independent changes do not invalidate unchanged risky evidence, but changed risk paths do. A concrete new blocker still prevents affected readiness. Optional improvements go to the backlog only when useful; they do not silently expand the increment.

## 3. Ready and merge brief

A PR can be ready when its outcome works, relevant required checks pass, any required independent review is complete and blockers are resolved. Routine work does not wait for an optional reviewer. Keep it draft if Jason requested that or required evidence/dependencies remain pending. A source-only PR can be ready while deployment remains a separate blocked task; make that boundary explicit.

Give Jason one short brief:

- PR and exact head; outcome and material changes.
- Checks actually run/reused; review classification and reviewer if required.
- Remaining gaps/risks and any decision needed.
- Merge/deployment effects and next action; mention existing autodeployment.

Do not create duplicate audit/approval documents. A working-record link is enough for detailed evidence. Readiness is not human acceptance or deployment permission.

## 4. Merge and follow-through

Normally Jason merges. Existing explicit authorization, including a recorded standing permission for a particular repository/action, governs that scope. Without it, obtain specific authority for an agent merge; do not generalize an exception. Never bypass protections, invent formal approval or enable auto-merge. When using Jason's GitHub account, do not claim a separate human approval that GitHub cannot distinguish.

After observing the merge, record the actual revision. One owner updates shared main fast-forward-only when clean and reserved for synchronization; preserve other worktrees. Release only within authorization. Check the actual relevant target behavior before calling it deployed/accepted; routine changes do not demand a fresh complete recovery rehearsal.

## Ownership

The delivery owner implements or coordinates the outcome and its evidence. The author repairs findings. A reviewer is involved only where needed. Repo management assists with actual cross-worktree/publication issues and integration, not mandatory sign-off for every PR. DevOps/Backend/Frontend contribute where their interfaces are affected. Jason retains material decisions and normal merge/release acceptance.
