# Windows collector

Build with `cargo build --release --locked -p agent-monitor-collector`. Run `agent-monitor-collector.exe C:\AgentMonitor\private\config.json` in your own terminal. Ctrl+C stops the collector. Ordinary Claude and Codex launch commands continue to work independently.

## Configure sources

Choose a fresh private directory on a local NTFS volume for the configuration, a second for API credentials, and a third for the managed socket. Create these as the Windows user who will run the collector. For each **new empty directory**, configure an explicit inheritable grant before creating files or starting the owner. In PowerShell, with `$dir` set to that new directory:

```powershell
$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
icacls $dir /inheritance:r /grant:r "*${sid}:(OI)(CI)F"
icacls $dir
```

The collector checks ownership and ACLs. Use trusted provider executables and config paths that only the operator or administrators can modify. Never grant other users access to a control-capable Codex socket. In a terminal, start the dedicated owner with `codex app-server --listen "unix://<absolute-socket-path>"`. In each desired project terminal, run `codex --remote "unix://<absolute-socket-path>"`. The operator owns these processes. The collector starts only disposable read helpers. It never bootstraps a missing owner.

Example config (replace all paths, create the directories first, and choose a non-sensitive display label):

```json
{
  "port": 32941,
  "private_directory": "C:/AgentMonitor/private/api",
  "sources": [
    {"mode":"codex_managed","project":"Example","binary":"C:/Tools/codex.exe","cwd":"C:/Projects/Example","socket":"C:/AgentMonitor/private/owner/monitor.sock"},
    {"mode":"claude_passive","project":"Example","binary":"C:/Tools/claude.exe","cwd":"C:/Projects/Example"}
  ]
}
```

Optional `codex_passive` uses the same fields as managed Codex. It queries the explicitly configured private endpoint for a limited cwd-filtered inventory. Discovery supplies provisional identity only. Claude 2.1.287 activity requires a verified PID running the configured executable. Managed Codex requires Windows owner version 0.167.0. Configure each cwd explicitly. Use the provider's spelling of the absolute path.

## Native subscriber protocol

Connect to `ws://127.0.0.1:<port>/v1/sessions` using an RFC 6455 native client. Send one exact Host header matching that loopback address and port, one `Authorization: Bearer <token>` header, and omit Origin. Read the 64-character token from `dashboard.token` in the private API directory. Restart rotates credentials, disconnects all readers, and changes the instance ID. Graceful shutdown removes the token. After a crash the next start replaces it. Protect the token as a local secret. A same-user process able to read it can subscribe.

Each text frame is a complete replacement view with `schema: 1`, opaque `instance`, increasing `revision`, `runtime`, Unix epoch `sampledAtMs` (serialization time), `type`, ordered `sources`, and `alerts`. First frame, reconnect, and revision gaps produce a snapshot without alerts. A consecutive update may contain a newly confirmed input wait or completion; key it by instance, revision, source index, session, turn and kind. Ping/pong supports liveness. There are no application commands. Discard the previous view on disconnect.

A source carries mode (including provider), tested/observed version if available, project label, opaque owner identity, sequence, coverage, observed_ms (poll start), sample_cost_ms, unidentified count and a session map. Scope session keys by collector instance, source index and owner. Each session separates activity, confidence and latest_turn, plus visibility, sequence, observed_ms and changed_ms. Latest-turn kind is unavailable, empty (successful empty list), or known with ID and status. Confidence is confirmed, provisional, unsupported or unknown. Compute conservative sample age from the source/session observation time. Do not infer completion from idle. Visibility can be present, missing, offline or stale; coverage can be starting, complete, partial or a static failure reason. Complete refers only to the configured query. Absence is never proof of exit.

## Limits and privacy

At most eight sources poll independently, one in-flight poll per source, with a six-second total deadline. After a successful poll Codex waits one second and Claude three seconds. Failed polls back off exponentially up to 30 seconds. Managed Codex inspects one page of eight loaded threads; discovery inspects one page of 32. The response budget is 1 MiB. The store keeps at most 64 sessions per source, with eight retired turn IDs per row. Unseen data loses freshness at 15 seconds and expires at 120 seconds. Output uses metadata allowlists. No raw transcript, prompts, commands, tool output, credentials or cwd are emitted in dashboard frames. Keep project labels non-sensitive.

The native listener admits eight concurrent readers, bounds handshake time to two seconds and write time to 800 ms, and checks for changes every 250 ms. Client data frames close the connection; incoming frames/messages are limited to 1 KiB. Slow or disconnected readers can miss transitions. The API is a sampled current view with no durable history. There is no hook installation, command/approval forwarding, remote control, or agent lifecycle management.

See the [validation record](phase-2-validation.md) for the exact builds and observed behavior.

## Dashboard identity across Codex sources

Within one collector instance, group managed and passive Codex rows for presentation only when their verified owner lifetime and thread UUID both match. Keep per-source sequence, age, confidence and coverage independent. Present fresh confirmed managed facts in preference to provisional discovery. When managed evidence becomes unavailable, display uncertainty or a discovery-only row; passive evidence cannot clear a wait, establish an outcome, or generate an alert. Never group by UUID alone, across owners, or across collector restarts.

Missing source executables and working directories report unavailable and retry independently. A correlated per-thread request rejection marks the managed page partial. Other coherent rows remain usable. Transport, identity and malformed-response failures invalidate the source sample.
