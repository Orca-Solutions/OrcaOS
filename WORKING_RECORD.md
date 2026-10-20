# Scaffold working record

## October 20, 2026 — Scenario A on Golem, deterministic variant

Jason chose to build the deterministic Scenario A experiment on Golem. Built under `experiment/`: the shared reference implementation (`shared/orca-ref`), the mock mail service, the Golem 1.6.0 component (journal writer, supervisor, activity coordinators, processors, drafter, approval path, mail adapter), and the harness with failure injection and facilitator controls. Out of scope and not built: the AI participant, the conventional baseline, any OrcaOS runtime or spec, and promotion.

Result: the build passed all seven acceptance tests and the P1/P2 revocation race (20 repetitions per crash or race test, 100 for the budget race) under the assumptions in `experiment/results/2026-10-20-golem/record.json`. That is a conformance result, not a security claim. Golem findings that change how the plan's configuration rules can be followed are in `experiment/README.md` (Findings): installed cards narrow rather than add, so children cannot hold cards derived from the parent's and rule 4's cascade is not achievable as written; key-value grants are whole-bucket or exact-key; a secret grant cannot be narrowed to one secret; quotas are invocation-scoped; Golem resends in-flight HTTP on recovery, which the adapter stops with a custom durable operation; processors ran as agents, not tools.

Verification: `cargo test -p orca-ref` (18 tests), the full suite, card lists before and after, `reuse lint`. A fresh-context reviewer read the authorization and recovery slice; two major findings (recipient not checked at mint; test 7 timing window) and four minor ones were fixed before the recorded run, and one minor (the journal trusts the caller-named source) remains open as documented. Scorecard (tokei 12.1.2): 1,126 counted code lines and 170 configuration lines; the adapter splits 73% reusable, 27% mock-mail-specific.

Next: the conventional baseline, the parity review, performance measurements, and the hands-on evaluation on this build's facilitator controls. Draft PR on branch `claude/scenario-a-golem-ovke1m`; merge belongs to Jason.

## October 11, 2026 — Readable code, v3.1.4

Applied Jason’s queued General Management request during the regular documentation review: tasteful whitespace between distinct logical blocks, closely related statements kept together, and respect for existing formatters/linting and language idioms. Generated/minified code excluded; focused improvements to touched code only. No behavior changes, source-formatting campaign, checklist, review gate or formatter mandate. Updated guide, identical entry points and optional handoff prompt; aligned current companion version labels. Prior scaffold preserved in `Scaffold Policy Archive/before-v3.1.4-20261011`. Author verification covers affected text, entry-point parity, local links, whitespace and unchanged adapter/settings hashes. Central edits are complete; project adoption and draft PR delivery remain separately tracked in the weekly review state.

## October 10, 2026 — Approved identity and scaffold consistency, v3.1.3

Jason approved the company identity and requested central-scaffold integration and consistency. Read the approved `creative-team/COMPANY_IDENTITY.md` and all 24 scaffold files, including hidden Claude adapters/settings, hook, privacy exclusions, deployment starter and dated infrastructure/handoff records. Added respect for clients' taste and imagination, including playful/unconventional entertainment, while retaining evidence-based technical reasoning and approved scope. Explained the existing ORCA Method as perspectives within the delivery loop, not separate documents, gates or a judgment phase. Company narrative and accepted visual/tagline decisions remain centralized.

Reconciled current guide/entry/README/template/PR/role companion versions; made integrated author verification explicit in PR/role guidance; clarified recorded standing permissions in README/handoff; updated the docs-index description and deployment-starter completeness wording. Preserved completeness, documentation resilience, confidentiality, risk-based review and authority. Brief/spec remain empty project placeholders. Hidden adapters/settings, signing hook, ignore rules, dated platform snapshot and prior handoff receipt are unchanged. No invented current infrastructure or acceptance evidence.

Previous complete scaffold, prior alignment record and before-hash manifest archived at `Scaffold Policy Archive/before-v3.1.3-20261010-141254`. Current exact scope/findings: `creative-team/DOCUMENTATION_ALIGNMENT.md`. Author checks: affected diffs, parity, local links, active-version consistency, whitespace and unchanged adapter/settings/history hashes. Central scaffold only; no repository rollout/PR update/publication/merge/runtime action or new mandatory review/report layer.

## October 10, 2026 — Human-centered practice, v3.1.2

Creative Team coordinated Jason's direction: there is a human center underneath intuitive interfaces and deep engineering. Read its draft `creative-team/COMPANY_IDENTITY.md`; translated the durable principle into the local scaffold without copying draft mission/vision language into project instructions. The guide now starts from the person's goal/context/attention and includes nonblaming language, accessible interactions, honest status, consequential choices, recovery and maintainers as users. Matching entry points carry one concise rule; README and optional handoff prompts make the guidance usable in ordinary work.

Reviewed PROJECT_BRIEF.md and docs/SPEC.md: both are empty project placeholders, left unchanged rather than inventing approved scope or brand claims. Preserved completeness, documentation resilience, proportionate author verification, focused high-risk review and existing authority. No new forms, reports, approval gates or specialist staffing. No change to accepted ORCA Method/Signal or public website language.

Prior six scaffold files archived at `Scaffold Policy Archive/before-v3.1.2-20261010-140611`. Exact changes and findings: `creative-team/DOCUMENTATION_ALIGNMENT.md` in the management folder. Author checks: affected diffs, entry-point parity, local Markdown links, whitespace and retained safeguards. Local scaffold only; no existing repository/PR rollout, publication, merge or runtime action.

## October 9, 2026 — v3.1.1 repository adoption

Rolled the final completeness and documentation-resilience principles into the existing documentation candidates across the same maintained scope: 16 existing draft PRs updated with signed incremental commits and verified remote heads/draft states/signatures; five additional signed candidates retained pending direct publication approval. Guarded local adoption updated 43 policy files across 15 folders, including five non-Git projects. Project commands/contracts, privacy exclusions and DevOps's explicit optional-review/direct-publication authority preserved. No application merges or runtime deployments.

Per-project paths, exact heads, PR links, checks, exceptions and publication holds are recorded in `Documentation Review/workflow-v3-rollout/v3.1.1-20261009/incremental-report.md`; `latest-rollout.json` in the parent rollout directory points to the current report. Previous v3 results remain historical evidence. Author checks included targeted policy diffs, paired entry-point consistency, staged Markdown scope/whitespace, exact public snapshot secret scans, guarded local hashes, Treasury's required privacy check, and published draft/signature verification. All updated PRs attached to Repo & Documentation Mgmt.

## October 9, 2026 — Completeness clarification, v3.1.1

Jason clarified through General Management: simplicity must never substitute for completeness; simplicity on the outside, necessary complexity underneath. Added an explicit paragraph to the operating philosophy and matching rule to both entry points: complete approved behavior includes necessary edge cases, invariants, failure handling and operational obligations; simplify the recipient's experience without reducing required capability or correctness. Completeness follows approved scope, without speculative expansion, and material consequences, permissions, costs and recovery choices remain visible.

Previous four files archived at `Scaffold Policy Archive/before-completeness-clarification-20261009-205956`. Author checks: inspected the small wording change and verified entry-point parity. Central scaffold only; no new process gates, repository rollout, runtime or publication changes.

In the same narrow v3.1.1 amendment, Jason emphasized documentation as the core of resilience across teams. The philosophy and documentation guidance now require durable shared memory sufficient for another authorized team/session to continue: scope, consequential decisions/why, implementation versus deployed state, interfaces, remaining issues, useful evidence and applicable setup/release/recovery. Detail follows actual scope/risk; records must be navigable and complete rather than merely short. Less duplicate reporting, not less durable knowledge. Updated both matching entry points, preserving dated history and one authoritative home per fact without a new review/report layer. The immediately preceding wording is also preserved in the timestamped `Scaffold Policy Archive/before-documentation-emphasis-*` snapshot. Author checks: affected wording, entry-point parity and whitespace.

## October 9, 2026 — Principles restored, v3.1

Jason requested restoration of substantive principles condensed during v3.0, while retaining efficient delivery. He clarified that deep thinking and complex logic beneath an intuitive interface and powerful simplicity is the operating philosophy for everything we do, not only product design. The guide now applies it explicitly to products, architecture, code, documentation, tools, coordination, verification and handoffs.

Restored explicit architecture boundaries and data portability, service/licensing and spending discipline, client/employment confidentiality and scope, and proportionate durable-delivery requirements. Added the clarification that author verification happens during implementation, without a separate exhaustive review/report/subagent by default. Existing consequence-based independent review, evidence reuse and project-specific authority remain unchanged. Updated both matching agent entry points; no new review or approval gates.

Prior guide, entry points and record preserved in `Scaffold Policy Archive/before-v3.1-20261009-205723` outside the active scaffold. Author checks: inspected the affected policy text and archived-version comparison, matching entry-point bytes and referenced local paths. This updates the central local scaffold only; existing repository rollout PRs are not updated by this edit. No runtime or provider actions, commits or publication.

## October 9, 2026 - Practical workflow v3.0

Jason requested a fast, economical workflow with quality preserved, explicitly rejecting independent review on every change. Updated the central local scaffold accordingly. Default is one delivery owner, meaningful author verification and focused independent review only for high-risk behavior/concrete consequential concerns. Existing project-specific requirements and repository protections remain until deliberately reconciled.

Updated guide, matching entry points, PR workflow, optional roles/templates, indexes and two adapter descriptions. Configured adapter models, settings, deployment starter and private platform snapshot unchanged. No project synchronization, commits, publication, deployment or messaging performed by this revision.

Previous files archived at `/Users/jasonfricano/Documents/orca solutions mgmt/Scaffold Policy Archive/before-v3-20261009-192406`. Author verification: entry-point parity, linked active-file existence, policy consistency and adapter-only trigger changes. This local policy update does not require a ritual independent review under Jason's request; no independent reviewer claimed.

Coordination received during this edit: Repo Management reports DevOps PR52 installed a separately authorized DevOps-only direct-push/ready-merge workflow. This scaffold preserves that project-specific authority; it does not extend it to application repositories. No additional review of PR52 performed here.

Next: adopt in existing projects when reconciling their instructions; do not silently rewrite live project contracts or relax required GitHub checks.

## October 9, 2026 — Deployment starter

Requested by Jason in DevOps Management. Added `docs/DEVOPS.md` and private `docs/PLATFORM_SNAPSHOT.md`; linked the agreement in matching AGENTS.md/CLAUDE.md and indexes; added the snapshot ignore rule. Existing policies and contributor workflow remain in place.

Source: private Orca-Solutions/dev-ops PR53, commit `205908e1dbacf86b0e050a4830322a8ff42f121f`. Independent source review passed at the exact source commit; independent local scaffold review passed with no actionable findings. This remains an unfilled project starter, not deployment acceptance. The usage snapshot is from October9 05:59 Pacific; forecasts are conditional and billing is unverified.

The scaffold folder is not a Git repository. Its reviewed source backup is in DevOps PR53; this local delivery is not a commit or merge in an application repository. No app, credential, provider or live service changed.

Reviews: Prod Backend Deployments verified source, retained-usage and price calculations, links, signature and secret scan. Repo & Documentation Mgmt verified local parity, matching agent instructions, source references and ignore boundaries. Neither review established model diversity, live usage/billing, deployment acceptance or visual Mermaid rendering. See docs/handoffs/20261009-devops-starter-review.md for the reviewed file hashes.

Next: use the starter when initializing a project. Fill all relevant project TODOs before claiming an agreed deployment contract. Keep the internal snapshot out of public source; Git clones will not receive ignored instructions or this snapshot automatically.
