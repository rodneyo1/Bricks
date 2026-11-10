# Language and expected tooling

## Language decision

Use Rust, edition 2024, for the input and transport implementation. It fits low-level OS integration and provides memory safety around a small reviewed unsafe/FFI boundary. It avoids a garbage-collected runtime but does not prevent allocator delays, page faults, OS scheduling stalls, driver delays, or GPU synchronization. No language guarantees sub-millisecond tails on a general-purpose OS.

Rust 2024 is an edition, not a toolchain pin; it was introduced with Rust 1.85. See the [edition guide](https://doc.rust-lang.org/edition-guide/rust-2024/index.html). The foundation is built and tested with Rust 1.99.1, pinned in `rust-toolchain.toml`, with committed application `Cargo.lock`. Do not claim a reproducible build from the word `stable` alone.

Use Python for offline analysis. C/C++ SDK glue is acceptable where GPU APIs need it, with a documented ABI boundary. No Python in the input hot path.

The existing [development diagnostic launcher](desktop-ui.md) uses a small C# WinForms controller on .NET Framework 4.8. It runs the existing Rust diagnostics and verified recording-only connection commands; it does not process per-event input. The installed Framework compiler builds that development tool without adding Rust dependencies.

Slint 1.18.1 is pinned for the [product frontend](slint-desktop.md), with winit, software rendering, accessibility and compat-1-18; Qt, GPU renderers and application system-tray support are not enabled. The event-driven app has no application animations and uses the implemented named-pipe SDK to a separate per-user Rust agent. Input/media stay outside that control channel. See [agent implementation](single-pc-agent-and-recovery.md), [frontend design](frontend-design.md), [SDK contract](control-sdk.md), [Slint Rust API](https://docs.slint.dev/latest/docs/rust/slint/) and bundled [desktop license](../ui/desktop/Slint-Royalty-Free-2.0.md). About includes the required AboutSlint widget. Full dependency notice/security review remains a public-release gate. Development/test debug information and incremental artifacts are disabled to limit disk use.

## Dependencies to evaluate and lock

The foundation subset is installed and locked: `windows-sys`, `serde`, `toml`, `serde_json`, `thiserror`, and `sha2`. Identity provisioning adds `rustls`, `rcgen`, and `zeroize`. The live recording-only probe adds Quinn with ring crypto and Tokio. Other entries below remain candidates for later milestones, not installed features or an audited dependency set. Continue reviewing maintenance, licenses, features, and transitive dependencies when adding them.

| Area | Candidate | Decision |
| --- | --- | --- |
| Windows APIs | `windows-sys` initially; `windows` where useful later | Raw checked Win32 FFI for the initial adapter; enable only required features |
| Linux input | `evdev` | Evaluate capture and its [uinput module](https://docs.rs/evdev/latest/evdev/uinput/index.html); no duplicate wrapper by default |
| Network/session | `quinn`, `rustls`, `tokio` | Datagrams + control streams; dedicated input worker where measurements justify it |
| Wire codec | Small explicit codec | Fixed byte order, bounded records, versioned golden vectors; no native struct casting |
| Configuration | `serde`, `toml` | Typed validated config outside hot path |
| Errors | `thiserror`; optionally `anyhow` in binaries | Explicit protocol/platform errors; readable startup errors |
| Timing | `std::time::Instant`, OS adapters | QPC on Windows where direct timestamps are needed; no TSC shortcut by default |
| Metrics | `hdrhistogram` | Preallocated histograms; prevent histogram update stalls |
| Diagnostics | `tracing`, bounded trace sink, `serde_json` offline | No per-event JSON formatting or synchronous disk writes in hot path |
| Test generation | `proptest`, `cargo-fuzz` | State/codec invariants and malformed-input parsing |
| Microbenchmarks | `criterion` | Codec/queue/clock costs; not proof of game or photon latency |
| Offline analysis | Python `numpy`, `scipy`, `matplotlib` | Lock environment; paired errors, bootstrap confidence intervals, plots |

Do not add KCP/laminar on top of QUIC, a custom crypto stack, a fitted log-normal model, Prometheus, or a generic mocking framework without a demonstrated need. Use a deterministic fake clock and in-memory transport for core logic. Consider Loom only if custom concurrency primitives make it useful. [Microsoft's timing guidance](https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps) supports QPC for local intervals; measurement resolution still requires calibration.

## Development environment

Windows: supported Windows installation, Rust MSVC target, Visual Studio C++ Build Tools and Windows SDK, GPU driver/SDK only when media work begins. Use Windows Performance Recorder/Analyzer for CPU scheduling and queue analysis; GPUView/PresentMon can support frame investigations. Their software statistics supplement physical measurements.

Linux: Rust toolchain, build tools, evdev/uinput permissions, `iproute2` including `tc netem`, packet capture tools. Do impairment on a controlled bridge or router and shape directions separately. A Windows impairment utility is optional; document its actual configuration and verify the observed impairment.

Network: `iperf3` for available bandwidth under load, Wireshark or equivalent for packet timing and sizes, WireGuard/Tailscale diagnostics for route type. Bandwidth or ping alone is not an input-latency benchmark.

Physical latency: a microcontroller-controlled HID stimulus with an electrical reference plus photodiode/logger, or a calibrated high-speed camera that sees both stimulus indicator and display response. Record sampling resolution, trigger position, display scanout location, and measurement uncertainty.

## Build and checks

`bricks-agent` and `bricks-lab` use optimized release builds for resource/transport characterization. Bricks build scripts use MSVC `/DEBUG:NONE` because Rust's no-debug-info profile alone still allowed large PDB outputs on this machine. This follows [Microsoft's linker documentation](https://learn.microsoft.com/en-us/cpp/build/reference/debug-generate-debug-info). Dependency symbols can still be generated; normal source/build cache remains regenerable. See the [single-PC runner](single-pc-agent-and-recovery.md) for executable hashes, measurement scope and remaining hardware gates.

The portable frontend uses the separate `desktop` size profile; the agent uses release optimization with full LTO and retains unwinding. Control/config/JSON parsing are size-oriented; critical packet/core/QUIC code retains speed optimization. Benchmark builds preserve exact binaries. The lab alone makes a scoped [system-required idle-sleep request](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate); it restores the previous thread state and respects manual sleep/lid actions. Modern Standby/power policy can still interrupt a run. Actor pauses over one second mark new reports interrupted; older over-budget runs are explicitly identified by the analyzer. Neither their diluted CPU averages nor their successful-event latency tails qualify for acceptance.

`-HighQoS` is an explicit process-local benchmark experiment, separate from `-GamesScheduling`. It uses the documented [Windows QoS policy](https://learn.microsoft.com/en-us/windows/win32/procthread/quality-of-service) and restores the prior process state. It is not a power-plan change or a default production policy. Keep AC/battery, profile and background-load metadata with results; short comparisons do not establish an improvement or an endpoint CPU pass.

The CLI lab separately requests that Windows honor its active timer-resolution request even when the console is not visible, following [SetProcessInformation's timer-resolution policy](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation). Support is recorded, and the previous process policy is restored. Older Windows versions fall back explicitly if this bit is unsupported. It still is not a wakeup or QPC-accuracy guarantee. A coarse-generator run (observed around 15 ms lateness p99) cannot establish a uniformly paced 1 kHz or CPU improvement; keep pacing statistics beside delivery timing.

Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and workspace tests on supported targets. Unit/property tests must cover missing/duplicate/reordered transitions, cumulative movement, epoch reset, repeat policy, timer expiry, queue overflow, and release cleanup. Fuzz parsers before opening remote listeners. Use platform integration tests in an interactive session; hosted CI alone cannot certify game acceptance or GPU/display latency.

Record compiler, lockfile hash, source revision, OS build, power settings, GPU/driver, capture device polling rate, and all runtime options in benchmark manifests. Package portable user-session binaries first. Add installers, service supervisors, signing, and broader CI when deployment needs them. Review licenses of any reused Sunshine/Moonlight code or linked codec components before distribution.
