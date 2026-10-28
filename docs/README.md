# Documentation index

Where Diamond Dog's repository documents live and when to read each one. The repository and older documents still use the working name OrcaOS.

## The project

- ../README.md: what Diamond Dog is, what is built versus planned, and the license split.
- ../WORKING_RECORD.md: dated record of what was decided, built and checked. Newest entry first.
- ../PROJECT_BRIEF.md and SPEC.md: status notes only. The approved scope lives in the experiment plan and decision log outside this repository, and the core specification waits for the experiment's results.
- decisions/: accepted architecture decisions. 0001 licensing; 0002 the fake Diamond Dog; 0003 the experimental runtime skeleton (proposed).
- handoffs/: dated implementation, review and merge handoffs.

## The code and its contracts

- ../interfaces/README.md: the provisional contract, its HTTP binding and fixture format, and the sensor observation draft (MIT).
- ../fake/README.md: running the fake Diamond Dog, what it does not prove, and where it reads the contract.
- ../runtime/README.md: the experimental runtime skeleton, its enforcement core, what it does not do yet, and where it reads the contract differently from the fake.
- ../examples/diamond-dog-fake/README.md: the reference household, manifest and fixtures.
- ../experiment/README.md: Scenario A on Golem, its results, scorecard and findings about Golem.

## How we work

- ../CLAUDE.md / ../AGENTS.md: identical agent entry points and project commands.
- ../SOFTWARE_OPERATING_GUIDE.md: shared operating philosophy, complete behavior, durable documentation, consequence-based verification and authority boundaries.
- ../PR_WORKFLOW.md: proportionate verification, selective independent review and merge follow-through.
- ../HANDOFF_TEMPLATES.md: optional handoff prompts.
- AGENT_ROLES.md: optional specialist contracts for Claude and Codex.
- DEVOPS.md: fill-in deployment agreement. Nothing is deployed; review it before any hosting, auth or release change.
- PLATFORM_SNAPSHOT.md: dated internal usage and cost reference. Local and Git-ignored; never committed.
