<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Scenario A on the conventional composition

The conventional baseline from the experiment plan: the same deterministic Scenario A, built from Temporal, PostgreSQL, Podman and Wasmtime instead of Golem. It reuses everything the plan says both builds share, unchanged: the reference implementation (`shared/orca-ref`), the task logic (`shared/scenario-task`), the synthetic inputs, the mock mail service, and the harness with the same seven acceptance tests, the test 3 variant and the same checks. The plan uses it as the strongest practical alternative that "acceptable effort" on Golem is judged against.

Results are stated as narrowly as the Golem build's: a pass means this composition plus the code here passed the conformance suite under the recorded assumptions. It does not mean the build is secure.

- Results: [../results/2026-10-27-conventional/summary.md](../results/2026-10-27-conventional/summary.md), with the reproducibility record, mount lists, every repetition, the journals and the mock service logs next to it.
- Scorecard and the early-exit check against Golem: [Custom machinery](#custom-machinery) and [Early-exit check](#early-exit-check) below.

RESULTS-PLACEHOLDER

## What runs

| Process | What it does | Authority |
| --- | --- | --- |
| PostgreSQL 16 | Holds each run's journal writer (one row per run, locked for each command), the budget and its reservations, and the proposals. | Local socket only; no participant can reach it. |
| Temporal dev server (CLI 1.9.1, server 1.32.0) | Durable workflows and activities, with its own SQLite file so it survives a SIGKILL. | Loopback only. |
| Coordinator worker (`conventional worker`) | One `Coordinate` workflow per activity step: checks the journal, reserves budget, runs a participant container, records its outputs. | The journal and Podman. |
| Mail adapter (`conventional adapter`) | One `DeliveryWorkflow` per authorization on its own task queue. The only process holding the mail credential. | The journal, the credential file, the mock mail service. |
| Approval service (`conventional approval`) | Shows the principal the proposal with its payload hash, journals the approval, mints the authorization and starts the delivery workflow. | The journal and the adapter's task queue. |
| Participant containers | processor-1, processor-2, processor-2b and the drafter: one fresh Podman container per step, running the participant component under Wasmtime. | Only the store objects their live grants name, mounted read-only or writable; no network, no credentials, read-only root, all capabilities dropped. |

Every run has its own identifier: its journal row, its stores under `/opt/baseline/stores/RUN`, its workflow identifiers and its mock records are its own.

### The delivery path

The protocol in `src/adapter/delivery.rs` is the Golem build's, line for line, minus Golem's durability calls:

1. The drafter writes its draft; its coordinator submits it as the drafter's proposal. The approval service checks under the journal lock that the drafter's submit-proposal grant is live, and journals the proposal with its payload hash.
2. The principal approves the hash they were shown. In one transaction the approval service journals the approval, mints a single-use authorization and writes it to a delivery outbox; after the commit it starts `DeliveryWorkflow` with it, and an outbox loop starts any that a crash left unstarted. No participant ever holds the authorization.
3. The adapter checks the binding and asks the journal whether the authorization is live.
4. It asks the journal to consume the authorization if it is still live. Only when that consumption has committed does it submit, then journal the submission and the receipt.
5. If the adapter dies, the activity stops heartbeating and Temporal starts a new attempt within about five seconds. The new attempt runs the same code with the same delivery token (workflow and activity identifier), so the journal answers "already consumed" and the adapter takes the recovery path: status check first, re-dispatch only if the journal still allows it, otherwise status checks and receipt updates only. An attempt that is still alive when Temporal gives up on it (its heartbeats stopped reaching the server) stops at its next step when Temporal cancels it.
6. An outcome the adapter cannot settle gets a scheduled status check on a durable Temporal timer. The workflow is the standing reconciliation grant: cancellation and close are journal commands, not Temporal cancellations, so it outlives both. Scheduled checks never re-dispatch.

### Grant table to native enforcement

The journal holds the same grant table as the Golem build (`src/supervisor.rs`, `GRANT_TABLE`). Each time a participant container starts, `src/containers.rs` reads the holder's live grants from the journal and mounts exactly those objects: a snapshot read becomes a read-only bind mount of that one file, an append or write grant a writable mount of the output space or object. A grant that is revoked, expired or derived from a revoked grant mounts nothing.

| Plan grant | Journal grant | Native enforcement |
| --- | --- | --- |
| processor-1 reads `c1@v1`, `c2@v1`; appends to `out/A1.1` | `g-p1-read-c1`, `g-p1-read-c2`, `g-p1-append` | `/stores/docs/c1@v1` and `c2@v1` read-only, `/stores/out/A1.1` writable |
| processor-2 reads `c3@v1`; appends to `out/A1.2` | `g-p2-read-c3`, `g-p2-append` | `/stores/docs/c3@v1` read-only, `/stores/out/A1.2` writable (its step state lives there too) |
| drafter reads `out/A1.1`, `out/A1.2`; writes `out/A1/draft`; submits a proposal | `g-d-read-a11`, `g-d-read-a12`, `g-d-write`, `g-d-propose` | Both output spaces read-only and `/stores/out/A1/draft` writable; the coordinator submits only while `g-d-propose` is live |
| Mail adapter sends as `principal-1@mock.test` | `g-send`, on an authorization minted from an approval | Only the adapter process reads the credential file. Every service runs as root on this host, so that is by construction, not enforced; participants cannot reach it |
| Processors have no network | — | `--network=none` on every participant container |

`harness cards RUN` (with `ORCA_PLATFORM=conventional`) compares every participant's mounts with the grant table the journal recorded; the suite does this before and after, and stops if any participant would mount something undeclared.

## Running it

The recorded run used one Ubuntu 24.04 host with PostgreSQL 16.15 (preinstalled), Podman 4.9.3, Wasmtime 49.0.4 (`cargo install wasmtime-cli --version 49.0.4 --locked`), the Temporal CLI 1.9.1 built from source with Go, Rust with the `wasm32-wasip2` target, and `protoc` for the Temporal SDK's build. Paths default to `/opt/baseline`; `ORCA_BASE`, `ORCA_PG`, `ORCA_STORES`, `ORCA_PARTICIPANT_IMAGE` and `ORCA_APPROVAL` override them.

```sh
cd experiment
cargo build -p conventional -p mock-mail -p harness
cargo build --release -p participant --target wasm32-wasip2

# 1. The participant image: Wasmtime and the component, nothing else
mkdir -p /opt/baseline/image
cp /opt/baseline/bin/wasmtime target/wasm32-wasip2/release/participant.wasm /opt/baseline/image/
podman build -t localhost/orca-participant:latest -f conventional/Containerfile /opt/baseline/image

# 2. A random mail credential, never committed
mkdir -p /opt/baseline/run
head -c 24 /dev/urandom | base64 | tr -d '/+=' > /opt/baseline/run/mail-credential
chmod 600 /opt/baseline/run/mail-credential

# 3. Mock mail service
MOCK_MAIL_CREDENTIAL="$(cat /opt/baseline/run/mail-credential)" \
  ./target/debug/mock-mail --log-dir /opt/baseline/run/mock-logs &

# 4. The platform: PostgreSQL cluster and schema once, then every process
conventional/stack.sh init
conventional/stack.sh start

# 5. The suite, with the same repetitions as the Golem run
ORCA_PLATFORM=conventional ./target/debug/harness suite --reps 20 --race-reps 100 \
  --prefix conv1 --out results/<date>-conventional
```

`stack.sh kill` stops every platform process with SIGKILL; the crash repetitions call `kill` then `start`, so nothing else should use this stack during a suite. Tests 7a, 7b and 7d end by waiting for the 10-minute validity window to close, as on Golem.

The facilitator controls are the same as on Golem: `ORCA_PLATFORM=conventional harness ctl RUN OP` (setup, process, show, approve, revoke, grant, cancel, close, replace, fault, clear, hold, release, crash, journal; `pending` exists only on Golem). `conventional ctl RUN OP` is the platform's own tool, used by the harness: the principal's controls, `export`, `mounts HOLDER` and `participant HOLDER ARGS`.

## How the harness differs per build

The tests, their checks and their pass conditions are the same; only where native evidence is read differs (`harness/src/platform.rs`, `harness/src/scenario.rs`).

| Test | Golem build | Conventional build |
| --- | --- | --- |
| 1 | Refusals read from the processor's oplog (`denied:` lines), journaled with source `golem-kv`. | Refusals read from Wasmtime's WASI trace (an `open-at` that returned an error), journaled with source `container-fs`. |
| 3 | After the restart, re-grant `g-send-2` and `g-d-propose-2`. | The same two, plus the drafter's three store grants (`g-d-read-a11-2`, `g-d-read-a12-2`, `g-d-write-2`). Revoking A1's grants took the drafter's mounts with them; Golem's cards do not narrow on revocation, so the Golem drafter kept its store access. |
| 5 | Narrowed natively: the narrowing card installed. Direct call refused by Golem's key-value store. Card lists compared. | Narrowed natively: the old holder's mounts are empty and any running container of its is stopped. Direct call finds no `c3@v1` to read. Mount lists compared. |
| 3, 7 | Crash: SIGKILL the Golem server and restart it. | Crash: SIGKILL PostgreSQL, Temporal, both workers and the approval service, and restart them. |
| 7 | "Golem did not resend the request on its own." | "Temporal did not resend the request on its own." Same check: no wire request after the crash. |

## Findings

What building this found about the conventional composition, for comparison with the Golem build's findings.

1. **Revocation cascades natively.** Because a container mounts only what the journal says is live at the moment it starts, revoking a parent grant removes everything derived from it at the store, which Golem's cards could not do (Golem finding 5). The cost is that a running container keeps the mounts it started with, so after every revocation, cancellation, close or replacement the supervisor stops any running container whose holder's live grants changed since it started (each container carries a digest of its mounts as a label).
2. **Temporal never resends on its own; it re-runs our code.** A crashed activity is retried as a new attempt from the top, so nothing like the Golem build's custom durable operation is needed to stop a resend. The journal's delivery token does the work. Detecting the crash in seconds needs an activity heartbeat; without it the retry waits for the whole start-to-close timeout (an hour here).
3. **At-least-once activities and plain services need crash-safe side effects.** A retried activity repeats its journal writes unless they are made safe to repeat: recording outputs skips one already journaled, and the approval service returns the proposal it already made for the same submission key. A service crash between two commits leaves half a step, so the approval service commits its journal commands with its own rows in one transaction under the journal lock, and hands minted authorizations to the adapter through an outbox. Golem replays a completed call's result and makes the call to the adapter durable instead. This is most of why the baseline's approval code is larger than Golem's.
4. **Store refusals leave no record unless the runtime traces them.** A file outside the mounts simply is not there. Wasmtime's `wasmtime_wasi=trace` log records every WASI call and its result, which the inspector parses: an unmounted object returns `no-entry`, a write to a read-only mount returns an I/O error after the host's "Read-only file system".
5. **A restart is fast.** Killing and restarting all five processes took about 0.7 s, and Temporal retried the interrupted delivery about 5 s later, against Golem's roughly 30-second resume.
6. **PostgreSQL after SIGKILL on this host.** The killed postmaster stays a zombie until the container's init reaps it, and PostgreSQL refuses to start while either lock file names a live process. `stack.sh kill` waits for it to die and removes both lock files.
7. **Store refusals share a stream with the participant.** Wasmtime writes its trace to the container's stderr, where the participant could print lines that look like it. The importer therefore distrusts any trace in which the participant opened stderr; ours never does. Wasmtime 49's `-D log-to-files` wrote no file in this setup, so the trace could not be moved out of the participant's reach.
8. **The journal trusts the source its caller names**, as on Golem. Every service uses one PostgreSQL role; no participant can reach PostgreSQL at all.
9. **The participants are not the Golem build's binary.** Both builds compile the shared `scenario-task` crate, here into one WASI command component run under Wasmtime, on Golem into agent types (Golem finding 15). The plan wants one compiled component on both platforms; that is still open.

## Independent review and known limits

One fresh-context reviewer read the authorization and recovery slice (the services, the container runner, the schema, the stack script) and the harness refactor before the recorded run. It found nothing major. The findings and how each was handled:

| Finding | Severity | Disposition |
| --- | --- | --- |
| The approval service's steps were not atomic: a crash between journaling a proposal and storing it, or between an approval and its mint, left a run unable to propose or an approval never minted; nothing started a delivery whose start was lost. | Minor | Fixed. Journal commands and the service's rows commit in one transaction under the journal lock, and minted authorizations go through an outbox. |
| The delivery activity ignored Temporal's cancellation, so an attempt that outlived its heartbeat timeout could submit alongside its retry. | Minor | Fixed. The attempt stops at its next step when cancelled. A step already in flight (a submit on the wire) cannot be recalled; the journal's records show it. |
| Revoke, cancel and close did not reach containers already running. | Minor | Fixed. Each stops the containers whose holders' grants changed; `stack.sh kill` stops participant containers too. |
| Fixed reservation identifiers meant a second run of a step was not charged. | Minor | Fixed. Each step reserves under its workflow's identifier, stable across retries. |
| A proposal could be submitted after the drafter's submit-proposal grant was revoked mid-step. | Minor | Fixed. The approval service checks the grant under the journal lock and journals a refusal. |
| The README overstated the credential's isolation. | Minor | Fixed (wording). |
| The before-and-after check compared the journal with itself and did not look at container flags. | Minor (harness) | Fixed. It now creates each participant's container without starting it and compares Podman's own report with a fixed copy of the plan's grant table, plus no network, a read-only root, no added capabilities and no privilege escalation. |
| A participant could print fake trace lines to pass test 1's evidence check. | Minor (harness) | Mitigated (finding 7). |
| Test 7a's "the budget reservation is released" reads the journal's flag only: the send's reservation is never drawn from the budget ledger on either build. | Minor (harness, both builds) | Open, recorded here. It predates this build and applies to the Golem results too. |
| Small items: the draft output journaled again on retry, no timeouts on mail calls, the lock-file removal after a timeout, a proxy-aware HTTP client. | Nit | Fixed. A delivery workflow without continue-as-new would reach Temporal's history limit only after weeks of status checks; left open. |

The Golem path of the refactored harness was compiled and read through by the reviewer, who found it behaviourally unchanged; it was not re-run, because this environment has no Golem server.

## Custom machinery

Counted with `tokei` 12.1.2 (code lines: non-blank, non-comment) on the tagged files, with the same rules as the Golem count. Configuration lines are non-blank, non-comment lines of `schema.sql` and the `Containerfile`, kept separate. Every counted file opens with a `Scorecard:` line naming its category and the tests it serves.

### Code, per category

| Category | Files | Code lines |
| --- | --- | ---: |
| Grants (journal hosting, budget, coordinators, supervisor, native mounts) | `journal.rs` 147, `coordinator.rs` 149, `supervisor.rs` 119, `containers.rs` 78 of 88, `common.rs` 25 | 518 |
| Approvals (proposals, display, payload binding, minting, the outbox to the adapter) | `approval.rs` | 202 |
| Recovery (consumption and receipt integration, retried and superseded attempts, status-check scheduling, unknown outcomes across close) | `adapter/delivery.rs` 128 of 136, `adapter/mod.rs` 95, `adapter/mock_mail.rs` 81 of 86 | 304 |
| Inspection (receipt layer, experimental interface, importing the store's refusals) | `harness/src/inspector.rs` 186 of 210, `harness/src/wasmtime_trace.rs` 28 | 214 |
| **Total counted code** | | **1,238** |

Not counted, as on Golem: the process wiring and command line in `main.rs` (70; the Golem build's `lib.rs` was 15), the harness hooks (the checkpoint hook, 5 in `mock_mail.rs` and 7 call sites plus the trait method in `delivery.rs`; the container probe, 10 in `containers.rs`), the participant wrapper (`participant/src/main.rs` 77, task code), `stack.sh` (operations) and the Golem-only oplog parser in `inspector.rs` (24). Counting `main.rs` under grants would make grants 588; it does not change the result below.

### The adapter as a developer-model test

| Part | Reusable for any adapter contract | Specific to mock mail |
| --- | ---: | ---: |
| `adapter/delivery.rs` (the protocol and the `Contract` trait) | 128 | 0 |
| `adapter/mod.rs` (the workflow, retries, status-check scheduling) | 81 | 14 (credential and validity settings) |
| `adapter/mock_mail.rs` (submit and status over HTTP) | 0 | 81 |
| **Total** | **209 (69%)** | **95 (31%)** |

### Configuration, per category

| Category | Where | Lines |
| --- | --- | ---: |
| Grants | `schema.sql`: journals 4, budgets 6, reservations 11; `Containerfile` 5 | 26 |
| Approvals | `schema.sql`: proposals 10, deliveries (the outbox) 7 | 17 |
| Recovery | None: the retry and heartbeat settings are code in `adapter/mod.rs` | 0 |
| **Total counted configuration** | | **43** |

### Components and hours

- **Components we wrote and deploy:** three services from one binary (coordinator worker, mail adapter, approval service), the participant component and its container image.
- **Components we only operate:** PostgreSQL, the Temporal dev server, Podman and Wasmtime. Five long-running processes against Golem's one.
- **Operational burden, clean host to first passing run:** PostgreSQL was preinstalled. Podman and `protoc` came from apt. Wasmtime built from crates.io in about six minutes. The Temporal CLI's release download was refused by this host's network proxy, so it was built from its Go module source (v1.9.1); that took three attempts and about ten minutes, because the module's `replace` directive blocks `go install` and a sibling module had to be fetched by commit. Two of our own bugs were found on the first by-hand run (a serde tag name and a file mount created as a directory), and the PostgreSQL lock-file problem on the first crash. From the start of the session to the first by-hand delivery took about 32 minutes and to the first passing harness test about 42, but that time includes writing the build, so it is not a setup-only figure.
- **Hours:** not logged per category. Like the Golem build, this one was written by one agent session (2026-10-27, from about 02:53 UTC), harness changes and recorded run included. Neither build's elapsed time measures a human team's effort.

## Early-exit check

Jason fixed the thresholds on 2026-10-20, before either build was measured (experiment plan, "Early exit"). On the source-code measure, per category:

| Category | Golem | Conventional baseline | Golem as a share of the baseline | Threshold | Met |
| --- | ---: | ---: | ---: | --- | --- |
| Grants | 465 | 518 | 90% | At most 50% | No |
| Approvals | 93 | 202 | 46% | At most 50% | Yes |
| Recovery | 358 | 304 | 118% | At most 50% | No |
| Inspection | 210 | 214 | 98% | At most 50% | No |
| **Total** | **1,126** | **1,238** | **91%** | | |

Approvals is the one category Golem meets, and only since the review: before the baseline's approval service was made crash-safe (finding 3) it was 140 lines and Golem's share 66%. Durable calls are where Golem saves the most code.

- **Golem-specific custom grant code must be zero.** Golem needed 465 lines. Not met.
- **Matched zeros:** no category has zero baseline-specific code, so none applies.
- **Reported separately, outside the ratio:** configuration (Golem 170 lines, baseline 43), operated components (Golem one server, baseline four systems and five processes), hours (not comparable), and the shared reference implementation (1,457 lines of source, 424 of tests), which both builds use unchanged.

**Verdict.** Golem passed all seven acceptance tests, but it misses the effort threshold in three of the four categories (grants, recovery and inspection; it meets it only for approvals) and the zero-grant-code rule: preserving the plan's rules on Golem took about as much custom code as building them on a conventional composition. By the plan's table, that result is "a candidate OrcaOS contribution, stated as 'preserving these rules required this much custom machinery; OrcaOS makes them reusable platform behaviour'" (OrcaOS is Diamond Dog's former name). The early exit (an interface or integration on Golem instead of a runtime) is not supported. Missing a threshold does not justify a runtime by itself, and the usability threshold is still unscored until the hands-on evaluation runs.
