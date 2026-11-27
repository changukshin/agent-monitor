# Phase 1C: optional managed observation

**Recommendation for maintainer review: keep Phase 2 NO-GO.** A Windows user can opt into the official Codex TUI connected to a dedicated local App Server. A separate Rust client can then distinguish an active turn, a genuine input wait and a provider-reported terminal turn outcome. The result applies only to the tested owner, client and runtime. Passive launch remains the default. This report proposes a narrower support contract; it does not approve it.

## Setup and ownership

November 27, 2026. Windows 10.0.26300.0, Codex CLI 0.167.0, Claude Code 2.1.332; Ubuntu 24.04 on WSL 2.7.12.0, kernel 6.18.33.2, NAT. Baseline repository commit e41ef05. The Windows release artifact at eb724b4 supplied the timed samples below. The final code commit 72a7130 adds the initialized notification after the handshake. See [artifact CI](https://github.com/changukshin/agent-monitor/actions/runs/45955976495) and [final code CI](https://github.com/changukshin/agent-monitor/actions/runs/45956545259).

1. Use an already trusted disposable working directory and a **new, private** directory for a temporary socket. The Windows trial removed ACL inheritance from the socket directory and granted access to the testing user's SID. Keep the socket outside shared folders. The owner endpoint is control-capable; the ACL is the access boundary. The observer's allowlist constrains this executable, not other clients with access.
2. Start a dedicated owner explicitly: `codex app-server --listen "unix://<absolute-socket-path>"`. Start two official clients in the trial cwd: `codex --remote "unix://<absolute-socket-path>"`. No daemon bootstrap, public TCP listener, trust edit, approval override or agent wrapper is required. The ordinary launch is unaffected.
3. Run `native-observe codex-managed <absolute-native-codex-executable> <absolute-trial-cwd> <absolute-socket-path>`. Resolve the native executable belonging to the selected CLI version. Managed mode requires an explicit absolute socket. The observer never selects the ordinary daemon as a fallback.
4. The official proxy carries WebSocket frames over the Unix socket, including on Windows. The first newline-JSON attempt stalled; an HTTP upgrade returned 101. The Rust client uses tungstenite over the proxy pipes, lists up to eight loaded threads, reads thread metadata without turns, and reads only the latest turn with itemsView=notLoaded. Ephemeral threads are excluded. An early build lost the whole snapshot on an unmaterialized/ephemeral history error. The corrected client preserves visible thread status when the turn RPC is unavailable. No transcript files are accessed. No request/approval is answered by this client.
5. Close the TUIs and stop only the explicitly launched owner. The observer does not own the server's lifetime. While the owner was still running, an immediate sample after both /exit commands still showed loaded idle threads. Presence and absence are unsuitable process-exit signals.

The installed CLI help, its generated TypeScript types and live protocol responses were used to check version-specific behavior. References: [Codex App Server](https://developers.openai.com/codex/app-server), [0.167.0 Unix socket transport](https://github.com/openai/codex/blob/rust-v0.167.0/codex-rs/app-server-transport/src/transport/unix_socket.rs), [Claude agent view](https://code.claude.com/docs/en/agent-view#read-session-state-from-a-script).

## What the two clients actually did

A and B below represent different thread UUIDs. T1, T2 and T3 represent different turn UUIDs; aliases are local to this table. The client correlates by thread and turn, not by message content, cwd alone or list position.

| Visible official client action | Independent observer evidence |
| --- | --- |
| A replied FIRST. B opened a native two-choice form in Plan mode. | A had an idle thread and a completed latest turn. Three successive Rust samples showed B active with waitingOnUserInput and its own inProgress turn. |
| The maintainer selected BETA. That choice was entered through the official TUI. | The same B turn changed to active with no wait flags, still inProgress, and later completed. This confirms wait and clearance in this isolated single-form trial. No request ID or simultaneous-wait disambiguation is available in the snapshot. |
| In a later A turn, the shell exited 7; the next read-only tool succeeded and A replied RECOVERED. | The thread retained its identity. Its new turn completed. Tool failure/recovery itself was visible in the TUI; the observer excludes tool items. |
| The first attempt to interrupt another A turn arrived after completion. A repeat used Escape while Working. | First attempt remained completed. On retry the same turn ID went from active/inProgress to idle/interrupted while B remained completed. |
| Two identical no-tool prompts were run after an observer crash. | Both supplied ACK. One-shot snapshots during the second run showed active/inProgress; a later sample showed completed. |

Sanitized projections of sampled metadata:

    A  idle    []                    T1 completed
    B  active  [waitingOnUserInput]  T2 inProgress
    B  active  []                    T2 inProgress
    B  idle    []                    T2 completed
    A  active  []                    T3 inProgress
    A  idle    []                    T3 interrupted

Status and latest-turn metadata are separate RPCs, not an atomic event stream. Polls can miss a short wait or an entire intervening turn. An absent latestTurn means its RPC was unavailable; null means an empty returned page. A failed sample or partial page must never imply termination, completion or clearance of a human wait. The one-shot client retains no cross-poll state.

## Cost, failure and non-interference

- Three independent Windows release-process samples with B waiting took 222, 133 and 123 ms. Three after submission took 88, 85 and 83 ms. Subsequent samples correlated the same IDs after reconnect. Input is bounded to 1 MiB with a five-second client deadline.
- The observer was forcibly terminated after its proxy child appeared. The proxy exited within the three-second check, the owner and both TUIs survived, and fresh samples took 101 and 112 ms. The official client subsequently completed more turns. This checks a specific crash point, not every possible termination schedule.
- The baseline ACK turn was detected after 5.37 s; the sampled ACK turn after 7.60 s. These single wall-clock measurements include inference, terminal polling and detection delay. They do not estimate causal slowdown. Managed observation adds no provider hook; existing user hooks remained in place. The Phase 1A synchronous hook result still exceeds the unchanged 500 ms target.
- The inherited approval policy was never. A real approval prompt or automatic permission resolution was not established; it was not forced by changing the policy. The live wait was a user-input form, not a permission grant. Managed continued-Stop and terminal failed-turn fixtures were not demonstrated. Phase 1A's continued Stop remains a counterexample to inferring completion from Stop.

## Independent Claude and WSL trials

On Windows the isolated Claude 2.1.332 print-mode control returned exit 1 and insufficient account credit. A separate background launch in an already trusted directory produced working and then busy/working. During the bounded 12-poll trial it did not yield a recognized waitingFor, done or failed. The owned background trial was stopped. No actual approval was answered. Phase 1B's Claude 2.1.287 evidence still covers ordinary Windows identity and busy/idle on that version; the two versions' evidence is not interchangeable.

The native WSL PATH check found neither a Linux Codex nor a Claude executable in the tested standard/user locations. The inherited Windows npm shim is not Linux provider coverage. Neither a native WSL shared owner nor a native Claude wait/outcome is demonstrated.

## Proposed v0.1 support matrix

C = CONFIRMED, P = PROVISIONAL_ONLY, S = UNSUPPORTED on the tested interface, U = UNTESTED. Cells concern an external observer. A terminal turn is not the same as process/session exit.

| Provider / mode / runtime | Identity | Active work | Human input wait / clear | Approval wait / clear | Turn completed | Interrupted turn | Terminal failure | Process exit |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Codex managed, Windows 0.167.0 | C | C | C (single wait) | U | C | C | U | S |
| Codex passive, Windows 0.167.0 | P | U | S | U | P (Stop only) | C (explicit Interrupt hook, Phase 1A) | U | P |
| Claude passive, Windows 2.1.287, Phase 1B | C | C (busy) | U | U | U | P | P | P |
| Claude passive, Windows 2.1.332 | P (one background trial) | P (busy) | U | U | U | U | U | U |
| Codex passive or managed / Claude passive, native WSL2 | U | U | U | U | U | U | U | U |
| Claude managed, either runtime | S | S | S | S | S | S | S | S |

**Workflow proposed for review.** Keep ordinary launches in passive mode with version-specific coverage. For users who need Codex turn identity and input waits on Windows, offer instructions for a private owner and the official TUI as an explicitly experimental opt-in. Display unknown for unsupported, stale or unavailable facts. Do not promise complete transition delivery, approval notifications, automatic launch, native WSL coverage or Claude turn success. The narrower release contract requires a maintainer decision in #3. The approved MVP contract has not been amended.

### Final artifact replay and cleanup

On Windows, the final code artifact (72a7130) observed two fresh official clients on the same owner, both idle before their first turn (656 ms). Submitting one prompt made only that thread active/inProgress (259 ms). The same turn then reported completed while the second thread stayed idle (458 and 456 ms on repeat). These external refresh times include proxy startup and owner load, and are not agent hook latencies. After the first pair of TUIs had exited the owner's loaded page eventually became empty; the empty page still cannot distinguish why. Both final TUIs were closed and only the dedicated owner was stopped. A new observer exited 1 with owner_unavailable in 101 ms. No Codex process targeting the trial socket remained. Windows and Ubuntu each passed 18 tests and built the release binaries for 72a7130.
