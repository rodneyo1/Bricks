# Development status and first commands

Windows-to-Windows is the first target. M0 foundation and M1 identity/ownership libraries are implemented. Recording-only QUIC now has both the original probe and a streaming recovery adapter. The separate user agent and named-pipe SDK serve Slint. Real Windows capture/injection networking, full interactive acceptance and two-PC measurements remain pending; this is not yet a usable remote-control application.

Run `.\target\debug\bricks-host.exe recovery-selftest` for 12 virtual-time recovery scenarios. The [agent/network-recovery milestone](single-pc-agent-and-recovery.md) additionally provides actual encrypted-loopback faults, lifecycle/crash checks and sustained release-build characterization. Full M3 acceptance still needs real OS release supervision and two-PC work.

The [2026-10-23 validation and benchmark report](test-results/2026-10-23-agent-recovery.md) records completed runs and unresolved performance targets; a successful synthetic suite is not full M3 acceptance.

See [2026-10-24 performance fixes](test-results/2026-10-24-performance-fixes.md) for the subsequent gap-age correction, timer/feedback changes, optimized portable package and remaining resource budgets.

## What exists

| Crate | Implemented now |
| --- | --- |
| `bricks-config` | Strict TOML schema v1, role-specific validation, one selected profile, effective JSON, SHA-256 fingerprint; shared config-inspection CLI |
| `bricks-protocol` | Explicit 48-byte little-endian experimental key-transition codec, golden vector, malformed-length/flags tests; not the final packet format |
| `bricks-core` | Epoch isolation, ordered key state, owned-key cleanup; separate transport-independent bounded sender/reorder/retry/ACK recovery module |
| `bricks-input-windows` | Foreground Raw Input/legacy receipt test window, explicit supported scan-code/HID mappings, core injector adapter, checked SendInput, aggregate mouse capture; automated tests never inject OS input |
| `bricks-transport` | UDP reference, DPAPI identities/trust, mutual TLS and exclusive ownership, live QUIC recording probe, timeout/revocation cleanup; in-memory and real loopback tests |
| `bricks-client` / `bricks-host` | Configuration and identity commands, `probe-host`/`probe-client`, one-command `probe-selftest`, and host `recovery-selftest`; synthetic data only |
| `bricks-bench` | Exploratory JSON report for codec/UDP echo round-trip, explicit boundary/class/limitations, optional application-config snapshot |
| `bricks-control` | Validated desktop persistence, restricted local IPC SDK, typed/versioned bounded frames, atomic snapshots, events, cancellation and diagnostic adapters |
| `bricks-agent` | Separate per-user headless runtime, state/operation authority, idle shutdown and unclean-restart detection; no live OS input |
| `bricks-lab` | Optimized QUIC recovery/fault/stress runner, recorded lifecycle cases, actual agent crash/reconciliation test and process-resource sampling |
| `bricks-ui` | Runnable Slint product frontend: saved-PC CRUD, settings, explicit identity setup, event feedback, Advanced checks/export and About; live controls disabled |

Build/package with `tools/build-desktop.ps1`; run `tools/test-desktop.ps1` for controller/SDK tests and isolated rendered UI smoke. `tools/run-single-pc-bench.ps1` runs the sustained recording-only suite. See [desktop instructions](slint-desktop.md) and [agent/network limits](single-pc-agent-and-recovery.md).

No live OS input forwarding, mouse injection/forwarding, external Windows watchdog supervisor, discovery, installer or video is claimed. The legacy probe still uses stop-and-wait control-stream ACKs and stops on gaps; the new recovery harness streams real QUIC datagrams with cumulative datagram ACKs, bounded history/reordering and retries. Two-PC operation remains unmeasured. Send history and owned keys are preallocated; receive reordering and Quinn packet ownership still allocate. These optimizations do not establish zero-allocation transport.

E1/Pause, consumer/media keys, and unsupported scan codes fail explicitly rather than being silently remapped. The mapping table covers a defined subset of keyboard-page usages, not every keyboard/layout. The fixture's F8 gesture explicitly submits one test A down/up pair. No automated test captures real typing or injects keys into this desktop.

## Interactive Windows fixture

```powershell
cargo run --locked --offline -p bricks-input-windows --example fixture
```

The window has a **Run test (F8)** button and visible status, submission/receipt counters, and last receipt time. Click the button or press F8 (Fn+F8 on some laptops) for a synthetic A down/up test. SUCCESS means a marked keydown was received; blocked/pending/failure messages are shown separately. Type/move/click/scroll while focused to exercise capture, then press Escape or close the window to print JSON. No background input sink is registered; no typed text or per-key trace is saved. Input counters include relative motion totals and vertical wheel units; absolute mouse records are counted separately.

Fixture report schema v2 names the total `explicit_test_submissions` because both button and F8 can trigger tests; earlier v1 reports used `explicit_f8_test_submissions`. The first visual-feedback version could redraw status before keyboard receipt. Following the [46-sample third run](test-results/2026-10-23-interactive-fixture-run3.md), `deferred-feedback-v1` defers formatting/redraw until after receipt or a 250 ms pending fallback. Compare like-for-like fixture revisions and trigger methods; controlled attribution of a UI-related speedup remains pending.

The [49-sample fourth run](test-results/2026-10-23-interactive-fixture-run4.md) measured median I0->A0 at 2.963 ms and observed p95 at 5.508 ms, versus 6.715/10.551 ms previously. Mean post-return delay fell from 4.382 to 1.240 ms; mean OS-call duration was similar. This is encouraging observational evidence, not a controlled causal comparison or acceptance pass. Four unmapped records remain unresolved.

The [fifth run with unmapped diagnostics enabled](test-results/2026-10-23-interactive-fixture-run5.md) processed 255 raw keyboard records with zero unsupported records or diagnostic drops. Earlier unmapped events were not reproduced, not diagnosed or fixed. All 32 test submissions produced receipts without reported failures. Local diagnostic median/p95 were 3.273/4.628 ms. The historical issue stays tracked while work proceeds to supported-input network integration; repeated manual timing runs are not needed to investigate an absent scan code.

F8 operates only when the fixture is foreground, A is not already held, and no previous test is waiting for receipt. It submits the pair in one OS call, checks the insertion count, and attempts release on partial failure and cleanup. Like all SendInput-based tests, the foreground check and OS submission are not atomic with respect to a focus change. Keep the window focused during the test. The fixture is diagnostic only and is not suitable for exclusive game capture.

The report labels its boundary as **I0 before SendInput to A0 receiving marked WM_KEYDOWN**; it also records the last call's API-return duration. These values are not capture-to-submit or physical latency. Inspect submission/cleanup failures, unresolved key state, read errors, dropped samples, and pending receipt before interpreting timings. Latency storage is bounded to 1,024 samples. Synthetic receipt markers correlate the local test; they are not authentication credentials.

The [second user-run report](test-results/2026-10-23-interactive-fixture-run2.md) observed zero unsupported keyboard records and vertical wheel activity. Its receipt samples were 5.6933 ms and 0.9399 ms, without enough instrumentation to explain the slower sample. The updated fixture adds `receipt_timing_samples`: paired submission-to-API-return, submission-to-receipt, and API-return-to-receipt durations with submission IDs. A null breakdown means a boundary was unavailable or inconsistent; never turn it into a zero delay. These are diagnostic samples, not game or two-PC acceptance results.

Manual acceptance matrix: ordinary keys, left/right modifiers, extended keys, held/repeated keys, rollover, mouse movement/buttons/wheel, focus loss, Escape/close, and F8 receipt. Repeat under CPU/GPU load. A [first user-run fixture report](test-results/2026-10-23-interactive-fixture.md) confirms two marked F8 receipts and keyboard/mouse capture, with four unsupported keyboard records still unexplained. The full acceptance matrix remains open. The updated report includes aggregate `unsupported_keyboard_reasons` without key identities or typed text. Buffered high-rate input draining, device-aware repeat ownership in the live capture path, and complete mouse forwarding remain work before accepting M1/M2 input performance.

## Authentication boundary

`bricks-transport::auth` builds TLS 1.3 configs from explicit certificate/key material and separately verified peer trust. The server requires a client certificate; the client validates the host chain/name. No public root set, anonymous fallback, disabled verifier, or early input data is enabled. Successful TLS alone does not authorize input ownership.

Tests generate temporary identities, exercise DPAPI-protected persistence, and reject wrong host/client trust, wrong hostname, absent credentials, missing control permission, and missing host approval. Live QUIC loopback tests also cover datagram application, disconnect, malformed/stale packets, idle revocation, and silence cleanup of recorded state. See [identity and ownership](identity-and-ownership.md) and [QUIC probe](quic-probe.md) for commands and remaining OS-input integration work. These tests do not establish a production security review or two-PC connectivity.

## First live network diagnostic

```powershell
.\target\debug\bricks-host.exe probe-selftest 100
```

This runs actual mutually authenticated QUIC loopback endpoints with 200 synthetic transitions and no OS injection. The initial compiled run completed all 200 transitions without duplicates or cleanup failures. Its exploratory roundtrip p95 was 0.8075 ms; it measures neither the Windows fixture boundary nor physical/network input latency. See [probe instructions](quic-probe.md) for the separate two-PC setup and all limitations.

## Toolchain

Rust `1.99.1`, edition 2024, pinned in [rust-toolchain.toml](../rust-toolchain.toml), with committed [Cargo.lock](../Cargo.lock). Windows builds require the MSVC C++ tools and Windows SDK. The current development machine already had these; a minimal user-local Rust toolchain was installed without changing persistent PATH. If `cargo` is absent from your shell PATH, use `%USERPROFILE%\.cargo\bin\cargo.exe` or the check script's fallback.

## Build and verify

From the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run --locked -p bricks-client -- config docs/examples/client.toml
cargo run --locked -p bricks-host -- config docs/examples/host.toml
```

Or run `powershell -ExecutionPolicy Bypass -File tools/check.ps1` for the same checks using a user-local Cargo fallback. This process-scoped option does not change machine execution policy. The script stops on the first failed command.

## Exploratory loopback reference

```powershell
cargo run --release --locked -p bricks-bench -- loopback-reference --samples 10000 --config docs/examples/client.toml
```

The JSON measures from before client encode/send to after client validates the echo. It includes both directions and same-thread echo work. There is no input capture, OS injection, game receipt, QUIC comparison, or external display measurement. Do not compare its percentiles to the LAN input gates. Clock calibration and confidence intervals are explicitly unknown; it is an exploratory reference, not an acceptance report. Application settings are recorded for context but do not configure the isolated reference transport.

A malformed response, timeout, invalid configuration, or failed exchange exits nonzero without emitting a successful report. For real M2/M4 benchmarking, add full hardware/route inventory, clock calibration, repeated runs/confidence intervals, equal authentication/recovery, and source/build artifact identity. An absent source revision or unknown uncertainty remains explicit rather than invented.

## Next work

Run the recording probe between the two PCs, then connect bounded live capture/injection queues and the OS backend with real event timestamps, device state/repeat ownership, and application recovery. Keep the non-reproduced unmapped-input issue tracked. Two-PC results and physical latency need the second PC/instrumentation; loopback data cannot satisfy those gates.

The second-PC check is deferred until hardware is available. In the meantime, use `recovery-selftest` for the implemented key-recovery engine, then integrate it with authenticated loopback and a foreground fixture. Keep live OS injection, high-rate input, mouse barriers, snapshots/resync and crash supervision as separate, observable acceptance tasks.

The [Windows desktop preview](desktop-ui.md) now provides a portable double-click launcher, visible checks, input-fixture launch, public-identity verification and recording-only host/client connection tests. C# WinForms presents the UI; the existing Rust tools execute all diagnostics/session commands. Build with `tools/build-ui.ps1`, then open **Start Bricks** or `dist\Bricks\Bricks.exe`. This is partial M5 interface work, not a signed installer or live remote-control app. Controller/GUI verification and remaining manual gates are recorded in [UI evidence](test-results/2026-10-23-desktop-ui.md).

Product direction clarified: the WinForms app above remains a development diagnostic launcher. The user selected Slint with plain functional screens, no animations, and clear event feedback. The next design step is recorded in [product frontend design](frontend-design.md) and [local control SDK](control-sdk.md): three screens plus pairing/approval dialogs, a persistent per-user Rust agent, shared UI/CLI commands, capability-driven controls and confirmed session/cleanup events. No Slint frontend, control pipe/server or new agent crate has been implemented by this documentation change.

### Targeted unmapped-key diagnostic

```powershell
.\target\debug\examples\fixture.exe --diagnose-unmapped
```

The window explicitly indicates diagnostics are on. Press the keys used in previous runs, including Fn/function keys if applicable, then close it. F8 repetitions are not needed for this check. Inspect `unsupported_key_codes`: aggregate make-code, raw-flags, virtual-key identifiers, and counts for unsupported inputs only. No event sequence, key timestamps, or typed text is recorded. Collection is off by default, capped at 64 distinct tuples, and reports overflow separately. These identifiers may reveal which unsupported keys were used; enable this only for deliberate test inputs.

## Verification recorded on 2026-10-23

M0 recorded 16 passing tests. M1 expanded this to 21 passing tests, including mutual TLS and supported key mappings. After the user's first interactive run, an additional diagnostic-classification test passes, bringing the suite to 22 tests (the six input-adapter tests were rerun after that change). The fixture builds, with limited interactive evidence recorded above. Formatting and Clippy with warnings denied pass. Both example configurations pass the actual CLI; the wrong-role configuration and unimplemented live-start command are rejected. Local documentation links and example TOML syntax were checked.

The latest implementation passes **29 tests** across the complete workspace with normal Windows profile access, including opt-in diagnostic privacy/bounds, DPAPI persistence, verified trust, exclusive admission, and idle revocation cleanup. The restricted execution profile could not access DPAPI; these tests were rerun successfully with the normal profile rather than skipped. The fixture and both development binaries build; formatting and Clippy checks pass. No actual per-user Bricks identity was initialized outside temporary test stores.

After the live QUIC probe landed, the complete suite passes **33 tests**, adding real-loopback success, denied permission/approval, an untrusted client, and a fault matrix covering disconnect/malformed/stale/revoked/silent sessions. The compiled `probe-selftest 100` also completed 200 acknowledged transitions. Final workspace build, formatting, and Clippy checks pass. These are recording-only network checks, not OS input or two-PC acceptance.

The release UDP reference completed 10,000 measured request/echo exchanges after 1,000 warm-up exchanges, with zero identity mismatches. Its report is saved locally at `artifacts/loopback-reference.json` (generated artifacts are git-ignored). It remains uncalibrated exploratory software data, not proof of any LAN input target or gaming advantage.

The transport-independent key-recovery addition brings the complete suite to **48 passing tests** with normal Windows profile access. Formatting, Clippy with warnings denied, and workspace/all-target builds pass. The compiled `recovery-selftest` passes 12/12 scenarios with embedded defaults and explicit client configuration; nondefault queue/timer limits also pass in the unit suite. CLI trailing arguments are rejected. See [recovery evidence](test-results/2026-10-23-recovery-selftest.md). No live-network recovery or OS-release result is inferred from these simulations.
