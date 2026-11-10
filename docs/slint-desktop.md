# Bricks Slint desktop

This is the first implemented product frontend, not the older WinForms diagnostic launcher. It has Computers, Share this PC, Settings, and a top-level About screen. No application animations, background capture, automatic sharing, or sample computers are added.

## Run

Double-click **Start Bricks** in the workspace, or `dist\BricksDesktop\Bricks.exe`. No Rust installation or terminal is required to run the built app. Keep the entire portable folder together when copying it to another Windows 10/11 x64 PC. A signed installer and clean-machine prerequisite validation remain pending.

1. On Computers, choose **Add PC**, enter a friendly name and its LAN or tunnel IP address, then Save. The default port is already filled in.
2. Select a saved PC to **Edit** or **Forget** it. Forget asks for confirmation; it removes the entry without changing separately established peer trust.
3. Under Settings, change the computer name or the default port for new entries, then **Save settings**. Changes survive restarting the app.
4. On Share this PC, **Set up this PC** creates or validates existing Windows current-user DPAPI identities. It never replaces an existing key, starts a listener, or grants trust. Public certificate paths and fingerprints appear only under Settings > Advanced after setup.
5. Settings > Advanced offers **Check recovery**, **Check local QUIC**, **Check network recovery**, **Cancel check**, **Export last report**, and **Open data folder**. These invoke the actual Rust modules through the agent, with separate pending, cancellation-requested and completed states. They use recording injectors; no automated operation sends keys to Windows.

**Connect and Start sharing remain disabled.** Live networked OS input, short-code pairing, discovery, native video/audio and background sharing are not implemented. The separate headless agent is implemented. A saved address means neither paired nor online. This frontend does not complete the remote-control or gaming acceptance milestones.

## State and architecture

`bricks-control` provides the UI-independent controller and bounded typed IPC SDK. `bricks-agent` owns state, storage and diagnostic operations in a separate user-session process. `bricks-ui` contains Slint views and presentation logic; its SDK worker handles named-pipe traffic while Slint stays on its event thread. There is no per-input JSON or per-operation subprocess. No input/media worker starts just by opening the app.

The [separate agent and SDK subset](single-pc-agent-and-recovery.md) now implement restricted local pipes, atomic snapshots, bounded state events, operation IDs, cancellation and restart reconciliation. The broader [control contract](control-sdk.md) still gates live input, pairing and external cleanup supervision. The controller/SDK have no Slint dependency and can serve headless consumers.

Data lives under `%LOCALAPPDATA%\Bricks`: `desktop.json` is schema 1, `desktop.lock` prevents concurrent writers, existing identities remain in `identities`, public exports in `public`, and exported reports in `reports`. Diagnostics do not overwrite previous reports. `desktop.json.pending` is a staged write, never authoritative at startup. The controller writes/syncs the new file before replacement and updates memory only after success. This is not a guarantee against storage hardware or power failure.

State is strictly decoded and size bounded (128 KiB); unknown fields/schema versions, malformed data and invalid settings fail visibly and preserve the original. Device names are 1–64 characters, at most 64 devices are saved, IDs remain stable across edits, addresses must be numeric unicast IPv4/IPv6 with a nonzero port, and duplicate endpoints are rejected. Edits check the current revision. Existing saved endpoint ports do not change with the default port. These desktop settings do not silently rewrite the engine's TOML configuration.

Recent activity holds at most 100 timestamped events in memory. Important errors show in the status area and active editor. One operation runs per frontend; close requests cancellation and waits for its result. Connection loss marks agent state unavailable, disables mutations and starts reconnection/snapshot refresh without replaying an uncertain write. Auto-started agents exit after 30 idle seconds; explicit CLI agents can stay running headlessly. No tray or background sharing is implied.

## Build and validate

```powershell
powershell -ExecutionPolicy Bypass -File tools/build-desktop.ps1
powershell -ExecutionPolicy Bypass -File tools/test-desktop.ps1
```

The build script uses user-local Cargo if needed, builds the UI with the size-oriented `desktop` profile and the agent with its optimized release profile offline from the lockfile, packages both executables and notices, and points Start Bricks at this app. Keep both executables together. On a fresh developer installation, fetch locked dependencies first. `tools/build-ui.ps1` delegates to this product build; `tools/build-diagnostics.ps1` builds the old diagnostic launcher separately.

The UI-only `desktop` profile uses size optimization, full LTO, stripped symbols and abort-on-panic. The separate agent retains panic unwinding and cleanup guards. Agent entry/control/config parsing use size optimization and its executable uses full LTO; packet/core/QUIC dependencies retain speed optimization. Accessibility, software rendering, Unicode, notices and all functional screens remain enabled; compressed download size is not substituted for installed size.

Controller tests cover persistence, stable IDs, revision conflict, duplicates, invalid addresses/settings, capacity, corrupt/oversized data preservation, a second writer, failed writes, idempotent Windows identity setup and report export. The UI smoke test drives the actual Slint callbacks/worker in an explicit fresh data directory, checks CRUD/settings/error feedback/recovery/export, and captures rendered screens. Its results stay in `artifacts\desktop-smoke-*`. It does not create real-account identities, capture typing or inject OS input. Windows DPAPI/QUIC tests need the normal Windows user profile, not the isolated sandbox profile.

`tools/test-desktop-accessibility.ps1` additionally exercises the actual Windows accessibility controls: invoke Add PC, set the named edit fields, invoke Save, edit again, and verify persisted values. It runs in its own explicit data directory and closes only its test app. This checks widget wiring and accessible names without sending OS key input. Keyboard/screen-reader/high-DPI acceptance beyond these checks remains manual work.

`--data-dir PATH` selects an explicit isolated data root for development. `--smoke-test` requires that option and refuses an existing `desktop.json`. It is not the normal launch flow.

Slint is pinned to 1.18.1 with winit, software rendering, accessibility and the required compatibility feature. Qt, application system-tray support and GPU rendering backends are not enabled. See [distribution notices](../ui/desktop/NOTICES.txt) and the bundled [Slint royalty-free license](../ui/desktop/Slint-Royalty-Free-2.0.md). The top-level About screen contains the unmodified AboutSlint attribution widget. This desktop license choice does not assign a license to the Bricks source. Public release still needs full transitive notice/security review, signing and installer validation.
