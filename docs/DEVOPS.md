# Project deployment agreement

Status: **Unreviewed starter — not deployment-ready**

| Record | Value |
| --- | --- |
| Project / repository | TODO |
| App development owner | TODO |
| DevOps owner / coordination chat | TODO |
| Frontend and domain owner | TODO |
| Product / budget / launch owner | TODO |
| Template source | Orca-Solutions/dev-ops, `templates/project-devops/DEVOPS.md` |
| Copied template commit | 205908e1dbacf86b0e050a4830322a8ff42f121f |
| Platform brief date | October 9, 2026; revalidate for this project |
| First development review / latest contract review | TODO: date, reviewer, app commit |
| Release handoff / accepted deployment evidence | TODO: repo-relative links, or not yet available |

This file records choices that development and operations must agree on. Fill
unknowns explicitly and link their resolution. It contains configuration key
names and credential ownership only, never secret values or customer data.

## Platform brief supplied by DevOps

These are available deployment patterns, not reserved capacity or a promise that
every feature is installed. Confirm the selected pattern before building around
it. GitHub remains the source repository regardless of the hosting choice.

| Pattern | Established direction | Boundary to check |
| --- | --- | --- |
| Static landing, docs or blog | Cloudflare Pages is the current default; existing suitable GitHub Pages sites can remain | Static output only unless server execution is explicitly designed. Select Git-triggered or manually approved deployment. A Pages project does not automatically configure the production domain. |
| Public backend | Shared Oracle VM in San Jose (`us-sanjose-1`), Ubuntu ARM64; Cloudflare proxy → shared Caddy → app-specific private network | One 2-OCPU / 12-GiB host with existing workloads. New apps need measured resource and disk admission. No public DB/control ports or unrestricted proxy trust. |
| Private development backend | App-specific private Oracle stack and authenticated operator tunnel, following iYosi's pattern | Accepted iYosi access is synthetic and bound to the operator Mac's loopback listener. Physical devices, other developers and real private data need separate access/acceptance decisions. |
| Small personal app | Railway is already used for personal GetFit and Treasury | Separate service, credentials and persistent volume. Verify actual plan, expiry, costs and backups; neither a subscription nor durable recovery is assumed. |

Shared Oracle apps have separate networks, state, secrets and resource limits,
but share a host/kernel and failure domain. Build immutable ARM64 images outside
production; agree on private registry or verified offline delivery and pin the
source commit plus image digest. Do not assume GitHub can SSH to the host.

App databases and private object storage are chosen per project. Existing
Postgres/PostGIS, SQLite or Kafka deployments are not a shared database service
for new apps. Managed PostgreSQL, point-in-time recovery, independent object
storage and an unattended backup service are not universal installed features.
Recovery hooks and independent encrypted-copy procedures exist for accepted
apps; a new app must integrate and verify its own restore semantics.

Static pages and API traffic can be separate browser requests. Decide exact
origins, cookies, CORS, OAuth callbacks, WebSockets and security headers together.
No provider purchase, paid upgrade, public exposure or credential creation is
authorized by this document. Record the approval needed for the actual choice.

## Project declaration — fill before architecture is settled

- Selected pattern and reason: **TODO**.
- Users and data: **TODO** (public/private; synthetic/real; sensitive categories;
  expected first-stage users, concurrency and workload).
- Components: **TODO** (runtime/version, static build, DB/version, jobs/queues,
  external services; ARM64 support or exception).
- State: **TODO** (each persistent store, approximate initial/growth size,
  authoritative vs cache/ephemeral, owner and export format).
- Public and private routes: **TODO** (proposed hostnames, internal listening ports,
  health/readiness, callback and WebSocket paths; no invented IP allocations).
- Deployment budget and approved services: **TODO** (monthly allowance, usage
  drivers and alert owner; a budget is not a provider-enforced hard cap).
- Availability and recovery: **TODO** (acceptable maintenance, recovery time and
  maximum lost writes; singleton or safe overlap, and why).

## Decisions and early flags

Record concrete conflicts at the first development review. A resolved item needs
a decision/evidence link, not just a green checkbox. App-specific choices override
generic suggestions only after the affected owners reconcile the consequences.

| Topic | App requirement or proposed choice | Status / question / impact | Decision owner and resolution |
| --- | --- | --- | --- |
| Runtime, ARM64 and image delivery | TODO | Unknown | TODO |
| Database roles, storage and migrations | TODO | Unknown | TODO |
| Singleton, background jobs and graceful shutdown | TODO | Unknown | TODO |
| Domains, authentication and browser origins | TODO | Unknown | TODO |
| Cache and service-worker update behavior | TODO | Unknown | TODO |
| Resource limits, load, disk growth and cost | TODO | Unknown | TODO |
| Backup, restore, deletion and retention | TODO | Unknown | TODO |
| Secrets, mail and external providers | TODO | Unknown | TODO |
| Release triggers, fallbacks and approval | TODO | Unknown | TODO |

Use `Unknown`, `Proposed`, `Blocked`, `Agreed` or `N/A (reason)`. Escalate a
specific dependent decision while continuing work that does not depend on it.
For example: an x86-only library affects Oracle compatibility; a single-writer
SQLite process affects overlapping deploys; a destructive migration affects
fallback; a cached offline client affects release coordination and data deletion.

## Implementation contract — complete applicable parts

**Runtime and operations.** Specify start/build commands, internal ports,
non-root UID/GID and mount permissions, health vs readiness, shutdown timeout,
job ownership/locking, CPU/RAM/PID/log limits and storage growth bounds. Keep
migrations serialized and separate from readiness. List every writable path;
container-local files are ephemeral unless explicitly mounted. Measure normal
and maintenance peaks; limits alone do not prove capacity. Production hosts are
not build machines or destructive rehearsal targets.

**Security and browser behavior.** List secret key names, injection location,
least-privilege roles and rotation owner; do not embed server secrets in frontend
builds. Define the exact trusted ingress peer/header chain, allowed origins,
cookies/CSRF, request limits and public route allowlist. Set appropriate policies
for HTML, assets, API, auth responses and service workers. Auth/private responses
must not enter a shared cache. Agree on OAuth, mail sending/receiving and optional
challenge providers without assuming accounts or quotas already exist.

**Updates and fallback.** Define whether main merges build artifacts, deploy
previews, deploy production or require an approval. Source merge, image build,
npm publication and host activation are independent events unless explicitly
wired and verified. Pin candidate and retained fallback identities. Document
schema and old-client compatibility, worker/singleton behavior, stop/start order,
health acceptance, failure detection and recovery after interruption. Ordinary
app fallback preserves new writes and auth/sync state; a DB restore or destructive
downmigration is a separate recovery operation. If fallback is incompatible,
declare that early and choose a safe maintenance/forward-fix strategy.

**Recovery and privacy.** Name every store, backup destination and derivative
(WAL/base backups, dumps, object versions, staging and restore copies). Define
encryption/key recovery, off-provider copies when required, cadence, age-based
retention, expiry verification and delivered failure alerts. Align promises with
actual deletion behavior, including offline clients, fallbacks and old backups.
Where restores could resurrect deleted data, agree on independent deletion
evidence and replay before requests resume. Bellos's 28-day backup target,
30-day maximum and separate 60-day journal are project-specific, not defaults
for every app. Verify recovery in disposable storage; never test by overwriting
live data or deleting production volumes.

## Before first deployment

- Review exact source/image/config and migration/fallback compatibility.
- Confirm selected provider/plan, actual capacity/storage and credential custody.
- Verify auth, persistence after restart, cache behavior, domain/TLS and any
  proxy/WebSocket behavior on the actual target, as applicable.
- Verify required backup/export/restore and retention controls with independent
  recovery access; list unperformed checks explicitly.
- Record rollout order, acceptance/abort criteria, fallback and ownership in the
  release handoff. Apply this repository's actual merge and deployment approvals.

These are review prompts, not an installed automated gate. A static site can mark
DB/migration/recovery items N/A. Private synthetic development and public launch
need different acceptance evidence; do not promote one into the other by changing
a status label.

## Changes affecting the other team

Both app developers and DevOps update this record when they change a dependency,
schema, runtime, state path, origin, route, cache policy, secret interface, resource
budget, retention or deployment order. Put the impact and linked decision in the
same PR as the change where practical. Coordinate changes requiring both repos;
record the safe rollout order rather than assuming simultaneous merges.

| Date / source or infrastructure revision | Change and reason | Affected code/config and rollout action | Owners / decision link |
| --- | --- | --- | --- |
| TODO | Initial platform review | TODO | TODO |

Keep this file navigable and complete for its scope: link detailed runbooks and
release receipts rather than duplicating them, and retain essential operating
knowledge. Review it at architecture changes and before releases; do not silently
replace agreed project choices with a newer upstream template.
