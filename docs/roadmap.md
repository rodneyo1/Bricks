# Implementation roadmap

Amended baseline: 2026-10-23, following the consolidated review. Immediate forwarding is the timing policy; Raw Input is the default; QUIC datagrams are the authenticated baseline; a small UDP reference is measured early. Performance work precedes installer polish. See [development status](development.md) for what actually exists.

| Milestone | Deliverable | Exit criterion |
| --- | --- | --- |
| M0: Workspace/configuration | Rust workspace, typed versioned TOML profiles, explicit module contracts, two development CLIs | Reproducible build, config validation and foundation tests pass; no live control claimed |
| M1: Local correctness/authentication | Raw Input message pump, checked SendInput integration, physical-key mapping, receiver fixture, authenticated session setup | Real Windows capture/receipt tests and local gates pass; no live unauthenticated ownership grant |
| M2: Two-PC input | Authenticated QUIC datagrams, explicit host addresses, bounded queues, clock model, early UDP reference | Actual two-PC LAN baseline, exact boundaries and uncertainty reported; same IP path works over existing tunnels |
| M3: Failure behavior/timing | ACK/redundancy, motion barriers, snapshots, watchdog, reconnect epochs; implement immediate-forwarding policy | Loss/reorder/crash/overflow tests; cleanup externally observed; no stale-epoch input; timing error characterized |
| M4: Transport bake-off | Equal-security, equal-delivery/recovery QUIC versus protected UDP experiment | Measured decision on latency, CPU, bandwidth, recovery; changing the default is explicitly allowed |
| M5: Simple install/connect | Share/Connect shell, signed-installer plan, discovery, convenient reviewed pairing, saved hosts | First-user setup test, clean uninstall; no mandatory terminal/TOML for normal use |
| M6: Hybrid physical proof | Baseline input versus Bricks input with the same Sunshine/Moonlight video path | Physical instrument and single input owner verified; publish reproducible results and decide whether media work is justified |
| M7+: Bricks media/expansion | Selected hardware capture/encode/decode/presentation/audio path, then other platforms/backends | Full-stack comparison at matched quality; each added capability has its own profile and compatibility evidence |

## Explicit gates

Do not start media implementation before M1-M6 pass their correctness, two-PC latency, recovery, measurement-validity, and usability gates. A valid hybrid experiment need not show a victory, but its result must justify the cost of proceeding. If it finds no useful opportunity, record a stop/narrow-scope decision instead of assuming video is inevitable. Do not call the input agent a complete Sunshine/Moonlight replacement.

Independent work may proceed out of milestone order while hardware is unavailable. The [single-PC recovery engine/check](recovery-selftest.md) is an implemented M3 foundation: bounded key history, retries, cumulative applied ACKs, reorder/deduplication and recorded cleanup. It does not complete M2 or M3. Live integration, mouse barriers, snapshots/resync, real Windows release, external crash protection and two-PC/physical gates remain pending.

An initial [Windows desktop shell](desktop-ui.md) also proceeds independently: portable launch, visible checks, fixture launcher, manual verified-identity setup, and real recording-only connection-test actions. This implements part of M5's interface work while preserving performance gates. Installer/signing, PAKE pairing, discovery, live capture/control and first-user usability acceptance remain pending; no M5 completion is claimed.

The existing WinForms shell is a development diagnostic tool. The [Slint frontend](slint-desktop.md) now uses a separate per-user agent and restricted local IPC SDK for device/settings/setup/diagnostics. [Network recovery and single-PC fault/lifecycle/stress work](single-pc-agent-and-recovery.md) also proceed independently. These implement parts of M2/M3/M5, without passing the real OS cleanup, two-PC, installer/usability or media gates.

Authentication is implemented with M1 and required before M2 live sessions. Deferred pairing screens never imply deferred authentication. Use pinned peer identities during development rather than a plaintext reusable token. The initial unauthenticated UDP echo is loopback-only, cannot inject input, and is a reference harness, not a remotely deployable backend. The M4 comparison must add equivalent peer authentication, replay protection, traffic protection, and application semantics before interpreting latency differences as a transport decision.

The timing decision is already made in [timing policy](timing-policy.md), not left open until M3: inject immediately when dependencies are satisfied; measure residual differential jitter and recovery delay. No sampled key releases. A fixed delay after arrival does not correct differential jitter.

Every result carries a class (local, loopback, two-PC LAN, tunnel, hybrid physical, full-stack physical, or clearly marked software reference) and exact start/end markers. Unknown measurement uncertainty makes small claimed improvements inconclusive. Scheduler jitter stays in the latency distribution.

## Open choices and risks

| Item | Current position | Resolution |
| --- | --- | --- |
| Platform | Windows-to-Windows confirmed | Input first; Linux/macOS deferred |
| Host/client/GPU/display hardware | Not supplied | Inventory before hardware acceptance runs and media backend choice |
| Raw Input/SendInput compatibility | Not demonstrated | M1 fixture plus actual target games; E1/Pause, multimedia keys, and layouts need explicit mapping |
| Authentication implementation | Required in M1 | Review maintained TLS/QUIC identity APIs; explicit development trust before friendly pairing |
| Pairing UX implementation | PAKE flow planned | Select/review library and identity binding before M5 |
| UI toolkit/installer/signing | Slint selected for the product UI; installer/signing pending | Implement the simple frontend/control contract; keep existing WinForms tool for diagnostics |
| Recovery defaults | Provisional | Tune from fault traces without hiding lost history |
| Crash cleanup | In-process cleanup may be insufficient | Evaluate supervisor against externally observed release bound |
| Transport default | QUIC initially | Real M4 result can switch to protected UDP |
| Internet convenience | Existing IP tunnel first | No custom NAT traversal or managed relay in v1 |
| Gaming advantage | No evidence yet | Physical hybrid measurement and later full-stack comparison |

Each backend addition needs typed configuration, declared capabilities, adapter contract tests, and a benchmark manifest. Keep useful defaults without accumulating untested profiles.
