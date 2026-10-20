# Lightweight task and handoff templates

Companion to policy v3.1.4. Optional aids, not mandatory forms. Include only information needed for the task; empty fields confer no authority. Do not create another document when the PR/working record already contains the answer.

## Start a new product / substantial feature

Define the user/problem, smallest useful outcome, exclusions and meaningful UX. Inspect relevant existing code and hosting constraints first. Investigate risky integration with a thin end-to-end slice. Ask Jason for missing material decisions, then record the approved essentials in PROJECT_BRIEF.md and execute without repeated planning approval. No broad build before scope/experience direction is clear.

Useful prompts for discovery, implementation or a handoff; use only those relevant to the existing task:

- What is this person trying to accomplish, in what context, and what currently causes effort, uncertainty or frustration?
- What experience, style or imaginative possibility do they want to create? What actual technical constraints need resolving to realize that intent?
- Is the next useful action clear and accessible? What do they need to understand or control before a consequential choice, and how can they recover from failure without blame?
- Could the next authorized teammate or maintainer understand the current state, evidence and next action without the original chat?
- When touching code, does tasteful spacing reveal its logical blocks while respecting existing formatting and keeping related statements together?

## Ordinary change

Implement [outcome] in [relevant scope]. Preserve unrelated work; use an owned branch/worktree. Reuse approved requirements. Verify with [observable behavior/check], update affected docs and brief state, and prepare one PR/merge brief. Author verification is sufficient unless actual consequences meet the high-risk rule or a specific concern/protection requires review.

## Focused independent review

Review [exact source/head] for [specific risky behavior] against [requirements]. You did not author it. Inspect supplied checks [links] and risky paths; run targeted checks only where they add confidence. Return actionable blockers/material gaps with evidence, reused/run/unrun checks and a concise verdict. No broad audit, tracked-file edits or merge. After repair, verify the delta and affected dependencies rather than restarting unchanged work.

## Specialist consultation

Answer [specific question] using [code/evidence/constraints]. Work read-only. Return a recommendation, relevant uncertainty and next step. Stop when answered. No new implementation/audit/release scope.

## Working record

- Current revision and stage; owner and next useful action.
- Approved scope/permissions or links.
- Actual blockers/decisions and dependent work.
- Relevant checks and review classification/evidence.
- PR/release links; source vs deployed state.

Update for meaningful state changes, not every command. Link detailed receipts only when needed.

## Merge brief

[PR / exact head]: [outcome]. Checks: [actual results/reused evidence/gaps]. Review: [author sufficient + reason OR focused reviewer + result]. Remaining risk/decision: [if any]. Merge effects/next action: [including autodeploy if applicable]. Jason normally merges; honor recorded project-specific standing permissions within scope and do not generalize a one-time exception.

## Human decision

Decision needed: [reserved choice/missing authority]. Recommendation: [concrete option and evidence]. Consequence/cost/risk: [material facts]. Blocked work: [specific dependency]; continuing work: [unaffected scope]. Reuse existing authorization and batch nonurgent questions.

## Client work

Keep each client's scope, records, code and data within its own project. Record inclusions/exclusions, acceptance and revision limits. Synthetic data by default; real data requires applicable confidentiality/access authorization. Present scope/time/cost changes to Jason; do not agree for him. Deliver usable setup, verified behavior and relevant maintenance/recovery boundaries.
