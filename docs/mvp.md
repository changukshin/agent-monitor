# Agent Monitor — MVP specification

Status: Draft. This document defines the intended v0.1 scope and acceptance criteria, not verified implementation capabilities.

## 1. Goal and scope

Observe multiple existing Codex CLI and Claude Code sessions in a single local dashboard without changing how users launch or interact with their agents. Target a single Windows user with native Windows and WSL2 sessions. No orchestration, remote access, multi-host synchronization, dedicated hardware, database, or persistent session history is required.

The smallest useful release must reliably show progress, human attention, and turn completion for the supported provider versions and environments. Unsupported states must be explicitly identified. If the essential states cannot be observed reliably, stop and propose a scope revision rather than claiming support or inventing an inference engine.

## 2. Feasibility gate — before production implementation

Complete a small, isolated technical spike before building the production collector or dashboard. Keep experimental code disposable. Record the following in an English evidence report:

- Exact Codex CLI, Claude Code, Windows, WSL distribution and networking-mode versions/configurations tested.
- Official documentation references and sanitized real event fixtures. Distinguish CLI from desktop applications; desktop support is out of scope.
- A provider/version support matrix for session start/end, turn start/completion/failure/interruption, actual approval/input pending and resolution, and idle. For each state record evidence, whether it is confirmed, provisional-only or unsupported, and how later events invalidate earlier observations.
- Compare observed events against the actual agent behavior. Exercise normal completion, a Stop hook that causes continuation, approval requested but resolved without human interaction, blocked prompt submission, tool failure followed by recovery, user input, interruption and session exit where supported. Use isolated test sessions and existing or temporary test hooks; never alter real project approval policies.
- Validate the single-collector transport in section 4 and demonstrate non-interference under the failure cases in section 5.
- Record unsupported or untested combinations, rather than treating documentation or simulated fixtures as live proof.

Hook names such as Stop, PermissionRequest, UserPromptSubmit and PostToolUse are examples to investigate, not a universal mapping or a promise of availability. A Stop invocation may precede continuation; a permission request may be resolved by another hook; prompt submission may be blocked; tool completion is not turn completion. Use an officially supported read-only observation interface in addition to hooks only if necessary and validated. Do not require users to relaunch agents under a wrapper solely to monitor them.

**Gate:** do not proceed with a full implementation until both target providers have a demonstrated useful observation path and a single local collector can receive events from native Windows and at least one tested WSL2 mode without altering agent behavior. Reliable confirmation of core states (running, actual human attention, completion) is the release goal; gaps in these states require an explicit maintainer decision before narrowing that goal. A mode not tested is not supported by default. No requirement to change a user's networking configuration to pass the gate.

## 3. Observation and state contract

### 3.1 Normalized evidence

For provider-native snapshots, correlate by provider, runtime/host and provider session ID. The caller supplies host identity and sample time. A Windows observer invoked from WSL still observes Windows. The sample carries provider-reported status, session kind or active flags. A failed query conveys loss of visibility. Absence from the next list does not establish exit. A later status supersedes the earlier status only for the same session.

Confirm an outcome only from evidence tied to that turn. Confirm an attention wait or resolution only when its kind and request can be distinguished. Keep hook receipts provisional. The native listing can operate independently of direct HTTP and the WSL relay. See the [Phase 1B validation and scope decision](phase-1-feasibility.md#phase-1b--provider-native-observation).

The following is an illustrative internal event, not a provider payload or a frozen wire schema:

```json
{
  "schemaVersion": 1,
  "eventId": "unique-id",
  "source": "codex",
  "hostId": "local-wsl-distribution",
  "sessionId": "provider-session-id",
  "turnId": "optional-provider-turn-id",
  "type": "turn.started",
  "confidence": "confirmed",
  "occurredAt": "2026-11-26T05:00:00.000Z",
  "cwd": "/home/user/project"
}
```

- A session key combines source, hostId and sessionId. hostId distinguishes the native environment and each WSL distribution; it is not a public hostname or an account identifier.
- Use a provider turn ID when available. Never invent an ambiguous association between observations and turns.
- Candidate evidence types include session.started, turn.started, tool.started, tool.finished, approval.requested, approval.resolved, input.requested, input.received, turn.completed, turn.failed, turn.interrupted and session.ended. Final mappings and any additional evidence types depend on the spike.
- A confirmed observation establishes the stated transition according to the verified provider/version contract. A provisional observation is a signal whose effect is not yet established. Unknown means the current state cannot be established. Unsupported evidence must not be manufactured.
- Allowlist only required metadata. Raw hook payloads may be parsed transiently to extract allowed fields but must never be logged, stored or forwarded. Do not collect prompt or response content, commands, tool arguments, environment variables or credentials. Optional task labels are explicitly user supplied, never derived from transcript content.

### 3.2 Reducer

Keep the model small: one current session record, a status, confidence, last observed time, and only the correlation metadata needed for correct transitions.

| Confirmed evidence | Result |
| --- | --- |
| Session established and ready with no active turn | IDLE |
| Turn is actually in progress | RUNNING |
| Human approval is actually pending | WAITING_APPROVAL |
| Human input is actually pending | WAITING_INPUT |
| Wait resolved and execution confirmed to resume | RUNNING |
| Turn ended normally | COMPLETED |
| Turn explicitly failed | ERROR |
| Turn explicitly interrupted | INTERRUPTED |
| Session explicitly ended | ENDED |

- A resolution without evidence of the next state yields UNKNOWN. Session registration alone does not establish an active turn or an idle state.
- Provisional evidence may display a clearly marked candidate state or last-known status with uncertainty. It must not trigger confirmed counts or attention/completion notifications. An ambiguous or contradictory observation invalidates certainty when it could affect the current state; do not preserve an apparently confirmed status after losing the evidence for it.
- A completed turn is distinct from a closed session, successful implementation, passing tests or deployment. A failed or completed individual tool does not determine the whole turn. Avoid double-counting parent sessions and subagents.
- Deduplicate retransmitted event IDs. Late events from earlier turns or session lifetimes must not overwrite newer states. Use verified correlation and ordering information; when ordering cannot be established, mark the relevant state uncertain rather than guessing from timestamps alone.
- Record the time of the last observation and state change separately. Silence alone proves neither completion, failure nor termination. If reliable liveness is unavailable, show observation age and uncertainty; do not fabricate a heartbeat.
- If collection is interrupted or evidence is lost, mark the affected current state unknown or uncertain. A collector restart clears in-memory state; subsequent valid events can register existing sessions without requiring a new SessionStart.
- Retain a bounded in-memory record set. Prefer ended and stale records for eviction, and define an explicit cap and stale visibility policy before release. Hide ended sessions by default; allow old observations to be hidden separately from confirmed termination. Expiry/eviction is a display/storage policy, not a state transition. No durable session history in v0.1.

## 4. Single-collector transport

Prefer one Windows-hosted collector bound to loopback (default http://127.0.0.1:8787). Native Windows hooks can send directly; WSL2 hooks should first evaluate a Windows-side Rust helper invoked through WSL interoperability, with the sanitized event passed via stdin rather than shell interpolation or command-line secrets. This is a candidate, not a mandatory implementation.

| Environment | Required investigation |
| --- | --- |
| Windows native | Direct loopback ingest and local dashboard access |
| WSL2 NAT | Verified local path to the same collector, preferably interop |
| WSL2 mirrored networking | Verified local path to the same collector; direct loopback or interop if appropriate |
| WSL2 with interop disabled or unavailable | Document limitation or validate an equally safe alternative |

Verify addressing, token access, path handling, quoting, latency and timeout behavior. Report each environment as tested/supported, unsupported or untested. The spike selects the simplest safe transport; do not introduce another listening bridge or multiple collectors as a default workaround. Do not change WSL network mode, firewall rules, interop settings or system permissions, and do not bind to all interfaces. If no safe path exists, report the blocker.

## 5. Non-interference and hook lifecycle — release blocker

Monitoring is observational only. It must not change tool authorization, prompt handling, stop decisions, agent output or continuation behavior.

- Validate the neutral output and exit-code contract for every installed hook type and tested provider version. Do not assume empty stdout, exit code zero or one shared JSON response is neutral for every event. Do not emit extra model context or permission decisions.
- Set and document a small end-to-end deadline for each hook delivery attempt, including helper startup and network I/O. Target no more than 500 ms of added blocking per event, including on failure; verify on real environments and escalate if infeasible. Use no synchronous retries or unbounded child processes. Drop failed delivery rather than block agent execution.
- Fail open when the collector is absent, hung, slow, rejects input, or cannot be reached, and when parsing fails or the transport helper is unavailable. Failure must not surface as a provider hook veto, injected context, or changed permission decision.
- Preserve existing hooks, ordering and unrelated settings. Use uniquely identifiable owned entries; do not replace an entire hook group. Inspect and validate settings before editing, back up the original securely, and use safe writes with change detection. If a conflict or unsupported format is detected, stop rather than overwrite.
- Installation and removal must be idempotent. Removal deletes only monitor-owned entries, preserves changes made after installation, and does not restore a stale whole-file backup over current settings. Document manual cleanup for partial failures.
- Validate behavior with and without the monitor installed and with other hooks present. Compare observable agent actions, output/decision behavior and timing, including real collector-down and timeout cases. Do not enable monitoring in an existing working configuration until this is demonstrated.

## 6. Local API and security

| Method | Path | Purpose |
| --- | --- | --- |
| GET | /api/sessions | Current snapshot |
| GET | /api/events | SSE snapshot and updates |
| POST | /api/ingest | Authenticated normalized evidence |
| GET | /healthz | Minimal collector health |

- Bind only to the verified local loopback endpoint. Restrict Host to explicit local host/port values; reject foreign origins on all routes, including read and SSE routes. Allow native local requests without an Origin header where appropriate. Reject browser origins for the native client API unless a separate supported browser integration is explicitly approved. Do not enable cross-origin reads or wildcard CORS. Protect metadata from DNS rebinding and cross-site access; use appropriate same-origin response headers. This is a single-trusted-local-user design, not isolation from malicious processes running as that user.
- Authenticate ingest with a cryptographically random per-installation token passed in an authorization header. Store it outside the repository in a user-private configuration file with restrictive Windows ACLs or Unix permissions as appropriate. Document creation, access from both environments, rotation and revocation. Never include the token in URLs, hook configuration command lines, event bodies, UI, or logs. Rotation invalidates the previous token; no login, OAuth or JWT service is needed.
- Validate event schemas, field lengths and request sizes; bound retained records, deduplication metadata, and pending SSE data. Handle slow/disconnected SSE clients without unbounded buffering. Do not expose sensitive diagnostic details in health responses.
- SSE connections receive a current snapshot on connection/reconnection, with subsequent updates ordered consistently. Reconnecting must not duplicate notifications for old transitions.
- Render agent metadata as text, not executable HTML. Do not implement command execution, approval, or control endpoints.

## 7. Dashboard

Open a native Rust desktop dashboard on Windows. The collector remains usable without the dashboard. A lightweight native Rust GUI toolkit may be selected during the dashboard phase; no embedded browser or JavaScript is required. Show Running, Needs attention and Completed counts for confirmed current records only, plus an explicit uncertain/unknown count or indicator. Show actual approval and input waits separately. Display provider, native/WSL environment, project name, optional user label, status, confidence, last observed time and elapsed time where meaningful. Clearly label stale or provisional evidence and prioritize confirmed attention requests. Use labels and icons as well as color.

Provide simple filters and opt-in native desktop notifications only for newly confirmed attention/completion transitions; suppress historical, provisional and duplicate notifications. Document OS notification limitations. Window activation, system tray, always-on-top, persistent history and external integrations are deferred.

## 8. Implementation sequence

1. **Feasibility spike:** produce the evidence report and support matrix in section 2. Resolve or explicitly escalate blockers before production implementation.
2. **Collector and reducer:** Rust, Cargo, verified adapters, bounded memory, authenticated local API and SSE or an equally small Rust-native local update stream.
3. **Dashboard:** native Rust GUI, live updates, confirmed counts and optional desktop notifications.
4. **Release:** safe hook installation/removal, documented supported versions and environments, CI and end-to-end evidence.

Prefer a small Rust source layout (probe, transport, server, reducer, adapters, native UI and tests). Use Cargo build and test; keep a single executable with subcommands until a split is justified. Build native release binaries for Windows and Linux/WSL2. No Node.js, npm, JavaScript, React, Electron, Docker, database or additional services are required. Commit the resolved Cargo.lock before release; review dependencies and their licenses. Use English for documentation, issues, PRs, commits and UI text.

## 9. Release acceptance criteria

- [ ] Feasibility report records official references, real sanitized fixtures, versioned support matrix, transport results and non-interference evidence; unresolved core-state or transport gaps have an explicit maintainer decision.
- [ ] At least four real concurrent sessions across both supported providers and Windows/WSL2 are visible in one collector without state contamination.
- [ ] Supported state transitions match real agent behavior. Stop continuation, automatic permission resolution and rejected prompts cannot generate false confirmed completion, attention or running states; unsupported states are clearly marked.
- [ ] Duplicate, delayed, reordered and missing evidence, interruption, session end, SSE reconnect and collector restart are tested. Stale records are bounded and expiry does not imply termination.
- [ ] Hook installation/removal preserves existing behavior and settings, including subsequent user edits and repeated operations. Collector-down, hang, timeout and malformed-event cases pass the non-interference deadline and behavior tests.
- [ ] No manual refresh is needed for live updates; target under one second from collector receipt to UI update in normal local operation.
- [ ] Unauthenticated ingest, invalid/oversized input, disallowed Host and foreign Origin are rejected; cross-origin reads, token rotation, bounded storage and slow SSE clients are covered.
- [ ] Raw payloads, prompts, responses, commands, tool arguments and credentials are absent from persisted state, event forwarding and logs. Token file protections are verified.
- [ ] Separate automated, simulated and real CLI/OS results. Document unsupported versions/modes, notification limitations, installation and removal instructions in English before release.

Phase 1 is a small feasibility spike, not the complete application. Rust replaces the original Node.js experiment; repeat the feasibility measurements on the Rust binary rather than treating the old results as evidence for it. Do not modify unrelated repositories, change agent approval policies, or claim unverified platform support.
