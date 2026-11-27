# Phase 2 validation, 2026-11-27

Base `0f156373`. Windows 10.0.26300, native Codex CLI 0.167.0 and Claude Code 2.1.287. Two official Codex TUIs shared a manually started private owner; two ordinary Claude TUIs ran the supported executable concurrently. Three independent configured sources included passive Codex against the same private endpoint. Existing authorization and project trust were retained. No synchronous monitor hook was installed.

## Windows observations

| Trial | Result |
| --- | --- |
| Four concurrent clients | Two managed thread IDs and two verified Claude session IDs remained distinguishable. Additional Claude inventory rows without verified running executables stayed unknown. Passive Codex returned provisional IDs only. |
| Managed normal turn | TUI answer observed; running with active turn changed to idle with completed turn. A connected reader received a completion transition. |
| Managed human input | The official TUI displayed a two-choice form while the collector reported waiting_input. The operator selected BETA. The same session was sampled running again and then idle/completed. Existing subscribers observed input-wait and completion alerts. |
| Managed interruption | During a new turn, one session was sampled running/active while its sibling remained idle/completed. Escape in the active official TUI resulted in idle/interrupted. |
| Passive Claude | A prompted client's listing changed to busy and back to idle. The collector exposed confirmed running/idle, with no latest-turn or attention inference. |
| Collector crash and restart | Forcibly ended only the test collector. The dedicated owner and official clients continued. Restart rotated the credential, rejected the old bearer and returned a fresh instance snapshot without replay. The subsequent interrupted-turn trial used the surviving TUI. |
| Unresponsive then offline owner | Suspended the disposable owner; the managed sample timed out after 6,018 ms and both rows showed offline. The independent Claude source advanced two sequences. Resuming the owner restored complete coverage with zero reconnect alerts. After exiting the TUIs and stopping the owner, both Codex sources became unavailable; Claude continued polling. |
| Local API boundary | Wrong bearer, foreign Origin and wrong Host were denied. Application data and oversized data disconnected the reader. An unfinished handshake did not block a second reader. Eight readers were admitted, the ninth was rejected and capacity recovered on disconnect. A disposable Everyone-readable config was rejected with acl_not_private. |

Live interaction, reader denial and crash trials used the locked Windows release artifact at `f1c8fabfb8ee52031468308b983d660f398dbb2c`. The final implementation at `fd7c20c3bd7d472f7176546f914ece607f55337b` was then attached to the same sessions. It passed the three-source inventory, ACL, eight-reader, timeout/recovery and offline trials. Final executable SHA-256: `2ecc20bc01f908d00b91a3ef576ce539d8a24d4ebcc39452616474251d858f66`. [Locked Windows and Ubuntu tests and release builds](https://github.com/changukshin/agent-monitor/actions/runs/45960152192).

One post-restart snapshot measured managed Codex 133 ms, Claude 502 ms and passive Codex 136 ms of sampling work. The three samples were 603 ms old when serialized. These are individual observations. Polling, serialization, and agent response latency measure different intervals. There is no synchronous request on the agent's execution path. No population latency percentile or desktop rendering time was measured.

## Synthetic coverage and limits

The production regression suite checks non-atomic reads and changed turn IDs, old terminal facts during activity, missing/empty turns, input flag combinations, duplicate/out-of-order samples, completed/interrupted outcomes, source identity changes, partial and empty listings, capped inventory, offline/stale expiry, passive scopes, Host/Origin/token handling, and snapshot resynchronization. The workspace also retains the separate Phase 1 tests. The interrupted and completed labels describe turns; neither means the session exited. No confirmed approval, failure or process-exit status is provided. Linux CI exercises portable logic and compilation; provider validation applies to native Windows only.

The first owner startup required correcting the disposable socket directory's inherited ACL to an explicit private grant. A first client-harness assertion expected a WebSocket close exception, while Windows reported a socket abort on rejected data. The harness accepted either disconnect signal and the boundary trial passed on rerun. No provider configuration or authorization change was required.

The sampled interface can miss short transitions. Subscribers receive the latest full view after a gap, without retrospective alerts. The trials do not establish behavior on other provider versions, native WSL, a rendered dashboard, or production fleet loads.
