# Development diagnostic launcher

This is the existing WinForms development launcher, separate from the implemented first [Slint product frontend](slint-desktop.md). The target persistent Rust agent and IPC SDK remain pending; see [frontend design](frontend-design.md) and [control contract](control-sdk.md). This document preserves instructions for the development tool.

Open `dist\Bricks\Bricks.exe` directly for this developer tool. **Start Bricks** now opens the [Slint product frontend](slint-desktop.md) after `tools/build-desktop.ps1` packages it. No terminal is needed after either portable app has been built. This developer preview exposes existing diagnostic tools and verified recording-only connections; desktop control and video remain pending.

## On one PC today

1. From Home, click **Run automatic checks**. Recovery and Local connection should each display **PASS**.
2. Click **Open test** for Keyboard & mouse.
3. In the separate input window, click **Run test**. You can use the button instead of F8/Fn.
4. Close that window to see its result in Bricks. Closing without a test is labeled **NOT TESTED**, never PASS.
5. Use **Export last report** if you want to save the most recent diagnostic as JSON.

Results have visible running, pass, failure, review, not-tested and cancelled states. Only one job runs at a time; navigation stays available. **Cancel check** stops recording-only checks. For the real input fixture it requests a normal window close so the fixture can perform cleanup. Closing Bricks during a job requests cancellation and waits for that operation to finish.

Automatic checks never submit real OS keys. The separate input fixture submits its marked test pair only when you explicitly click its Run test button or press its F8 gesture. No background keyboard capture or typed-text recording is added by the shell.

## When the second PC is available

Copy the complete `dist\Bricks` folder to it, including the `bin` directory and `Bricks.exe.config`. Open `Bricks.exe` on both PCs.

1. On **Connection**, choose **Host** on the PC you will eventually control and **Client** on the other PC.
2. Click **Set up this device** on each. This creates a protected identity only if one does not already exist; otherwise it loads/validates the existing identity. It does not replace an existing private key.
3. Click **Save public identity** on both. Copy each exported `.der` file to the opposite PC.
4. On both PCs, click **Load other PC's identity**. Compare **all four fingerprint lines** in the confirmation against **Your device** on the other PC, using a trusted comparison such as viewing its screen. Approve only when they match. The default answer is No. This verifies identity-file exchange; it is not short-code pairing.
5. On the host, select its LAN or tunnel address and click **Start host test**. Use a concrete address shown in the list. Non-link-local addresses are preferred over self-assigned `169.254.x.x` addresses; the selected address must still be the correct route for the client.
6. On the client, enter that host address and click **Connect & run test** within 30 seconds. Leave the port at its default on both PCs unless it conflicts with another app.

A successful connection test reports 200 synthetic transitions and ends. The host permits only the configured verified client for that invocation. Pressing Start host test is explicit approval for this one recording session. It does not start desktop sharing. The UI does not bypass mutual TLS, saved control permission, or ownership admission.

Pairing is role-specific: the host permits the verified client to request control; the client trusts the host without granting reverse control. Reapproving the same fingerprint revokes its old grant before writing the newly confirmed trust. **Remove pairing** revokes that saved peer. A GUI preference alone cannot establish trust; the Rust store and authenticated session remain authoritative. External changes to stored trust can therefore make a saved peer fail connection.

The same addresses work over reachable LAN or an existing Tailscale/WireGuard/IP tunnel. No custom NAT traversal, discovery, relay, router configuration or automatic firewall rule is added. If Windows blocks the host, allow the intended executable only on the appropriate private network. UI network errors include address/pairing/firewall guidance; full backend diagnostics remain in Checks / details.

## Portable build and prerequisites

The development bundle requires x64 Windows 10/11, .NET Framework 4.8, and the x64 Microsoft Visual C++ v14 runtime imported by the current Rust development binaries. Rust and the .NET SDK are not needed on a second PC to run the compiled bundle. The development PC has these runtimes. Microsoft provides the [supported C++ runtime downloads](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist); use the current supported x64 version if a second PC reports a missing library. The bundle does not install prerequisites automatically.

Build from the workspace:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build-diagnostics.ps1
```

The script uses the installed Windows Framework C# compiler, builds the locked Rust tools offline, copies the three tools into `dist\Bricks\bin`, embeds the TOML examples, includes the application config/manifest/icon, and creates the workspace's **Bricks Diagnostics** shortcut. `-SkipRustBuild` repackages already-built Rust executables. The product's Start Bricks shortcut is separate. Copied bundles launch via their own `Bricks.exe`. Build/run as your ordinary Windows account.

This is an unsigned portable developer preview with debug Rust binaries. A signed installer, clean-machine prerequisite installation, updates, discovery, PAKE short-code pairing and a first-user usability acceptance study remain future work. M5 is not complete.

## Architecture and configuration

`ui/windows` is a small C# WinForms presentation/controller layer using the Windows Framework already installed on the development PC. Input, identity, QUIC and recovery remain in Rust. This puts no managed runtime on the input hot path and adds no Rust library dependencies. It is an initial Windows shell choice; future media/game session control requires a reviewed persistent agent interface, not a subprocess per input event.

The shell invokes only the three packaged tools by explicit paths, without a command shell. Windows arguments are escaped individually; addresses cannot become commands. Both output pipes are drained concurrently with a 1 MiB cap per pipe. Recording-only jobs have timeouts/cancellation. The real-input fixture has no force-kill timeout and receives a graceful close request. This is not an external crash watchdog and must not be reused as a force-kill policy for future live injection sessions.

Connection files are generated from the same embedded host/client TOML examples; the selected numeric IP and port are substituted, then the Rust CLI's strict role/config validation runs before networking. Recovery checks use the Rust binary's embedded defaults. Resolved configs/hashes stay in tool reports where emitted. UI-selected addresses, port, role and public peer fingerprints live in the separately versioned `%LOCALAPPDATA%\Bricks\ui\settings.json`; writes use a temporary file plus replacement. No private keys are stored in UI preferences or exports. Existing DPAPI identity paths are unchanged.

Public certificate import reads a bounded snapshot and hashes those exact bytes for approval; the approved bytes are then passed to the Rust trust command. A later change to the selected source file cannot change the certificate being approved. Temporary public/config files are deleted by their known filenames; cleanup never walks or recursively removes a production identity directory. Reports are kept in memory and exported only on request. They can contain public fingerprints, addresses and local diagnostics; no typed-text trace is added.

The app opts into per-monitor DPI support using the [Framework configuration recommended by Microsoft](https://learn.microsoft.com/en-us/dotnet/desktop/winforms/high-dpi-support-in-windows-forms). Pages scroll on smaller windows. Multiple monitor/scaling combinations and screen-reader behavior still need interactive usability checks. Process handling follows Microsoft's [redirected-output guidance](https://learn.microsoft.com/en-us/dotnet/api/system.diagnostics.processstartinfo.redirectstandardoutput?view=netframework-4.8.1) by draining both pipes concurrently.

## Verification

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/test-ui.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File tools/test-ui.ps1 -Gui
```

The controller suite checks actual child-process argv round trips, simultaneous stdout/stderr, nonzero exits, bounded output, cancellation, timeout, numeric IPv4/IPv6/tunnel endpoints, saved-state validation, report completeness/cleanup checks, and generated settings accepted by Rust. The GUI suite requires an interactive desktop and normal profile access for DPAPI. It generates and pretrusts only its own temporary identities, exercises actual buttons and recording-only QUIC in both UI roles, checks visible results/cancellation, and closes its own fixture without sending keys. Its profile is isolated beneath the workspace and verified before recursive removal. It does not edit real user identities or pairings.

See [UI evidence](test-results/2026-10-23-desktop-ui.md). Interactive file-dialog/fingerprint verification, real user-triggered input receipt through the new shell, clean-PC setup, physical two-PC LAN/tunnel and gaming latency remain separate acceptance work. A green Local connection check is a loopback diagnostic; its roundtrip number cannot establish input-to-photon performance or an advantage over Sunshine/Moonlight.
