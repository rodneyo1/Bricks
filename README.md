# Bricks

![Bricks desktop app](docs/images/bricks-desktop.png)

Low-latency remote input and, eventually, a complete game-streaming system. The ambition is to outperform Sunshine + Moonlight on measured input-to-photon latency at comparable visual quality and stability.

The Rust foundation includes validated configuration, a reference codec, deterministic key state, a Windows capture/receipt test window, key diagnostics, DPAPI-protected identities, verified trust/ownership, and a live recording-only QUIC probe. Live remote OS control is not implemented yet. No two-PC or physical gaming benchmark results exist.

A transport-independent key-recovery engine and a [one-command virtual-time fault check](docs/recovery-selftest.md) exercise retries, ordering, overflow, session isolation and recorded cleanup. The new [authenticated QUIC recovery harness](docs/single-pc-agent-and-recovery.md) also exercises that engine over real encrypted loopback traffic; the original stop-and-wait probe remains separate.

```powershell
.\target\debug\bricks-host.exe recovery-selftest
```

Start with the [documentation index](docs/README.md), [design review](docs/design-review.md), and [targets and benchmarks](docs/targets-and-benchmarks.md).

Desktop app: double-click **Start Bricks**, or open `dist\BricksDesktop\Bricks.exe`. The simple Slint frontend saves/edits/forgets PCs, persists settings, sets up protected local identities, and shows event feedback. Local checks and JSON report export live under Settings > Advanced. See [desktop instructions](docs/slint-desktop.md).

The toolkit-independent controller now runs in a separate per-user Rust agent, connected through a restricted named-pipe SDK. Critical-key recovery also runs over actual recording-only QUIC, with a [single-PC fault/lifecycle/performance suite](docs/single-pc-agent-and-recovery.md). **Live remote OS input, code pairing and video are not available yet**; their UI actions remain disabled. See [frontend design](docs/frontend-design.md) and [SDK contract](docs/control-sdk.md).

The previous WinForms [diagnostic launcher](docs/desktop-ui.md) remains separately available at `dist\Bricks\Bricks.exe` for foreground input tests and fingerprint-verified recording-only two-PC tests.

Build/test instructions and exact implementation limits are in [development status](docs/development.md).
