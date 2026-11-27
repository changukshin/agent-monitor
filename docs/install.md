# Using Agent Monitor on Windows

Agent Monitor shows which supported coding sessions are working, waiting for input, or have finished a turn. The Windows x64 distribution contains two native Rust executables. Setup uses the Windows PowerShell 5.1 supplied with Windows. You do not need Rust, Cargo, Node.js, a browser or a monitor account. Provider CLIs and their existing authorizations are separate prerequisites.

## Install

1. Download the versioned Windows ZIP and its .sha256 file from the same approved distribution. Compare the checksum using `Get-FileHash .\agent-monitor-0.1.0-windows-x64.zip -Algorithm SHA256`. The checksum detects corruption; obtain it from a trusted source.
2. Extract the ZIP to a local folder you control. Keep it for upgrades and removal. Open Windows PowerShell in the extracted folder and run `powershell.exe -NoProfile -File .\Setup.ps1`. Review the scripts before running them. If organization policy blocks scripts, use your organization's approved procedure.
3. Default files go under `%LOCALAPPDATA%\Agent Monitor`. The explicitly invoked setup creates private directories and a per-user Start menu shortcut with the notification identity AgentMonitor.Dashboard. It checks the saved identity, target, working directory and arguments. It starts no process. A different new local folder can be specified with `-InstallRoot "C:\path with spaces\Agent Monitor"`. Keep the same path on later runs.

## Configure and start

Open the installed config.json in a text editor. Keep port 32941 and the generated private_directory for the installed shortcut. Replace the example provider binary and project cwd with their real absolute paths. Use a non-sensitive project label. The default example observes ordinary Windows Claude Code 2.1.287 passively; only session identity and busy/idle are supported. Unavailable sources retry independently.

For managed Windows Codex CLI 0.167.0, create a new short private socket directory using the ACL commands in collector.md. In a dedicated terminal run `codex app-server --listen "unix://<absolute-socket-path>"`. In each desired project terminal run `codex --remote "unix://<absolute-socket-path>"`. Add a codex_managed source with the Windows binary, project cwd and that socket to config.json. Optional codex_passive uses the same fields and supplies provisional inventory. See collector.md for complete examples. The owner and official clients belong to you. Close clients normally and stop the dedicated owner in its own terminal when finished.

From Windows PowerShell:

```powershell
$dir = Join-Path $env:LOCALAPPDATA 'Agent Monitor'
& "$dir\agent-monitor-collector.exe" "$dir\config.json"
```

Open **Agent Monitor Dashboard** from Start. Check **Desktop notifications** to opt in for this launch. See dashboard.md for filters and confidence labels. Polling can miss short transitions. Idle alone does not establish a completed turn. Native WSL2, approval detection and Claude turn outcomes are outside v0.1 support.

## Upgrade or remove

Close the dashboard and stop the collector with Ctrl+C. Leave your agent terminals and private Codex owner running if desired. Extract the next trusted package and rerun Setup.ps1 with the same installation root. The binary hashes in build.json and the local installation receipt control replacement. The installer preserves edited configuration. An unfamiliar root, shortcut, ACL, modified executable or in-use file can block an operation; inspect the reported item and retry after addressing it.

To remove application binaries and the owned shortcut, run `powershell.exe -NoProfile -File .\Setup.ps1 -Action Uninstall` from the extracted package. Add the same -InstallRoot if customized. Data, config and the ownership receipt remain for reinstall. Graceful collector shutdown removes dashboard.token. A crash can leave that credential file on disk; the next launch rotates it. Once you have stopped the collector, you may delete the retained token in the installed api directory and remove your own data after inspecting it. Do not remove a socket belonging to a running owner.

## Troubleshooting and contributing

Disconnected means the dashboard has no current view. Check that the collector is running, the configured port is available and the private directory matches the shortcut. Keep the ACL private. A notification error shows the WinRT stage and HRESULT. Check the shortcut and existing Windows notification policy; submitting a toast does not guarantee a visible banner. No global settings are changed by setup.

For a bug report, include Windows build, provider versions, build.json commit, scope, and sanitized diagnostics. Omit tokens, transcripts, raw event data and private paths. For contributions, follow docs/mvp.md and run locked workspace tests and release builds on Windows and Ubuntu. Submit a draft PR with actual versus synthetic validation clearly identified.
