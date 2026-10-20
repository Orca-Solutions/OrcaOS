<!--
SPDX-FileCopyrightText: The OrcaOS Authors
SPDX-License-Identifier: GPL-3.0-or-later
-->

# Scenario A on Golem

The deterministic Scenario A experiment from the OrcaOS experiment plan, built on a properly restricted Golem 1.6.0. It covers the mock mail service, the deterministic processors and drafter, the trusted mail adapter, the failure-injection harness with facilitator controls, and the seven acceptance tests plus the test 3 variant (revocation during the enforcement check at P1 and P2).

It does not cover the AI participant, the conventional baseline or any OrcaOS runtime. Results are stated narrowly: a pass means Golem plus the code here passed this conformance suite under the recorded assumptions. It does not mean the build is secure.

- Results: [results/2026-10-20-golem/summary.md](results/2026-10-20-golem/summary.md), with the reproducibility record, card lists, every repetition, the journals and the mock service logs next to it.
- Scorecard: [Custom machinery](#custom-machinery) below.
- Findings about Golem: [Findings](#findings) below. Several of them change how the plan's configuration rules could be followed.

## Layout

| Path | What it is | Scorecard |
| --- | --- | --- |
| `shared/orca-ref/` | The shared reference implementation: request identifiers, payload binding, the journal with consume-if-live, receipts on two axes, the recovery decision. No I/O, no clock. 18 tests. | Reported separately |
| `shared/scenario-task/` | Task logic shared by the processors and the drafter: digests and the composed message. | Excluded (task code) |
| `shared/synthetic-inputs/` | Fabricated contracts `c1@v1`, `c2@v1`, `c3@v1` and the decoys `c1@v2` and `tax-return-2025`. No real personal data. | Excluded |
| `mock-mail/` | The mock mail service. Port 18081 speaks the mail contract; port 18082 is the harness's control port (logs, fault switches, checkpoints). | Excluded (shared) |
| `golem/` | The Golem application: `golem.yaml` (cards, retry policy) and one component with ten agent types. | Counted, per file tag |
| `harness/` | Suite runner, facilitator controls, inspector, reproducibility record and card check. | Excluded, except `inspector.rs` |
| `results/` | One directory per recorded suite run. | Not code |

Every counted file opens with a `Scorecard:` line naming its category and the tests it serves. The adapter files also say whether they are reusable for any adapter contract or specific to the mock mail contract.

### Agents in the Golem component

| Agent type | Role | Authority (initialCard) |
| --- | --- | --- |
| `Journal(run)` | The journal writer, the single point of order. Hosts `orca-ref`'s journal and the shared budget ledger. | Its own environment only |
| `Supervisor(run)` | Acts for `principal-1`: opens activities, records the grant table, revokes, cancels, closes, replaces participants. | Invoke Journal; derive and install cards; write the `revoked` bucket's `none` key |
| `Activity(run, id)` | One coordinator per activity (A1, A1.1, A1.2). Checks the journal before running a participant and reserves budget. | Invoke Journal and the participants |
| `Processor1(run, holder)` | processor-1 in A1.1. | Read `docs.c1@v1`, `docs.c2@v1`; write `out-A1-1` |
| `Processor2`, `Processor2Alt` | processor-2 in A1.2 and its replacement for test 5. | Read `docs.c3@v1`; read and write `out-A1-2` |
| `Drafter(run, holder)` | Reads both outputs, writes the draft, submits a proposal. | Read `out-A1-1`, `out-A1-2`; write `out-A1`; invoke `ApprovalPath.submit` |
| `ApprovalPath(run)` | Displays the proposal with its payload hash, records approval, mints the authorization and hands it straight to the adapter. | Invoke Journal; invoke `MailAdapter.deliver` |
| `MailAdapter(run)` | The trusted adapter, the only holder of the mail credential and the only agent with a network card. | Invoke Journal; schedule its own `reconcile`; the credential; connect to 127.0.0.1:18081 and :18082 |
| `Seeder(run)` | Harness infrastructure: loads the inputs, reads stores back for hashes. Not a participant. | Read and write `docs`; read the output buckets |

Every agent takes the run identifier as its first constructor argument, so repetitions are separate agents and never share state. The mock service namespaces request identifiers by the `x-orca-run` header for the same reason.

### The delivery path

1. The drafter submits a proposal; the approval path records it with its payload hash.
2. The facilitator approves with the hash it was shown. The approval path journals the approval, mints a single-use authorization bound to the hash of the canonical payload plus operation, adapter, activity and expiry, and calls the adapter. No participant ever holds it.
3. The adapter checks the binding and asks the journal whether the authorization is live (the early check, which can only refuse early).
4. Inside one custom durable operation it asks the journal to consume the authorization if it is still live. Only when that consumption has committed does it submit, then journal the submission and the receipt.
5. If the host fails inside that operation, Golem re-runs it live. The journal answers "already consumed" for the same delivery token, which puts the adapter on the recovery path: status check first, then re-dispatch only if the journal still allows it (activity open, grant live, operation idempotent), otherwise status checks and receipt updates only.
6. An outcome the adapter cannot settle gets a scheduled status check under its standing reconciliation grant, which outlives cancellation and close. Scheduled checks never re-dispatch.

## Running it

The recorded runs used one Ubuntu 24.04 host with the Golem 1.6.0 single binary (`golem server run`), Rust with the `wasm32-wasip2` target, and the paths below. The harness reads `ORCA_GOLEM_APP`, `ORCA_GOLEM_CLI`, `ORCA_GOLEM_DATA` and `ORCA_RUN_DIR` if the defaults don't fit.

```sh
# 1. Golem server (local, single binary)
/opt/golem/golem server run --data-dir /opt/golem/data &
echo $! > /opt/golem/run/golem.pid

# 2. A random mail credential, never committed
head -c 24 /dev/urandom | base64 | tr -d '/+=' > /opt/golem/run/mail-credential
chmod 600 /opt/golem/run/mail-credential

# 3. Mock mail service
cd experiment
cargo build -p mock-mail -p harness
MOCK_MAIL_CREDENTIAL="$(cat /opt/golem/run/mail-credential)" \
  ./target/debug/mock-mail --log-dir /opt/golem/run/mock-logs &

# 4. Golem component, and the credential as a Golem secret read from stdin
cd golem
golem -Y build && golem -Y deploy
golem -Y secret create mailCredential --type String --value-stdin --update-existing < /opt/golem/run/mail-credential
cd ..

# 5. The suite: 20 repetitions of the crash and race tests, 100 of the budget race
./target/debug/harness suite --reps 20 --race-reps 100 --prefix full1 --out results/<date>-golem
```

The suite kills the Golem server with SIGKILL and restarts it for every crash repetition, so nothing else should be using that server. Tests 7a, 7b and 7d end by waiting for the 10-minute validity window to close and checking the certified not-started outcome; a full run takes about an hour and a half.

Facilitator controls for the hands-on evaluation are under `harness ctl RUN OP` (setup, process, show, approve, revoke, grant, cancel, close, replace, fault, clear, hold, release, crash, pending, journal), and `harness inspect RUN` prints the inspector's answers.

## Grant table to Golem configuration

The journal holds the grant table (`golem/src/supervisor.rs`, `GRANT_TABLE`) and is what the adapter's enforcement check and the activity coordinators consult. Golem's cards enforce the store and network side natively. The mapping:

| Plan grant | Journal grant | Golem enforcement |
| --- | --- | --- |
| processor-1 reads `c1@v1`, `c2@v1` | `g-p1-read-c1`, `g-p1-read-c2` | `Processor1` initialCard: `kv : read : docs.c1@v1`, `docs.c2@v1` |
| processor-1 appends to `out/A1.1` | `g-p1-append` | `Processor1`: `kv : write : out-A1-1.*` |
| processor-2 reads `c3@v1` | `g-p2-read-c3` | `Processor2`, `Processor2Alt`: `kv : read : docs.c3@v1` |
| processor-2 appends to `out/A1.2` | `g-p2-append` | `kv : read, write : out-A1-2.*` (the step state lives there too) |
| drafter reads `out/A1.1`, `out/A1.2` | `g-d-read-a11`, `g-d-read-a12` | `Drafter`: `kv : read : out-A1-1.*`, `out-A1-2.*` |
| drafter writes `out/A1/draft`, submits a proposal | `g-d-write`, `g-d-propose` | `Drafter`: `kv : write : out-A1.*`; `agent(ApprovalPath) : invoke : submit` |
| Mail adapter sends as `principal-1@mock.test` | `g-send` (held by A1; the adapter acts only on an authorization minted from an approval) | `MailAdapter`: the credential secret, `network : connect : 127.0.0.1:18081` |
| Mail adapter's standing reconciliation grant | The adapter's status checks and receipt updates on its own request identifiers | `MailAdapter`: invoke its own `reconcile`; the journal refuses anything but receipts and reconciliations once the grant or activity is gone |
| Processors have no network | — | No network grant in any participant card |
| A1 promotes one object through the storage service | Not built | No acceptance test reads promotion; not attempted |

`harness cards RUN` lists every agent's cards and compares them with `golem.yaml`; the suite does this before and after, and stops if any card grants something undeclared. Child grants expire after 24 hours in the journal. In Golem they live only in each agent type's initialCard, for the reasons in Findings.

## Findings

These are what the build found about Golem 1.6.0 (binary sha256 `499ba404…8fcc8`, golem-rust 3.0.0). Each was found by probing with a throwaway component and then confirmed in this build. Several mean a line of the plan's Golem configuration rules could not be followed as written; where that happened the build says so rather than substituting silently.

### Cards

1. **Installing a card narrows; it does not add.** An agent's authority is the intersection of what it holds: installing a card into an agent capped its wallet at the new card's upper bound. An agent that could read `c3` lost that read when a card for `c1` was installed. So a parent cannot hand a child extra authority at run time by installing a card.
2. **Derived cards reach other agents with an empty lower bound.** A card derived for another agent came back with only its upper bound (recipient `<component>/*`); its `lowerPositive` was empty. Deriving needs the upper bound's recipient to equal the wallet's (`?component/*`): a narrower one fails with `UpperBoundTooBroad`, `*` with `LowerBoundTooBroad`.
3. **A derived card cannot be derived from again** by anyone who does not hold it in their own wallet, and an agent that installed a card into itself lost its card-derive and revoke abilities.
4. **A card lent for one call replaces the callee's authority for that call.** The callee lost its environment read and failed on `GOLEM_AGENT_ID`.
5. **Consequence for the plan's rules 1 and 4.** Child activities cannot hold cards derived from the parent's, and revoking a parent card does not cascade to what was "derived" from it, because nothing usable was derived. The build gives each participant its authority from its agent type's initialCard (lower bound equal to upper bound, nothing beyond the grant table), and revokes a participant natively by installing a narrowing card from the Supervisor whose only grant is `kv : read, write : revoked.none`. Because cards narrow, that leaves the participant with no store access (test 5 checks this). The journal's parent links and 24-hour expiry are our code.

### Stores and secrets

6. **Key-value grants are whole-bucket or exact-key.** `bucket.*` and exact keys worked; `out.a11/*`, `out3.a11-**` and `out4.A1.1/**` were all denied. Output spaces are therefore one bucket each (`out-A1-1`, `out-A1-2`, `out-A1`).
7. **A secret grant cannot be narrowed to one secret.** Only `secret(?env) @ ?agent : hold : *` with `reveal : *` let the adapter read its credential. Naming the secret (`mailCredential`) or its id did not work. In this build the adapter is the only agent with any secret grant, so the effect is contained, but a second secret in the environment would be readable by it too.
8. **Grant strings that worked:** `network() @ ?agent : connect : 127.0.0.1:18081` (empty owner) and `config(?agent) @ ?agent : read : mail_credential` (the snake-case field name). `config(?env)` is rejected.
9. **Denials are recorded by Golem**, as `denied:` lines in the agent's oplog for refused store and network calls. Golem does not write them anywhere our journal can see, so the inspector reads them from the oplog and journals them with Golem's store as the source (test 1).

### Recovery

10. **Golem resends an in-flight HTTP request on recovery**, with the same `idempotency-key` header, as its documentation says. Left alone, that would be a re-dispatch after cancellation or revocation. The build stops it by running everything from consumption to the receipt inside one custom durable operation (`Durability::...with_forced_commit()`), which Golem re-runs live after a crash instead of replaying recorded effects. The journal then answers "already consumed" and the adapter takes the recovery path. Tests 7a to 7d read the mock's attempt log, which is written before deduplication, and found no attempt after the cancellation or revocation in any repetition. This is our code, counted under recovery; it is not configuration.
11. **Golem's own retries were turned off** for the mock service with a `never` retry policy on uri-host 127.0.0.1 in `retryPolicyDefaults`, so the adapter alone decides retries.
12. **Golem resumed an interrupted agent about 30 seconds after a restart.** That delay is inside every crash repetition's elapsed time.

### Budgets and components

13. **Golem's quotas are invocation-scoped**, so they cannot express a budget shared by two child activities. The shared CPU budget is a reservation ledger in the Journal agent (test 6). It reserves declared CPU-milliseconds; it does not meter actual CPU.
14. **Processors run as agents, not Golem tools** (a deviation from rule 3). Tools need separate registration and bindings that this build did not get to; agents with explicit initialCards give the same store narrowing, but each call is not a fresh Wasm instance.
15. **The processors are not the same Wasm binary** the conventional build would use. They are agent types in this component, built from the shared `scenario-task` crate. The plan wants one compiled component on both platforms; that will need a component both can load.

### Smaller observations

- The oplog stores message payloads in full. The data here is synthetic; with real data, the oplog would need the same handling as the mail itself.
- A1 stays Waiting after the send is acknowledged; no step in the deterministic scenario moves it on. Promotion and the close that would follow were not built because no acceptance test reads them.
- A failed agent refused all further invocations until deleted with `golem agent delete`.

## Custom machinery

Counted with `tokei` 12.1.2 (code lines: non-blank, non-comment), on the tagged files at the commit the results came from. Configuration lines are non-blank, non-comment lines of `golem.yaml`, kept separate.

### Code, per category

| Category | Files | Code lines |
| --- | --- | ---: |
| Grants (journal hosting, budget ledger, activity coordinators, supervisor) | `journal_agent.rs` 158, `supervisor.rs` 181, `activity.rs` 109, `common.rs` 17 of 31 | 465 |
| Approvals (display, payload binding, minting, delivery to the adapter) | `approval.rs` | 93 |
| Recovery (consumption and receipt integration, stopping Golem's resend, status checks, unknown outcomes across close) | `adapter/delivery.rs` 172, `adapter/mod.rs` 103, `adapter/mock_mail.rs` 83 | 358 |
| Inspection (receipt layer, experimental interface) | `harness/src/inspector.rs` | 210 |
| **Total counted code** | | **1,126** |

Not counted: `lib.rs` module wiring (15), the store helpers in `common.rs` (14, task I/O), the harness checkpoint hook in the adapter (5 in `mock_mail.rs`, 7 call sites and the trait method in `delivery.rs`).

### The adapter as a developer-model test

| Part | Reusable for any adapter contract | Specific to mock mail |
| --- | ---: | ---: |
| `adapter/delivery.rs` (the protocol and the `Contract` trait) | 172 | 0 |
| `adapter/mod.rs` (the agent, scheduling, pending outcomes) | 89 | 14 (credential and validity settings) |
| `adapter/mock_mail.rs` (submit and status over HTTP) | 0 | 83 |
| **Total** | **261 (73%)** | **97 (27%)** |

A second adapter would implement `Contract` (system, operation, idempotency, validity window, payload hash, submit, status) in roughly the size of `mock_mail.rs`, and reuse the rest.

### Configuration, per category

| Category | `golem.yaml` sections | Lines |
| --- | --- | ---: |
| Grants | Journal 10, Supervisor 18, Activity 20, Processor1 16, Processor2 16, Processor2Alt 16, Drafter 18 | 114 |
| Approvals | ApprovalPath | 14 |
| Recovery | MailAdapter 24, retry policy 7 | 31 |
| Application boilerplate | app, environment, component, `agents:` | 11 |
| **Total counted configuration** | | **170** |

Excluded: the Seeder (20, harness). Golem 1.6 needs both a lower and an upper bound on each initialCard; they are identical here, so about half of the grant lines are the same list twice.

### Separately reported and excluded

| Part | Code lines | Status |
| --- | ---: | --- |
| `shared/orca-ref` (source) | 1,457 | Reported separately: what OrcaOS would have to maintain |
| `shared/orca-ref` (tests) | 424 | Reported separately |
| Processors and drafter (`participants.rs` 150, `scenario-task` 77) | 227 | Excluded (shared task code) |
| Mock mail service | 512 | Excluded (shared) |
| Harness (excluding the inspector) and Seeder | 1,213 | Excluded |

### Components and hours

- **Components we wrote and deploy:** one Golem component (`scenario-a:main`, ten agent types), plus the mock mail service and the harness, which are excluded.
- **Components we only operate:** the Golem server, run as a single local binary with its built-in storage.
- **Hours:** not logged per category. The build was written by one agent session on 2026-10-20 from about 13:50 UTC, including probing Golem's card semantics; agent hours are not comparable to the conventional build unless that is built the same way.

## Independent review and known limits

One fresh-context reviewer read the authorization and recovery slice (the reference journal, the adapter, the approval path, the journal agent, the supervisor, the manifest and the test checks) before the recorded run. The findings and what was done:

| Finding | Severity | Disposition |
| --- | --- | --- |
| The send grant's recipient was never compared with the message, so an approved proposal to another address would have been sent. | Major | Fixed. Minting takes the operation's target and refuses one outside the grant's object; a reference test and a test 2 check cover it. |
| Test 7's "no attempt after the cancellation" window started when the harness's call returned, after the cancellation had committed. | Major (harness) | Fixed. Attempts and wire entries are now checked from the crash onwards, which is stricter. |
| The harness could release a recovery hold before acting, and did not check that the action committed before the adapter's first status check. | Minor (harness) | Fixed. Only the stale crash-point hold is released, and each variant checks that order. The Golem server must be up to record a cancellation, so the action happens after the server restarts but while the adapter is held at the start of its recovery path. |
| A certified non-start was recorded as a reconciliation to "certified not started" rather than to Failed, and the journal accepted a certificate before the window closed. | Minor | Fixed. It is now a reconciliation to Failed (certified not started), and the journal ignores a certificate before the window closes by its own clock. |
| Re-dispatch did not check the authorization's expiry; a server error on submit was treated as a refusal; status checks stopped after 20 tries. | Minor | Fixed. Expiry is checked, a 5xx reply leaves the outcome Unknown for a status check to settle, and checks back off to every 5 minutes instead of stopping. |
| The journal trusts the source its caller names, and any agent that can invoke the journal could post any command. | Minor | Open. Only trusted services (Supervisor, Activity, ApprovalPath, MailAdapter) hold a card that reaches the journal; no participant does. Golem does not hand the callee its caller's identity in a form this build used, so binding commands to callers would be custom machinery. |

## What was not built

The AI participant and its model-provider grant, the conventional baseline, promotion to the principal's workspace, the parity review of both builds, and the performance measurements (startup latency, peak memory, oplog growth, the no-controls baseline). The independent review of the enforcement design that the scorecard names is also outstanding; the code review above is narrower.
