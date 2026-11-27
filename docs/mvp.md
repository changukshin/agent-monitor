# Agent Monitor — approved v0.1 MVP specification

Status: **GO for Phase 2 implementation under the reduced scope**, approved by the maintainer on 2026-11-27. This is not a release approval or a claim that the production collector or GUI exists. Earlier broad feasibility requirements are superseded for v0.1; historical evidence and limitations remain in the [Phase 1A/1B report](phase-1-feasibility.md) and [Phase 1C report](phase-1c-managed.md). See #3, #8 and PR #9.

## 1. Goal and approved support matrix

Build a useful native Windows dashboard that identifies active work, actual human input needs and terminal turn results where the evidence establishes them. Passive observation is the default and never changes ordinary launches; managed observation is an explicit opt-in using the official client. Rust-only, local-first, read-only, fail-open, minimal dependencies, English-first, MIT.

| Capability | Codex managed Windows 0.167.0 | Codex passive Windows 0.167.0 | Claude passive Windows 2.1.287 |
| --- | --- | --- | --- |
| Session identity | Confirmed within selected owner | Limited/provisional discovery | Confirmed when stable UUID present |
| Running/idle | Confirmed sampled active/idle | Unknown | Confirmed reported busy/idle |
| Human input wait/clear | Confirmed single-form scenario only | Unsupported on tested path | Untested |
| Approval wait/clear | Untested, excluded | Untested, excluded | Untested, excluded |
| Completed turn | Confirmed provider-reported terminal result | Stop is provisional only | Unknown; idle is not success |
| Interrupted turn | Confirmed | Explicit Interrupt hook was corroborated historically, not a shipping hook integration | Unknown |
| Failed turn / process exit | Untested / unsupported, excluded | Unknown | Unknown |

The release-critical path is Windows managed Codex; Windows passive Claude inventory/activity is a limited secondary adapter. Passive Codex may expose safe discovered metadata as provisional where available; an inaccessible owner or notLoaded is unknown, not idle. Do not require synchronous monitor hooks in the release: Phase 1A exceeded the 500 ms target. Do not claim the historical Interrupt observation as a production capability unless a safe integration is subsequently validated. Claude 2.1.332 observations are limited experimental evidence, not a substitution for the 2.1.287 support contract. Other versions, native WSL2 providers, Claude managed mode, real approval waits, terminal failures, complete transition delivery and process-exit detection are excluded from confirmed v0.1 capabilities. Display unknown/unsupported explicitly. Do not treat a background done as successful turn completion.

No dedicated hardware, remote service, database, durable history, agent orchestration, auto-launch, automatic approvals, command execution, account for the monitor or multi-host synchronization. No UI window activation, tray/always-on-top, GitHub/CI integrations, or monetization features in v0.1.

## 2. Feasibility decision and workflow

Phase 1A (#2) established Rust transport, not reliable core states or the hook latency target. Phase 1B (#7) established limited ordinary Windows Claude activity/identity and documented passive Codex access limits. Phase 1C (#9) established same-owner Windows Codex observation with two official TUIs, single actual input wait/clear, completed and interrupted turns, and a specific observer-crash/reconnect trial. Polling observations are not an atomic stream; neither trial count nor code tests establish all schedules. The maintainer accepted the exclusions above; the Phase 2 gate passes for this narrower implementation, with final release validation still mandatory.

Managed workflow, based on the tested commands (substitute real absolute paths):

1. User explicitly creates a new private socket directory with verified user-only ACLs and starts a dedicated owner using the official Codex CLI: `codex app-server --listen "unix://<absolute-socket-path>"`.
2. User starts official TUIs using `codex --remote "unix://<absolute-socket-path>"` in configured project directories.
3. User configures the monitor with the absolute native Codex binary and explicitly selected private endpoint/cwd(s). The observer attaches via the supported proxy and queries metadata only. Do not select the ordinary daemon as a fallback.
4. User owns TUI and server lifetime. Monitor startup, failure, removal or exit must not terminate or reconfigure either. No production launch wrapper or custom TUI is required.

The owner socket is control-capable; the Rust observer's RPC allowlist is not a security boundary for other clients. The private socket directory/ACL is mandatory. Do not expose a public TCP listener, alter ordinary daemon/trust/approval settings or broaden permissions. Verify endpoint identity, OS/runtime and ownership/lifetime semantics; fail safely if they cannot be established. Claude uses the documented version-specific metadata listing in selected cwd(s); do not require relaunch. Unknown/unavailable adapters must not block independent healthy adapters.

## 3. Observation and state contract

- Normalize provider, version, mode, runtime/host, selected owner identity/lifetime where applicable, session/thread ID, optional turn ID, evidence kind, confidence, source sample/sequence and observed time. Keep project labels user-supplied or approved metadata; never derive them from transcript content. A Windows observer invoked from WSL still observes Windows.
- Distinguish owner lifetimes and scoped sources. Do not correlate by cwd, list position, or display label alone. Claude background ID may correlate only if validated within its runtime; unidentified rows are snapshot-only and must not generate definitive transition alerts.
- Managed observations use metadata-only loaded thread listing, thread/read without turns, and latest-turn metadata without items. RPC allowlist must forbid mutation, resume, tool execution, approval responses and prompt submission. Cap results and time; preserve partial status when a turn query is unavailable. Omitted latestTurn means unavailable; null means an empty returned page.
- Status and latest-turn metadata are separate queries. A sampled pair is not atomic. Record sufficient sequencing/timing and provenance; if facts cannot safely be correlated, show UNKNOWN or separate last-known facts rather than combining them into a confirmed current state. An old completed turn plus a new active thread is not a completed current turn. Use a conservative recheck or invalidate contradictory pairs. Do not assume poll-to-poll order proves an unseen intermediate transition.
- A current, coherent active/inProgress observation with no known pending wait supports RUNNING; confirmed single input-wait flag associated with the relevant active turn supports WAITING_INPUT. A later coherent active/inProgress observation for the same turn with the flag cleared supports return to RUNNING; do not claim which request resolved if request correlation is unavailable. An absent flag, partial list, failure or stale sample alone never clears a wait. Multiple or ambiguous requests and unvalidated approval flags remain unknown or unsupported.
- A provider-reported terminal completed or interrupted status tied to the correct turn supports COMPLETED or INTERRUPTED respectively; never infer completion from Stop, idle, silence, tool success or a missing thread. IDLE describes verified provider idle with no active turn, not successful work. Completion means the turn ended normally, not that coding, CI or deployment succeeded. ERROR, WAITING_APPROVAL and ENDED are reserved/unconfirmed for v0.1 unless a new maintainer-approved support revision follows live evidence.
- Claude busy/idle are activity observations only. Never derive turn outcomes, human attention or turn identity from them. Hook receipts remain provisional observations; no production hook integration or definitive alerts based on them.
- Deduplicate repeated observations and notifications using verified identities and source ordering. Ignore older evidence that would overwrite a newer turn. Mark contradictions, missing fields, disconnected sources, stale samples and missing rows as uncertainty; a partial or empty page does not establish process exit. An explicit scope/coverage indicator must accompany incomplete lists. Do not assume more than the tested/configured coverage; the experiment used an eight-loaded-thread page.
- Retain last observed time and state-change time separately. Bound sessions, deduplication metadata, queues and stale retention. Eviction is a visibility/storage policy, not a semantic state transition. Restart clears in-memory state; fresh snapshots re-establish visibility without fabricating historical transitions. Do not notify on initial or reconnect snapshots as though they were new transitions.

Only allowlisted metadata may be stored or forwarded. Raw provider responses may be parsed transiently but never logged, persisted or forwarded. Do not collect prompts, responses, commands, tool arguments, environment variables, credentials or private transcripts.

## 4. Collector and transport

One Windows-local Rust collector with independently bounded polling for configured sources, no overlapping unbounded samples, bounded backoff, cancellation and cleanup. Keep source failures isolated. Choose sensible documented polling intervals based on measured costs (Phase 1C samples ranged from tens to hundreds of milliseconds; Phase 1B Claude listing took about 2.2 seconds). Measure refresh age and overhead; no guarantee of observing all brief waits or intervening turns. Do not equate observer sample duration with hook-added or causal agent latency.

Expose a small authenticated loopback ingest/read/update API or an equivalently safe Rust-native local interface. The GUI consumes snapshots and ordered updates from the collector, not provider RPCs. If HTTP/SSE is used, prefer a small audited implementation rather than promoting the experimental narrow HTTP parser. Use a current snapshot on subscribe/reconnect and ordered, bounded updates. No control API, public bind, wildcard CORS or network/firewall changes. WSL transport artifacts remain experimental, not v0.1 native agent support.

## 5. Non-interference and local security

Monitoring never changes authorization, prompts, stop decisions, output or continuation. No automatic hook installation in v0.1. Existing hooks and trust/approval settings remain intact. The 500 ms provider-facing hook target is retained for future hook integration; do not relax it or ship synchronous monitor hooks that fail it. A native observer must not own/kill the agent or server, respond to approval requests, or inject context. Validate disconnect, crash, timeout, offline owner and child cleanup against real supported sessions, not just simulated fixtures.

Bind only to the verified local interface. For HTTP restrict Host, reject foreign/browser origins, no cross-origin reads; authenticate sensitive read/update and ingest routes with a cryptographically random per-installation token or equivalent OS-private IPC access control. Use user-private ACLs, secure token creation, rotation/revocation, no tokens in URLs, logs, arguments, UI or event bodies. Bound headers, messages, records, client connections and pending updates; reject malformed/oversized input; protect against slow consumers. Health responses must not disclose sensitive details. Document single-trusted-local-user threat model. Verify owner endpoint ACL separately from collector authentication.

## 6. Dashboard

Native Rust GUI on Windows; collector remains independent. Show confirmed Running, Input needed, Completed and Interrupted where supported, plus idle, unknown/stale/provisional and coverage indicators. No confirmed approval/failure/exit count or notification for unvalidated states. Display mode, provider/version, project label, last observed time, confidence, and relevant elapsed time. Use text and icons as well as color; prioritize verified attention. Distinguish Codex turn results and Claude session activity. Opt-in desktop alerts only for newly observed confirmed managed Codex input waits and completed turns; suppress stale, duplicate, initial, reconnect and unsupported observations. Show that sampling can miss transitions. Document OS notification limitations.

## 7. Implementation sequence

- Phase 2 (#4): production Rust collector/reducer and safe, version-aware read-only adapters for the approved scope. Commit Cargo.lock and use locked Windows/Linux CI. Keep spike experiments separate; no new investigation phase required unless a reproducible blocker invalidates the approved core path.
- Phase 3 (#5): native GUI and notification UX.
- Phase 4 (#6): Windows release binaries, safe installation/removal, documented manual managed workflow and end-to-end validation. Linux CI may continue but does not advertise Linux/WSL provider support.
- Deferred capabilities require separate live evidence and maintainer scope approval before promotion.

## 8. Release acceptance criteria

- [ ] Approved scope and exclusions remain explicit in docs/UI. Supported versions and runtime evidence are recorded; no inferred support from documentation alone.
- [ ] At least four real concurrent sessions across the approved Windows observation paths are distinguishable without contamination; include multiple managed Codex TUIs and passive Claude activity if advertised.
- [ ] Real managed Codex active/idle, input wait/clear, completed and interrupted turns match official TUI behavior. Claude inventory/busy/idle matches supported-version behavior; no invented turn outcomes. Unsupported states remain visibly unknown. Test synthetic and actual observed cases separately.
- [ ] Tests cover contradictory/non-atomic samples, a new turn between RPCs, previous completion while active, missing/latestTurn null, partial/empty listings, duplicate/out-of-order evidence, source failure, stale eviction, restart, reconnect and multi-session/owner isolation. Continued Stop and failed tools cannot produce false completion or attention.
- [ ] Measure sampling cost, refresh age and end-to-end non-interference. Exercise observer-down, unresponsive owner, crash/reconnect and cleanup; preserve existing agent configuration and user-controlled approvals. No synchronous monitor hooks ship.
- [ ] Live updates require no manual refresh; target under one second from collector receipt to UI update (separate from provider sampling interval). Notifications are opt-in, deduplicated and limited to verified transitions.
- [ ] Local authentication/ACLs, input limits, bounded resources, slow clients, token lifecycle and privacy tests pass. No raw payload or credential leakage.
- [ ] Locked Windows and Ubuntu CI tests/builds pass on the exact release commit. Verify install/remove/upgrade on supported Windows without Cargo, Node.js or browser runtime. Document the private owner setup and cleanup.
- [ ] Maintainer reviews release evidence and approves v0.1.0 before tagging/publishing. Phase 2 GO is not release GO.
