# Phase 1 — Rust observation feasibility spike

**Gate: NO-GO for Phase 2.** Rust replaces the Node.js experiment in PR #2. This is disposable experimental code, not a production monitor. Native binaries remove the Node runtime requirement and are expected to reduce startup overhead; that expectation is not a measurement.

## Implementation

One Rust binary (Cargo package agent-monitor-spike) supports init, serve, hook and relay. The Windows and Linux builds use the same source. Dependencies: serde, serde_json and rand. No Node.js, npm, embedded web runtime or separate relay server. The native GUI is deferred to Phase 3.

The hook reads bounded stdin, extracts only allowlisted identifiers and event metadata, and sends a provisional observation to 127.0.0.1:8787. It never declares confirmed state. The receiver accepts a deliberately narrow one-request HTTP/1.1 subset, exact loopback Host, bearer token, JSON content type, bounded headers/body, no Origin or transfer encoding; it is an experimental receiver, not a production HTTP implementation. Well-framed rejected requests have their bounded body consumed before the HTTP rejection is sent; their bodies are not deserialized or emitted. Malformed, oversized or ambiguous framing is not drained. The client half-closes its write side after sending the body. Up to eight connections are processed independently with an 800 ms per-connection deadline; excess connections are closed without queuing or a guaranteed HTTP response. A slow connection consumes only its own slot, not the accept loop. Receipt order is not provider event order. It prints only validated observations; no raw payload or session history is stored. A receipt means HTTP 204 was received, not that the event establishes any agent state. ObservedAtMs is local capture time, not provider ordering. The relay sends sanitized JSON through stdin to a Windows executable; no shell interpolation and no additional listener.

The 350 ms watchdog starts after binary startup; the relay has a 300 ms budget measured from hook entry. These are not end-to-end guarantees: process startup, OS scheduling, blocked stdout and Windows interop child cleanup require actual measurement. The watchdog exits even if stdin is incomplete. For a hook invocation the candidate neutral response remains {} followed by newline for Codex Stop, empty stdout for other events, exit zero in both cases. The internal relay may return a fixed diagnostic exit code; the provider-facing hook never propagates it. Observed provider outcomes and exceptions are recorded in the validation below. A missing executable or provider shell failure is outside the process's control.

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

The first Cargo run resolves dependencies and generates Cargo.lock. Review and commit the lockfile before release. CI compiles and tests Linux and Windows; a CI runner is not WSL2 or a real provider test. The original authoring environment lacked a Rust toolchain. The target-PC report below uses release artifacts from the verified CI run.

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

## Target-PC validation of Rust commit 40432c3f, 2026-11-26

**Decision: NO-GO.** The native transport works on this PC. The current hook evidence cannot reliably identify actual human attention or final turn completion across both providers. The 500 ms provider-facing target was exceeded, including with a healthy receiver.

### Environment and provenance

Windows build 26300, native Codex CLI 0.167.0 and Claude Code 2.1.287. WSL 2.7.12.0, Ubuntu 24.04.4 LTS, networking mode NAT. All live provider trials below ran on Windows. The exact-head [CI run](https://github.com/changukshin/agent-monitor/actions/runs/45773655048) passed 9 unit and 3 CLI tests per OS and produced both release artifacts. The [transport retest](https://github.com/changukshin/agent-monitor/pull/2#issuecomment-7185401564) received 5/5 synthetic Windows events (31–35 ms) and 5/5 WSL-to-Windows events (202–222 ms including wsl.exe). Live WSL providers and mirrored networking remain untested.

### Why the original Codex project hook was silent

1. In the original disposable clone, the installed CLI's app-server `config/read` with the exact cwd and `includeLayers` reported a disabled project layer and explicitly identified missing folder trust. In four separate configs, `hooks/list` reported an enabled, trusted **user** handler for SessionStart, UserPromptSubmit, PreToolUse or Stop, respectively, and no project handler for that event. The old CLI “Stop Completed” message had no project attribution.
2. With the maintainer's authorization, the interactive folder-trust prompt was accepted for the disposable root. All eight reviewed project commands then appeared as **project / untrusted**, alongside the existing user hooks. The normal Hooks review screen approved those eight commands after checking their absolute executable path and arguments. The inventory then reported **project / trusted**. The interactive default daemon launch had first failed with Windows access denied; the CLI's suggested `--no-daemon` fallback reached its ordinary trust UI.
3. After approval, each of the four events was configured individually in a fresh `codex exec --ephemeral` run using the read-only sandbox. All four invocations exited zero; the collector received each requested event. In the PreToolUse trial the final text did not match the requested exact marker, so only event delivery is credited. A separate instrumented app-server session distinguished project and user completions by source. For the project handlers, SessionStart, UserPromptSubmit and Stop took 458, 428 and 493 ms respectively, while their probe traces reached `ack`. The absolute Windows executable path with forward slashes worked through the provider's command launcher. The disposable token and diagnostic file variables reached the launched probe. No shell substitution, trust override or product code change was required.
4. The failure in the reproduced original configuration occurred **before command launch**, at project discovery. Folder trust alone would still leave the individual commands pending review. An empty trace in a different project cannot diagnose either condition by itself.

### State support on this PC

The matrix classifies what a separate monitor can establish from **hook receipts**. CONFIRMED applies only to the narrow event or transition explicitly corroborated in a trial. PROVISIONAL_ONLY means a relevant receipt exists but cannot settle the claimed state. UNSUPPORTED means the tested observation path supplies no distinguishing evidence for that state. UNTESTED records missing ground truth. Every transmitted Rust record still carries `confidence: provisional`.

| State or scenario | Codex | Claude Code | Ground truth and limits |
| --- | --- | --- | --- |
| Session registration | CONFIRMED | CONFIRMED | SessionStart reached the receiver on a real launch. This establishes a registered session only. |
| Prompt submitted / running | PROVISIONAL_ONLY | PROVISIONAL_ONLY | Both emitted UserPromptSubmit. Claude then failed before a successful model turn. Submission cannot establish ongoing work. |
| Normal turn completion | PROVISIONAL_ONLY | UNTESTED | Codex produced a normal final answer and Stop. The continuation trial below produced the same initial Stop before more work. Claude credit blocked successful completion. |
| Second turn in one session | PROVISIONAL_ONLY | UNTESTED | Codex accepted another prompt in the same TUI. Two successive turns on one app-server thread also completed. These receipts identify boundaries, not a continuous running state. |
| Actual human approval and resolution | UNTESTED | UNTESTED | Effective Codex approval policy was `never`. A request for permission to write one disposable file ended without a native approval request or pending flag. No write was requested of the shell. The policy was preserved. |
| Permission request resolved automatically | UNTESTED | UNTESTED | No actual permission request followed by an automatic decision was observed. A policy preventing a request is a different outcome. |
| Human input pending and resolved | UNSUPPORTED | UNTESTED | In Codex Plan mode a real native form waited for the maintainer. The maintainer selected BETA; the owning server cleared the wait flag and the turn completed. The probe emitted ordinary tool boundaries without the wait kind or its resolution. An earlier TUI prompt requesting a question remained working and was interrupted; it supplied no input-wait evidence. |
| Stop followed by continuation | PROVISIONAL_ONLY | UNTESTED | The separately approved one-time Stop fixture requested continuation once. On retry the one-shot fixture was consumed, the final answer was CONTINUED, and the collector observed Stop with false and true stopHookActive values. The first attempt exceeded the harness's 120 s limit; no success is credited to it. |
| Tool failure followed by recovery | PROVISIONAL_ONLY | UNTESTED | The TUI attempted two shell commands. Both failed to start; the agent recovered sufficiently to report both failures. In a separate real exec session, CLI JSON reported failed command execution (exit -1), then a completed read-only MCP tool call and a completed turn. The collector received PreToolUse twice and Stop. Neither PreToolUse distinguished failure from recovery. |
| Turn failure | UNTESTED | PROVISIONAL_ONLY | Claude print mode exited 1 for insufficient account credit and emitted StopFailure. A general mapping to final failure was not established. |
| User interruption | CONFIRMED | UNTESTED | Escape during an active interactive Codex turn produced the visible interruption message and an Interrupt receipt. This finding covers that explicit cancellation path. |
| Normal session termination | PROVISIONAL_ONLY | UNTESTED | Codex emitted SessionEnd on orderly shutdown. The receipt precedes termination; crash and disconnect coverage is unknown. |
| Idle | UNSUPPORTED | UNTESTED | The Codex receiver had no independent idle transition. Silence was not used as evidence. |

### Sanitized real fixtures

These are projections of received JSON records. Identifiers, wall times and capture IDs are omitted. Each line retains observed fields only; omitted fields are not claims about provider input.

```jsonl
{"source":"codex","event":"SessionStart","confidence":"provisional"}
{"source":"codex","event":"UserPromptSubmit","confidence":"provisional"}
{"source":"codex","event":"PreToolUse","confidence":"provisional"}
{"source":"codex","event":"Stop","stopHookActive":false,"confidence":"provisional"}
{"source":"codex","event":"Stop","stopHookActive":true,"confidence":"provisional"}
{"source":"codex","event":"Interrupt","confidence":"provisional"}
{"source":"codex","event":"SessionEnd","confidence":"provisional"}
{"source":"claude","event":"SessionStart","confidence":"provisional"}
{"source":"claude","event":"UserPromptSubmit","confidence":"provisional"}
{"source":"claude","event":"StopFailure","confidence":"provisional"}
```

### Read-only observation interfaces

On the owning Codex server, the real form produced `thread/status/changed → active [waitingOnUserInput]`, followed by the actual input request. After the human selection the sequence was `active [] → idle` and `turn/completed`. This **confirms that single wait and resolution on the owning server**. Its installed schema also defines `waitingOnApproval`; that flag was not validated live. The first input harness stopped without validating resolution; the repeat verified the answer and the cleared flag.

The isolated folder and reviewed commands were trusted through the provider UI. The test hook configuration and private credential were removed after the trials.

A separate server used the supported cwd-filtered `thread/list` and metadata-only `thread/read` against the disposable TUI session. Both reported `notLoaded` from that server. The local managed daemon's read-only version check reported connection refused. No agent relaunch requirement, daemon installation, private transcript reader or wrapper is proposed. Passive access to an ordinarily launched live session remains unproven. Claude print-mode hook lifecycle events were useful inside the test client; the installed CLI also advertises an independent read-only listing for interactive and background sessions via `claude agents --json`. The cwd-filtered listing returned zero rows while the disposable interactive launch was at its folder-trust screen. The maintainer chose to preserve Claude trust settings, and the launch was exited there. This does not test an already-trusted active session.


### Non-interference and provider-facing latency

A native probe process deadline excludes provider dispatch, shell startup and cleanup. We therefore measured project handler duration on a **real turn** and recorded whole-turn timing separately. Small sample sizes, model scheduling and pre-existing user hooks limit comparisons. “First / repeat” below means the first invocation in a fresh session and the next in the same session for Codex. A machine reboot or cold executable cache was not part of the trial.

| Codex 0.167.0, Windows, read-only | Project Stop duration, first / repeat (ms) | Whole turn, first / repeat (ms) | Actual result |
| --- | --- | --- | --- |
| Baseline, project hooks absent | — | 10,917 / 3,012 | Both turns completed |
| Receiver available | 522 / 527 | 14,020 / 3,386 | Both handlers and turns completed |
| Receiver offline | 964 / 1,002 | 12,321 / 3,193 | First handler completed; second handler **failed** under the configured one-second hook timeout. Both turns completed. |
| Receiver unresponsive | 929 / 857 | 10,648 / 2,791 | Both handlers and turns completed |

The installed app-server reported project-specific handler duration in `hook/completed.run.durationMs`. The controlled comparison used gpt-6.2-sol, one Stop probe, identical short prompts and existing user hooks. Whole-turn deltas include inference and are not isolated hook cost. The default-model baseline was stopped after several minutes without a completed turn. In a separate default-model healthy-receiver run, project Stop took 505 / 517 ms. The timings do not establish a general upper bound. Some interactive sessions also reported inherited MCP startup warnings.

| Claude Code 2.1.287, Windows print mode | Whole process, initial / repeat (ms) | SessionStart span (ms) | UserPromptSubmit span (ms) |
| --- | --- | --- | --- |
| Baseline, no project hooks | 3,093 / 2,983 | — | — |
| Receiver available | 3,566 / 3,557 | 387 / 385 | 247 / 187 |
| Receiver offline | 5,648 / 5,079 | 538 / 543 | 470 / 466 |
| Receiver unresponsive | 5,193 / 5,042 | 542 / 552 | 522 / 479 |

Claude ran two successive fresh, nonpersistent processes per condition using explicit isolated settings and no tools. Spans are wall time between matching hook-start and hook-response records in the supported streaming output, so buffering may affect them. StopFailure was received in healthy-receiver trials; its duration was not available as a matched stream span. Every condition ended with the same insufficient-credit error and exit 1. Same-session warm successful execution could not be measured. These tests support behavioral equivalence only for this failure path.

### Readiness recommendation

**NO-GO for Phase 2.** The discovery issue is explained and successful target-PC delivery is reproducible. Codex Stop can precede further work. The owning-server input flag resolved correctly but cross-process access to equivalent live Codex state is unproven. Ordinary approval waits and their resolution remain unvalidated under the existing policy. Successful Claude turns require account credit. Provider-facing handler latency exceeded 500 ms, and one offline hook failed at its provider timeout even though the turn completed. Retain the 350 ms watchdog and the 500 ms target. No Rust change is justified by the pre-launch trust failure. Revisit readiness with live approval evidence, independently readable attention/completion state and repeatable latency within target, or a separate maintainer decision revising scope.

## Phase 1B — provider-native observation

**Recommendation: NO-GO. Maintainer decision pending (#3).** An external process discovered ordinary Windows Claude sessions. Actual human attention and final turn outcome remain unconfirmed for both providers through the tested passive paths.

### Repeat the experiment

Build with `cargo build --release --bin native-observe` or use the release artifact for the host OS. Run once per sample:

    native-observe claude <absolute-claude-executable> <absolute-session-cwd>
    native-observe codex <absolute-native-codex-executable> <absolute-session-cwd>

Supply the provider's cwd spelling. Keep returned UUIDs local. Each invocation emits one scoped JSON snapshot or a static error and nonzero exit. Repeat to reconnect. Only one sample need be outstanding. The client caps input at 1 MiB and applies a five-second budget; process creation and cleanup can add wall time. Codex returns its first page (up to 32 threads) with partial=true when another page exists. An omitted thread cannot establish exit.

Claude uses `agents --json --all --cwd` and checks cwd again before emitting allowlisted metadata. Codex launches only the existing control socket's stdio proxy, initializes it and requests a scoped metadata-only thread list. It does not request turns/items, start threads, answer requests or start a daemon.

### Target-PC trials, November 26, 2026

Windows build 26300, Claude Code 2.1.287, Codex CLI 0.167.0, WSL 2.7.12.0, Ubuntu 24.04.4 LTS, NAT. Two ordinary interactive Claude launches used separate already-trusted directories. A third disposable session used Claude's standard background option. The initial listing was empty. Two successive independent Rust samples across the two directories returned three distinct stable UUIDs. Agent processes continued between observer invocations. Trust and approval settings were preserved.

| Scenario | External Claude observation | External Codex observation |
| --- | --- | --- |
| Discovery / correlation | CONFIRMED for two interactive sessions and one background session on Windows. | UNTESTED on a reachable owner. The default daemon version check reported connection refused; the native proxy sample returned unavailable. |
| Active work / second turn | CONFIRMED for the busy flag. While one interactive session retried a request, the other stayed idle. A second prompt made the first busy again. | UNTESTED on a shared owner. |
| Human input / approval pending and resolved | UNTESTED. Background state blocked did not identify the reason or a request. No approval was answered. | UNSUPPORTED on the tested ordinary path. Phase 1A's input wait and resolution were visible only on the owning test server; approval remained untested. |
| Successful completion / continued Stop | UNSUPPORTED by the tested listing fields. After a visible API connection failure the interactive session returned to idle. | UNSUPPORTED on the tested path. Phase 1A observed Stop before continuation. |
| Failure / recovery | PROVISIONAL_ONLY. The ordinary interactive client exhausted retries with connection refused. Isolated print mode separately reported insufficient credit. Neither identifies a monitor defect. Successful tool recovery was untested in these Claude trials. | UNTESTED externally. The Phase 1A owning-client recovery is not external evidence. |
| Interruption / exit | PROVISIONAL_ONLY. Escape during the second turn was followed by idle. Stopping the owned background trial yielded state stopped. Neither sample carried a terminal turn status. | UNTESTED externally. Phase 1A's Interrupt hook covered explicit cancellation only. |
| Reconnect / loss | CONFIRMED for resampling the same three identities. Unavailable or missing observations remain unknown. | An independent server previously reported notLoaded. Neither it nor a failed proxy connection establishes ownership. |

Sanitized projections use stable aliases in place of UUIDs. The blocked value came from the raw provider listing; the first Rust artifact mapped it to unknown, and the final allowlist preserves it.

    A interactive idle
    B interactive idle
    A interactive busy     B interactive idle
    C background  busy     state=working
    A interactive idle     UI reported connection refused
    C background  idle     state=blocked
    C background  unknown  state=stopped

Windows release-binary invocations for the two cwds took 2,250 and 2,211 ms. This external sampling cost constrains refresh frequency. No synchronous hook is added by either snapshot adapter. WSL interop successfully invoked the Windows observer against Windows Claude. The Linux artifact rejected an absent provider and timed out against a stalled disposable listing process in 5,647 ms including wsl.exe startup. WSL exposed a Windows npm shim for Codex which failed under Linux. No live native WSL provider pair was demonstrated. Phase 1A's NAT receipt remains transport-only evidence.

### Async discovery and non-interference

Claude 2.1.287 executed a Rust UserPromptSubmit command hook in synchronous and asynchronous modes using invocation-local settings. Baseline / sync / async print processes took 5,221 / 5,111 / 4,494 ms, one sample per condition. Each exited with the same external credit error. Both traces reached event validation and the intentionally missing token. The probe produced no context or decision. These whole-process measurements do not isolate async dispatch latency or establish the 500 ms target. Phase 1A exceeded that target even with a healthy receiver.

Codex documents async PostToolUse command handlers and synchronous SessionEnd. The installed hook schema exposed an async field. No newly trusted async command was installed or executed on Codex. Async delivery remains a supplemental discovery candidate. References: [Codex app server](https://developers.openai.com/codex/app-server), [Codex hooks](https://developers.openai.com/codex/hooks), [Claude CLI reference](https://code.claude.com/docs/en/cli-reference), [Claude hooks](https://code.claude.com/docs/en/hooks).

### Decision requested

Keep Phase 2 blocked. A narrower Windows Claude inventory could expose busy/idle while keeping human attention and final outcome unknown. Alternatively, investigate an explicitly opt-in Codex owning-server launch integration in a separately approved experiment. Neither scope has maintainer approval here. Ordinary passive Codex access, native WSL providers, request correlation, successful Claude outcomes and the 500 ms hook target remain open.

### Review follow-up: documented fields and observation coverage

The [Claude agent view reference](https://code.claude.com/docs/en/agent-view#read-session-state-from-a-script) documents optional session UUIDs, waiting reasons and the background done state. The adapter now preserves the five documented reasons only while status is waiting. Unknown reasons become unknown. A missing UUID leaves sessionId null; an eight-hex-digit background ID can correlate a background row within the same runtime. Rows without either identity are snapshot-only. A background done value describes provider session state; these trials did not correlate it with a particular successful turn. The new mappings were regression-tested with constructed fixtures. The earlier live trials did not exercise either a waiting reason or done.

The [upstream Codex discussion](https://github.com/openai/codex/issues/62230) distinguishes shared session discovery from live state in the owning App Server. A partial listing and notLoaded both limit what the passive monitor can assert.
