# Phase 1 — Rust observation feasibility spike

**Gate: NO-GO for Phase 2.** Rust replaces the Node.js experiment in PR #2. This is disposable experimental code, not a production monitor. Native binaries remove the Node runtime requirement and are expected to reduce startup overhead; that expectation is not a measurement.

## Implementation

One Rust binary (Cargo package agent-monitor-spike) supports init, serve, hook and relay. The Windows and Linux builds use the same source. Dependencies: serde, serde_json and rand. No Node.js, npm, embedded web runtime or separate relay server. The native GUI is deferred to Phase 3.

The hook reads bounded stdin, extracts only allowlisted identifiers and event metadata, and sends a provisional observation to 127.0.0.1:8787. It never declares confirmed state. The receiver accepts a deliberately narrow one-request HTTP/1.1 subset, exact loopback Host, bearer token, JSON content type, bounded headers/body, no Origin or transfer encoding; it is an experimental receiver, not a production HTTP implementation. Well-framed rejected requests have their bounded body consumed before the HTTP rejection is sent; their bodies are not deserialized or emitted. Malformed, oversized or ambiguous framing is not drained. The client half-closes its write side after sending the body. Up to eight connections are processed independently with an 800 ms per-connection deadline; excess connections are closed without queuing or a guaranteed HTTP response. A slow connection consumes only its own slot, not the accept loop. Receipt order is not provider event order. It prints only validated observations; no raw payload or session history is stored. A receipt means HTTP 204 was received, not that the event establishes any agent state. ObservedAtMs is local capture time, not provider ordering. The relay sends sanitized JSON through stdin to a Windows executable; no shell interpolation and no additional listener.

The 350 ms watchdog starts after binary startup; the relay has a 300 ms budget measured from hook entry. These are not end-to-end guarantees: process startup, OS scheduling, blocked stdout and Windows interop child cleanup require actual measurement. The watchdog exits even if stdin is incomplete. For a hook invocation the candidate neutral response remains {} followed by newline for Codex Stop, empty stdout for other events, exit zero in both cases. The internal relay may return a fixed diagnostic exit code; the provider-facing hook never propagates it. The neutral behavior for each provider/version/event is still unverified. A missing executable or provider shell failure is outside the process's control.

### Opt-in diagnostics

Pre-create a private, empty file outside the repository and set AM_SPIKE_DIAG_FILE to its path in the hook environment. The probe does not create the file. Records contain only timestamp, elapsed milliseconds and fixed stage/outcome names: hook_started, relay_started, stdin_read, event_validated, token_loaded, relay_spawned, ack, input_error, invalid_event, token_error, connect_error, io_error, http_rejected, relay_start_error, relay_failed, timeout. No raw input, path, session ID, token, stderr, or response body is logged. The watchdog can exit before the final record; an incomplete trace is not proof of a specific cause. Use one isolated trial at a time to correlate records by time; remove the variable and delete the private file after testing. On WSL the parent can observe the helper's fixed exit code without sharing the diagnostic file; separate helper diagnostics require explicit private environment configuration. Do not enable diagnostics permanently.

## Historical evidence — Node.js HEAD 2216ab7f only

The [target-PC validation comment](https://github.com/changukshin/agent-monitor/pull/2#issuecomment-7184514542) records the following on November 26, 2026:

| Scenario | Historical result (not Rust evidence) |
| --- | --- |
| Windows build 26300, Node 22.21.0 | 4/4 tests passed; synthetic Stop 6/6; first 567 ms, warm 118–144 ms |
| WSL 2.7.12.0, Ubuntu 24.04.4, NAT | First relay missed at 954 ms; repeat 5/5 at 288–363 ms; timing included wsl.exe startup |
| Synthetic hang/offline | 475/145 ms, exit zero and {} |
| Codex CLI 0.167.0, Windows | Normal response, but project Stop receipt absent; discovery/trust versus delivery unresolved |
| Claude Code 2.1.287, Windows | SessionStart, UserPromptSubmit, StopFailure received; successful turn blocked by account credit |

All observations were provisional. Neither the 500 ms target nor real core-state confirmation was established. Rust must be retested independently.

## Rust review follow-up

At Rust HEAD 3936337a, [Ubuntu CI](https://github.com/changukshin/agent-monitor/actions/runs/45772253636/job/139747421973) passed; [Windows CI](https://github.com/changukshin/agent-monitor/actions/runs/45772253636/job/139747422289) stopped after 5 unit tests passed and the wrong-token case returned Io instead of Http. Closing a connection before consuming a bounded request body is a plausible cause, not a confirmed socket-level diagnosis. The [review](https://github.com/changukshin/agent-monitor/pull/2#issuecomment-7185178644) also identified head-of-line blocking in the sequential accept loop.

The follow-up separates bounded request framing from header policy, drains the declared bounded body before sending a policy rejection, and dispatches connections with eight RAII-managed slots. Regression tests preserve the wrong-token Http assertion, verify exact rejection codes without emitting observations, hold an incomplete connection open while a second request receives an acknowledgement within the hook budget, and exercise slot exhaustion/release. These are synthetic tests, not live-provider or WSL2 evidence. CI must verify the new head; the original socket-level cause remains unconfirmed.

## Build and synthetic smoke tests

Rust stable (minimum declared Rust 1.85) and Cargo are required to build, not to run the resulting executable. Alternatively, download the Windows or Ubuntu release-binary artifact from the successful Rust spike CI run for the exact PR head being tested (GitHub Actions > run > Artifacts). Extract it in a disposable workspace; on Linux restore executable permission if needed (chmod +x agent-monitor-spike). Verify the run's commit SHA and platform before executing. CI artifacts are experimental, not a release; Linux runner compatibility with the target WSL distribution must still be checked. The commands below assume locally built binaries; substitute the extracted artifact path when testing prebuilt binaries.

    cargo test --all-targets
    cargo build --release

The first Cargo run resolves dependencies and generates Cargo.lock. Review and commit the lockfile before release. CI compiles and tests Linux and Windows; a CI runner is not WSL2 or a real provider test. Authoring environment lacks Rust and network access, so no local compile/test or benchmark result is claimed.

Use a disposable checkout. On Windows, run the Windows release binary in PowerShell:

    .\target\release\agent-monitor-spike.exe init
    .\target\release\agent-monitor-spike.exe serve

On Linux, use target/release/agent-monitor-spike with the same subcommands. Init creates a random token exclusively in the user's home directory (.agent-monitor-spike/token). Unix creation modes are 0700 and 0600; verify existing directory and file permissions. On Windows verify inherited ACLs with icacls before serve; the spike does not modify ACLs. An existing private token file may be selected with AM_SPIKE_TOKEN_FILE for serve/hook/relay, but init uses only the default. Never put the token in arguments, URLs, logs or repositories. Rotate by stopping the collector, deleting only the owned token, generating a new one and restarting.

Native Windows, separate PowerShell terminal:

    '{"hook_event_name":"Stop","session_id":"synthetic-windows"}' | .\target\release\agent-monitor-spike.exe hook codex Stop native-windows

WSL2 NAT: build the Linux binary inside WSL, build the Windows binary on Windows, start the Windows receiver. In an already running WSL shell:

    printf '%s' '{"hook_event_name":"Stop","session_id":"synthetic-wsl"}' | target/release/agent-monitor-spike hook codex Stop wsl-ubuntu '/mnt/c/dev/agent-monitor/target/release/agent-monitor-spike.exe'

Adjust the absolute Windows binary path as needed. The Windows relay reads the Windows home token. For mirrored mode, test direct loopback with AM_SPIKE_TOKEN_FILE pointing to a verified private mounted token, or use the same interop path. No network, firewall or interop changes. Interop unavailable means report the limitation. Expect {} and a matching provisional receipt; a zero hook exit alone is deliberately not proof of delivery. Repeat with collector stopped, serve --hang, invalid input, missing token and unavailable relay. Measure first and repeated invocations externally, both in an already running WSL shell and including wsl.exe startup as separate metrics.

## Real CLI trials (isolated only)

Official references: [Codex hooks](https://developers.openai.com/codex/hooks), [Claude Code hooks](https://code.claude.com/docs/en/hooks), [WSL networking](https://learn.microsoft.com/en-us/windows/wsl/networking), [WSL interoperability](https://learn.microsoft.com/en-us/windows/wsl/interop). Verify installed versions and the actual hook input/output contract before registering candidates.

Use a disposable project outside working repositories. Record exact CLI/OS/WSL versions, network mode and baseline behavior. Inspect inherited hooks and trust settings; do not replace existing files or broaden approvals. Use a single documented project hook and an absolute path to the native binary; examples for a Stop hook are illustrative, not automatically installed. Codex candidate .codex/hooks.json:

    {"hooks":{"Stop":[{"hooks":[{"type":"command","command":"C:/dev/agent-monitor/target/release/agent-monitor-spike.exe hook codex Stop native-windows","timeout":1}]}]}}

Claude Code candidate .claude/settings.local.json (verify event-specific output and onFailure support):

    {"hooks":{"Stop":[{"hooks":[{"type":"command","command":"C:/dev/agent-monitor/target/release/agent-monitor-spike.exe hook claude Stop native-windows","timeout":1,"onFailure":"continue"}]}]}}

Quote paths according to the actual provider shell. In WSL use Linux executable and optional Windows relay path as above. Use the exact configured event name in both the hook key and command. Test one event at a time; enable diagnostic mode only for isolated trials. Do not modify working project hooks or trust policies. Remove only disposable test configuration and token when finished.

### Provider matrix — Rust evidence pending

| State | Codex candidate | Claude Code candidate | Required distinction |
| --- | --- | --- | --- |
| Registration | SessionStart | SessionStart | Not proof of idle or running |
| Running | UserPromptSubmit, PreToolUse, PreModelRequest | UserPromptSubmit, PreToolUse | Prompt/tool may be blocked |
| Human approval | PermissionRequest | PermissionRequest, Notification | May be resolved without a human |
| Human input | No verified mapping | Elicitation, Notification | May be automatically resolved |
| Resolution | Subsequent evidence | ElicitationResult, PermissionDenied, subsequent evidence | No universal correlation |
| Completion | Stop | Stop, Notification(idle_prompt) | Stop can be vetoed; idle is not success |
| Failure | No verified turn mapping | StopFailure | Individual tool errors can recover |
| Interruption | Interrupt | No verified mapping | Test cancellation vs exit |
| Session end | SessionEnd | SessionEnd | Crash/disconnect may not emit event |
| Idle | No verified mapping | Notification(idle_prompt) | Silence proves nothing |

Every row remains provisional-only or untested for this binary. For each provider/environment, compare actual behavior and evidence for normal and subsequent turns, Stop continued by a separate one-time hook, manual approval/denial, automatically resolved approval under an isolated existing test policy, blocked prompt, tool failure followed by recovery, human input, interruption, exit and collector failure/hang. Verify no changed decisions or injected context; record provider-added latency versus baseline, including cold/warm trials. Repeat native Windows and WSL2 NAT/mirrored where available, using the same receiver. Verify Windows child cleanup and token ACLs.

Record exact versions, scenario, baseline versus instrumented outcome, sanitized observations, trials/max added latency, and verdict. Replace identifiers before publishing fixtures. Never publish transcripts, raw input, credentials or private paths. If diagnostics do not show hook_started, investigate discovery/trust/launch before transport; if event_validated is missing investigate input/schema; if ack is missing examine fixed outcome and receiver receipt. Do not treat missing traces as conclusive.

## Gate

Rust source, tests and CI are supplied; compilation and target-machine results must be verified before merge readiness. No new live provider or WSL measurements are claimed. Core-state confirmation, neutral hook semantics, startup/relay reliability, diagnostics and 500 ms provider-added latency remain open. Stop before production implementation if the evidence is insufficient. Only a maintainer can revise the scope.
