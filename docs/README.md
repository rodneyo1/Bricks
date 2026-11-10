# Bricks documentation

Design baseline: 2026-10-23. Status: proposed implementation baseline. Windows-to-Windows is confirmed as the first platform; hardware assumptions await confirmation.

| Document | Purpose |
| --- | --- |
| [Design review](design-review.md) | Evaluate the supplied decisions, correct errors, and identify missing requirements |
| [Product design](design.md) | Scope, behavior, operating modes, and expected results |
| [Architecture](architecture.md) | Components, threads, OS boundaries, and future media pipeline |
| [Product frontend design](frontend-design.md) | Simple Slint screens, pairing, capability gates and user feedback on events |
| [Slint desktop](slint-desktop.md) | Runnable product frontend, saved PCs/settings/setup, build, tests and explicit first-slice limits |
| [Agent and network recovery](single-pc-agent-and-recovery.md) | Implemented headless agent/IPC subset, actual QUIC recovery, lifecycle/fault/stress procedure and performance limits |
| [Agent/recovery results, 2026-10-23](test-results/2026-10-23-agent-recovery.md) | Completed single-PC runs, failed strict profile, resource gaps and pending hardware gates |
| [Performance fixes, 2026-10-24](test-results/2026-10-24-performance-fixes.md) | Gap-age correction, feedback/timer work, optimized package, repeated strict profile and remaining budgets |
| [Local control SDK](control-sdk.md) | Persistent Rust agent, UI/CLI contract, named pipe, state/events and cleanup semantics |
| [Protocol](protocol.md) | Delivery semantics, event identity, recovery, and session lifecycle |
| [Timing policy](timing-policy.md) | Immediate-forwarding decision and exact timestamp boundaries |
| [Tooling](tooling.md) | Language, dependencies, development tools, and verification |
| [Configuration](configuration.md) | Modular backends, versioned profiles, validation, and extension rules |
| [Identity and ownership](identity-and-ownership.md) | DPAPI credentials, verified peer trust, admission, and revocation |
| [QUIC probe](quic-probe.md) | Live authenticated recording-only loopback and two-PC diagnostic |
| [Single-PC recovery check](recovery-selftest.md) | One-command virtual-time fault suite; transport-independent key recovery |
| [Development diagnostic launcher](desktop-ui.md) | Existing WinForms checks and recording-only connection setup; separate from the product frontend |
| [Installation and connection](installation-and-connection.md) | Simple setup, pairing, discovery, reconnect, and packaging |
| [Targets and benchmarks](targets-and-benchmarks.md) | Provisional targets, measurement definitions, comparison rules, and regression gates |
| [Roadmap](roadmap.md) | Milestones, exit criteria, risks, and unresolved choices |
| [Development status](development.md) | What is implemented, how to build/test, and current limitations |
| [Sources](sources/README.md) | Original attachments and primary technical references |

The reviewed documents govern implementation. Files prefixed `original-` in `sources/` preserve the attachments verbatim and are historical inputs, not current requirements. All new numerical performance targets are engineering hypotheses until measured. Nothing here establishes that Bricks is faster than an existing system. The final external review is preserved as [consolidated feedback](sources/consolidated-feedback.txt); the timing policy and measurement definitions resolve its remaining ambiguities.

Confirmed initial platform: Windows host and Windows client. Provisional benchmark setup: wired LAN, keyboard/mouse, 1080p SDR at 120 FPS on a client display running at least 120 Hz. GPU vendor, device models, and exact OS versions remain unspecified. Linux input support follows the first Windows proof; macOS follows later. Normal installation and connection must work through the UI without editing configuration files.
