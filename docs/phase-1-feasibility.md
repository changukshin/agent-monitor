# Phase 1 — observation feasibility spike

**Status: INCOMPLETE / gate not passed.** This is disposable experimental code for validating the requirements in [the MVP specification](mvp.md#2-feasibility-gate--before-production-implementation). It is not a production monitor or a supported agent integration.

## What is implemented

- A dependency-free Node.js 22+ probe in [spike/probe.mjs](../spike/probe.mjs). It reads one hook invocation from stdin, retains only allowlisted metadata, and sends an observation to a single loopback receiver. All observations are provisional, including Stop and PermissionRequest.
- An experimental receiver at 127.0.0.1:8787/observe, authenticated with a random local token. It prints accepted metadata to its terminal, does not expose a read API, and does not write session history.
- Optional subprocess relay: a Linux-side probe sends sanitized metadata via stdin to a Windows-side Node process without shell interpolation. No raw provider payload crosses that boundary.
- A 350 ms in-process watchdog (300 ms relay subprocess timer); no retry loop. The 500 ms end-to-end target includes shell/runtime startup, and remains unverified on the target platforms.
- A simulated unresponsive collector mode and automated tests. No configuration installer, state reducer, dashboard, or approval/control API.

The output contract is a candidate: empty JSON object for Codex Stop, empty stdout for the other allowlisted hooks, and exit code zero. Do not install this experiment into an existing working configuration. A missing runtime, shell startup failure, external timeout or OS process termination is outside the probe's control; verify these scenarios in the actual provider before asserting non-interference.

## References and evidence classification

Official references reviewed on 2026-11-26:

- [Codex hooks](https://developers.openai.com/codex/hooks): common stdin fields, command hooks, event-specific output, Stop, PermissionRequest and Interrupt.
- [Claude Code hooks](https://code.claude.com/docs/en/hooks): command hooks, input/output, notifications, decisions and event-specific failure handling.
- [Microsoft WSL networking](https://learn.microsoft.com/en-us/windows/wsl/networking) and [WSL interoperability](https://learn.microsoft.com/en-us/windows/wsl/interop): NAT versus mirrored networking and Windows executable invocation from Linux.

Documentation suggests candidate observations; it is not live evidence. The probe's JSON schema is experimental and deliberately smaller than the proposed production contract. In particular, it omits cwd and transcript paths, command arguments, messages and tool responses. A random observation ID is assigned locally, not a provider event ID. The observedAt value is the local capture time, not a provider ordering guarantee. Session IDs are local metadata; redact them before sharing results publicly.

### Provider support matrix (initial hypotheses)

All entries below are **untested on real CLIs**, not support declarations. The probe emits only provisional evidence; no row is confirmed.

| Desired state / transition | Codex candidate | Claude Code candidate | Limitation to test |
| --- | --- | --- | --- |
| Session registration | SessionStart | SessionStart | Registration alone does not establish IDLE or RUNNING |
| Running | UserPromptSubmit, PreToolUse, PreModelRequest | UserPromptSubmit, PreToolUse | Prompt or tool may be blocked; model attempt is not the entire turn |
| Human approval pending | PermissionRequest | PermissionRequest, Notification(permission_prompt) | Another hook may resolve permission; notifications can be delayed or absent |
| Human input pending | No verified hook mapping | Elicitation, Notification(elicitation_dialog) | Form requests can be answered automatically; not all input mechanisms are covered |
| Wait resolution | Subsequent relevant tool/turn observation | ElicitationResult, PermissionDenied, subsequent tool/turn observation | No general verified resolution correlation; absence is not resolution |
| Turn completion | Stop | Stop, Notification(idle_prompt) | Stop may be vetoed; idle notification is delayed and does not alone establish normal completion |
| Turn failure | PostModelRequest is attempt-level only | StopFailure | Tool/model failure may recover; verify entire-turn semantics |
| Interruption | Interrupt | No verified general hook mapping | Test actual cancellation versus idle or exit |
| Session termination | SessionEnd | SessionEnd | Normal closure differs from crash or disconnection |
| Idle | No verified hook mapping | Notification(idle_prompt) | Cannot infer idle from silence |

Codex and Claude Code CLI versions: **not available in the authoring environment**. Desktop apps are out of scope. Investigate supported read-only observation interfaces if hooks cannot establish the core states; do not force agents into a wrapper or scrape private transcripts as a shortcut.

### Transport matrix

| Environment | Candidate | Result |
| --- | --- | --- |
| Linux container (not WSL) | Direct HTTP loopback | Synthetic smoke test passed |
| Linux container (not WSL) | Node subprocess relay | Synthetic smoke test passed; not Windows interop |
| Windows native | Direct HTTP loopback | Untested |
| WSL2 NAT | Windows Node helper through interop | Untested |
| WSL2 mirrored | Same helper or direct loopback | Untested |
| WSL2 interop unavailable | No default workaround | Untested; report limitation |

## Quick local experiment

Use a disposable clone/test workspace and a user-private home directory. Do not run against production project settings or existing sessions. Verify Node version with node --version (22 or later).

From the repository root, in a terminal on the intended collector host:

    node spike/probe.mjs init
    node spike/probe.mjs serve

Init creates a random token at ~/.agent-monitor-spike/token using exclusive creation and restrictive Unix modes. It will not overwrite a token. On Windows the file inherits directory ACLs; verify with icacls that only appropriate trusted principals have access **before starting the receiver**. Unix users should verify directory mode 0700 and file mode 0600, including pre-existing directories. Do not place the token in a repository, paste it in issue reports, put it in a command argument, or redirect raw hook input to a file. To rotate a spike token, stop the receiver, remove only the owned token file, then run init again and restart. AM_SPIKE_TOKEN_FILE may point to an existing private token file for a test transport; init uses only the default path.

The following synthetic smoke test sends a harmless Stop event. It does not invoke an agent.

PowerShell (native Windows):

    '{"hook_event_name":"Stop","session_id":"synthetic-windows","turn_id":"turn-1","stop_hook_active":false}' | node spike/probe.mjs hook codex Stop native-windows

Bash (Linux, direct path only when loopback reaches the collector and the same token is available):

    printf '%s' '{"hook_event_name":"Stop","session_id":"synthetic-linux","turn_id":"turn-1","stop_hook_active":false}' | node spike/probe.mjs hook codex Stop linux-direct

Expect the hook terminal to print only {} and the collector to print a provisional metadata record. No record is proof of delivery failure or absence of evidence, not an agent state. Confirm that the expected collector is listening before testing.

### WSL2 to Windows

Start the receiver in Windows. Keep a checkout of this same commit on Windows and a probe checkout in WSL. Use the actual installed Windows node.exe path and the actual Windows path of the Windows checkout. Example from Bash, substituting paths as needed:

    printf '%s' '{"hook_event_name":"Stop","session_id":"synthetic-wsl","turn_id":"turn-1","stop_hook_active":false}' | node spike/probe.mjs hook codex Stop wsl-ubuntu '/mnt/c/Program Files/nodejs/node.exe' 'C:\dev\agent-monitor\spike\probe.mjs'

The WSL probe starts the Windows helper with argument separation and passes sanitized JSON on stdin. The Windows helper reads its Windows home token. Use a distinct hostId for each distribution. Verify the same Windows receiver sees the native and WSL records and measure total invocation time, including the Windows runtime startup. For mirrored direct delivery, explicitly set AM_SPIKE_TOKEN_FILE to the verified mounted private Windows token path and omit the helper arguments. No network mode, firewall or interop changes are required or allowed by this procedure. If interop is disabled, the helper may fail to start; do not change settings to conceal this limitation.

## Real provider trial (manual, isolated only)

1. Record provider versions (codex --version, claude --version), node --version, Windows version, wsl --version, distribution name/version and actual networking mode (wslinfo --networking-mode if available). Never publish account details or token contents.
2. Use a fresh throwaway workspace outside existing repositories. Review inherited user/system/plugin hooks first; do not overwrite them. Run an uninstrumented baseline. Confirm the current provider docs for the exact installed version and command-shell behavior.
3. In the disposable project only, add a single test hook for one event at a time. For Codex, the candidate file is .codex/hooks.json; for Claude Code, .claude/settings.local.json. Use an absolute path to the probe. Do not add the following examples if a file already exists: inspect it and make a separate disposable workspace instead.
4. Test actual behavior and the output contract with a single hook, then with another controlled test hook. Expand to other documented events only after the first trial passes. Explicitly approve trusted hooks through the provider's own workflow; do not change approval or trust policies.
5. Remove only the disposable project hook configuration you created (or delete that disposable project), stop the receiver and remove the test token when finished.

Candidate Codex project config for Stop (adapt absolute path; the hook command has a provider timeout in seconds):

    {
      "hooks": {
        "Stop": [
          {
            "hooks": [
              {
                "type": "command",
                "command": "node C:/dev/agent-monitor/spike/probe.mjs hook codex Stop native-windows",
                "timeout": 1
              }
            ]
          }
        ]
      }
    }

Candidate Claude Code project config for Stop (verify support for onFailure in the installed version before using it):

    {
      "hooks": {
        "Stop": [
          {
            "hooks": [
              {
                "type": "command",
                "command": "node C:/dev/agent-monitor/spike/probe.mjs hook claude Stop native-windows",
                "timeout": 1,
                "onFailure": "continue"
              }
            ]
          }
        ]
      }
    }

These are templates, not automatically installed settings. Paths with spaces require quoting appropriate to the provider's actual shell. In WSL use the Linux probe path, a unique hostId, and the explicit helper arguments shown above. For other event types, use the exact event name in both the settings key and the probe command; matchers and available event types are version-specific. Do not register unsupported hook names. Claude Code's native Windows shell and failure semantics need particular verification.

### Scenario checklist

For every tested provider/environment pair, compare visible agent behavior with the provisional receiver events and the uninstrumented baseline:

- Normal prompt and turn completion; a second prompt in the same session; normal session exit.
- Stop continued by a separate one-time test hook; verify the probe's Stop observation does not establish completion.
- Human approval prompt displayed, then accepted or denied manually; separately test an automatically resolved request only in an isolated fixture under an existing explicit test policy. Never broaden real project permissions.
- Prompt rejected by a separate test hook; tool failure followed by recovery; real human input request; interruption.
- Collector absent, collector unresponsive (restart with serve --hang), malformed input, missing token, unavailable helper, and runtime startup failure. Confirm the agent still behaves like the baseline and does not see injected context or decisions. Measure added latency externally (PowerShell Measure-Command, Bash time, or an equivalent harness); record multiple trials and the worst case.
- Native Windows and at least one WSL2 mode reaching the same collector. Check child process cleanup after timeout, including interop behavior.

Use this report format for each scenario:

| Provider/version | OS/mode | Scenario | Visible outcome (baseline / probe) | Sanitized observation sequence | Max added ms / trials | Verdict |
| --- | --- | --- | --- | --- | --- | --- |
| To be measured | To be measured | Normal completion | Pending | Pending | Pending | Untested |

Keep only allowlisted observation records in fixtures. Replace session/turn/observation IDs consistently before committing them. Do not publish real logs, transcripts, prompts, tool calls, credentials or paths.

## Authoring-environment validation

Linux container, Node.js v22.16.0. Codex and Claude Code executables, Windows and WSL2 were not available. The following results use synthetic input, not real provider events.

    node --check spike/probe.mjs
    node --test spike/probe.test.mjs

Result: 4 tests passed, 0 failed. Covered metadata allowlisting, provisional-only evidence, event mismatch, exclusion of subagent events, receiver authentication/Origin/schema/content-type rejection, malformed and incomplete input, missing token and missing relay executable.

Additional manual synthetic smoke tests used a temporary token and receiver on Linux:

| Path | Observed wall-clock time (single trial) | Result |
| --- | ---: | --- |
| Direct delivery | 87 ms | Sanitized event received |
| Node subprocess relay | 180 ms | Sanitized event received |
| Unresponsive receiver | 397 ms | Neutral candidate output, exit zero |
| Receiver absent | 56 ms | Neutral candidate output, exit zero |

These are single-trial process durations, not measured added provider latency or a guarantee of the 500 ms target. The automated watchdog test uses a loose regression bound (1500 ms), not a release performance assertion. The relay trial used Linux Node, not Windows interop. No real fixtures or completed support matrix are available yet.

## Gate decision and next action

**NO-GO for the production collector/dashboard until the feasibility gate is satisfied.** The experiment can be reviewed and used in a disposable test environment. A maintainer or a developer with the target CLIs and Windows/WSL2 access must run the scenario checklist, document actual versions and sanitized evidence, and update the support matrix. If actual human attention or turn completion cannot be confirmed via supported observations, explicitly escalate to the maintainer for a scope decision. Do not convert a provisional event into a confirmed state simply to pass the gate.
