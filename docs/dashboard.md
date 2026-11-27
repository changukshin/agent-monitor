# Native Windows dashboard

Start the [collector](collector.md) and launch the dashboard as the same Windows user:

```powershell
agent-monitor-dashboard.exe 32941 "C:\AgentMonitor\private\api"
```

Use the collector's actual port and private API directory. For a source build, use `cargo build --release --locked -p agent-monitor-dashboard`. Closing the window exits only the dashboard. The collector and all agent sessions have separate lifetimes.

## Reading the display

The summary counts all grouped sessions. The drop-down filters the list; input waits sort first. Every state has a text label. **Idle / Completed** and **Idle / Interrupted** combine current idle activity with a known latest-turn outcome. Plain **Idle** carries no success implication. Claude passive supports activity only. Passive Codex discovery is provisional. Sample age and UTC observation time refer to the provider sample. The footer reports Windows source modes, versions and coverage; partial means the list can be incomplete. Polling can miss brief transitions. Approval, tool failure, process exit and native WSL2 are outside this version's state contract.

Within one instance, rows marked **(+)** have multiple observations for the same verified Codex owner lifetime and thread. Fresh confirmed managed facts take precedence. A degraded row can fall back to provisional discovery. Different owners remain separate even when labels or UUIDs match. Counts and filters use these same grouped rows. An uncertain or disconnected observation contributes no confirmed activity or outcome.

## Subscription and notifications

The network worker connects only to the IPv4 loopback address, reads the current token on each attempt and sends it only in the native WebSocket Authorization header. Each full frame replaces the previous view. Duplicate and older revisions are ignored. A gap or instance change requires a fresh snapshot; an invalid update forces reconnect. Disconnect clears the view. Backoff ranges from one to eight seconds. Read timeout is three seconds; the collector normally sends heartbeats every 250 ms. One current frame is retained, with a 1 MiB message limit, eight sources and 64 sessions per source. The UI checks for changes every 100 ms.

**Desktop notifications** starts unchecked each launch. Enabling it allows Windows toast requests for new, confirmed managed input waits and completions only. Snapshot differences never trigger a notification. The worker checks the referenced owner, session, turn, freshness, transition and coverage. Gaps, reconnects and stale or unsupported data suppress delivery. The footer reports submission or failure. Windows can suppress banners through notification settings, Do Not Disturb, session policy or unregistered unpackaged app identity. Submission does not prove a banner appeared. The dashboard does not register a shortcut or change notification settings.

## Toolkit and dependencies

We evaluated egui/eframe for its layout and rendering support. This Windows-only screen uses a table, filter and checkbox, all available as native common controls. Win32 controls provide keyboard navigation and accessible list text without shipping a graphics backend, font atlas or browser runtime. The trade-off is Windows-specific layout and message handling. The dashboard reuses the workspace's MIT/Apache-2.0 windows-sys bindings, Serde and tungstenite, and adds Microsoft's MIT/Apache-2.0 windows bindings for WinRT notifications. Cargo.lock pins the dependency graph. No network service, provider SDK or persistent history is needed.

See the [Windows GUI validation record](phase-3-validation.md).

## Windows notification diagnostics and delivery

The WebSocket reader only enqueues eligible alerts using a nonblocking send. A dedicated delivery thread handles one OS call at a time with at most eight additional jobs queued. A full queue drops the new alert. Jobs expire after five seconds; disabling notifications, disconnecting or accepting a new snapshot invalidates pending work. Delivery checks the source, owner, thread, turn, state-change time and freshness again immediately before calling Show. Windows owns a submission once that call begins.

Each native attempt initializes the calling thread with RoInitialize(RO_INIT_MULTITHREADED) and balances success with RoUninitialize after releasing the WinRT objects. The footer separates apartment initialization, notifier creation for the explicit AppUserModelID, settings query, XML creation/loading, toast creation and submission. A failed API call shows its stage and hexadecimal HRESULT. A successful settings query returning a disabled policy shows the NotificationSetting value instead. Diagnostics contain no exception message, paths, XML payload, token or session identifiers.

## Phase 4 packaging and banner acceptance

1. During an explicitly approved install, deploy the executable to its stable path. Create a Start menu Programs shortcut for the installing user (FOLDERID_Programs) with IShellLinkW, or the installer's equivalent. Set the executable target, working directory and the documented port/private-directory arguments. Never include the bearer token.
2. Open the shortcut's IPropertyStore. Set PKEY_AppUserModel_ID (System.AppUserModel.ID) to the exact string **AgentMonitor.Dashboard**, commit the property store and persist the .lnk using IPersistFile. This must match CreateToastNotifierWithId. Preserve the ID across upgrades, update the target when necessary and remove only the installer's shortcut at uninstall.
3. On a supported Windows account, read back the installed shortcut property, launch the installed dashboard and opt in. Using a real managed TUI, verify a **visible native banner** for a new input wait and completion. Record OS/build, identity, settings and actual banner evidence; Show succeeding or a footer message alone does not pass. Verify disabled notifications and subsequent collector updates. Respect the operator's OS settings and Do Not Disturb.
4. Successful banner delivery remains a Phase 4 release acceptance item unless the maintainer explicitly excludes it. The Phase 3 executable performs no shortcut registration, policy edits or installer actions.

[Microsoft's desktop shortcut and AppUserModelID procedure](https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid) and [desktop toast submission requirements](https://learn.microsoft.com/en-us/windows/win32/shell/quickstart-sending-desktop-toast).
