# Agent, recovery and performance results - 2026-10-23

The four single-PC work items are implemented: a separate headless agent and Slint SDK integration, recovery over authenticated QUIC, lifecycle/crash feedback, and sustained fault/resource benchmarks. The 150 ms recovery profile completed 35 network cases with 1,300,000 scheduled, generated, applied and acknowledged transitions. The stricter 30 ms profile failed to complete 24 of its 35 cases. Active CPU, bandwidth and desktop package size need further work. No real capture-to-submit, mouse, two-PC or gaming performance gate has passed.

The [targets and benchmark rules](../targets-and-benchmarks.md) retain their numerical values. The [implementation document](../single-pc-agent-and-recovery.md) explains runtime boundaries, IPC limits, wire behavior and reproduction.

## Delivered behavior

1. The portable Slint app starts/connects to a separate per-user `bricks-agent.exe`. Versioned, bounded, account/session-checked named pipes carry settings, saved-PC mutations, snapshots, diagnostic operations, cancellation and completion events. The UI reports disconnection/reconnection, refreshes authoritative state and does not replay an uncertain mutation. Editor revisions reject stale saves.
2. Recovery uses encrypted QUIC datagrams with authenticated admission, exclusive ownership, fresh session epochs, cumulative application ACKs, bounded history/reordering and retransmission. A reliable control-stream grant communicates the epoch and policy. Heartbeats preserve long holds while exposing missing history. Healthy arrivals apply immediately; only ACK feedback is batched.
3. Tests exercise dropped final releases, loss/reordering/duplication, silence, revocation, reconnect/replay, competing ownership, task abort, visible cleanup failure/retry and a ten-second hold. A separate test kills its own recording-only agent process and verifies disconnect, restart, persisted state, a fresh instance, confirmed cancellation and `unknown_local` cleanup status.
4. The release lab records complete counts, tail latency, pacing lateness, queue peaks, CPU/private memory, estimated bandwidth and effective configuration hashes. Scripts preserve failed runs and exact executed binaries. The product UI exposes the network recovery check, cancellation and report export.

The hot path preallocates sender history and the owned-key set, avoids per-key tree-node allocation, applies arrivals without a jitter buffer and shares one network worker in this laboratory. Storage, UI and JSON export are outside receive/apply. Quinn packets, impaired-path reordering and measurement buffers still allocate. This is not a zero-allocation or production OS-input claim.

## Machine and measurement scope

| Item | Recorded setup |
| --- | --- |
| Windows | NT 10.0.26200.0, x64 |
| CPU | Intel Core i5-8365U, 4 cores / 8 logical processors |
| GPU / driver | Intel UHD Graphics 620 / 31.0.101.2135; no media workload |
| Power / background | Balanced; ordinary interactive development desktop |
| Toolchain | Rust 1.99.1, `x86_64-pc-windows-msvc`, LLVM 23.1.1 |
| Network / backend | IPv4 loopback, both endpoints and synthetic harness in one process; recording injector |
| Optimized build | Thin LTO, one codegen unit, stripped debug information, MSVC `/DEBUG:NONE` |
| Timing | Same-clock uncalibrated `Instant`; generated transition to recording submission |
| Impairment | Seeded application datagram impairment in both directions before encrypted QUIC send |

No physical NIC, IP tunnel, OS capture/injection, game, encoder, decoder or display is included. The actor paces synthetic critical keyboard transitions at nominal 1/4/8 kHz; these rates do not establish physical mouse polling or motion fidelity. Generation-to-recording starts **after** the generator wakes: it excludes its reported pacing lateness. Windows timer requests and nanosecond report units do not establish calibrated timing accuracy.

The suite performs 60 seconds of prior traffic on a separate connection, five healthy 10,000-transition blocks, five seeded 20,000-transition blocks at each loss level, three 60-second rate cases, eleven additional fault cases, fourteen lifecycle cases, agent SDK crash checks and a 60-second idle-agent sample. Per-condition warm-up, controlled thermal/load conditions, randomized A/B comparison and hardware acceptance remain pending.

## Preserved runs and recovery policy

| Run | Selected history cutoff | Complete network cases | Lifecycle | Agent crash checks |
| --- | --- | --- | --- | --- |
| `artifacts/single-pc-20261023-185559` | 30 ms | 11/35; 24 stopped | 14/14 | 9/9 |
| `artifacts/single-pc-20261023-190516` | 150 ms | 35/35 | 14/14 | 9/9 |

The cutoff is the bounded time allowed to recover missing critical history, not an intentional delay before applying healthy input. Both values are within the documented 30-150 ms policy range. The follow-up explicitly selects 150 ms; it does not demonstrate that the 30 ms profile works. The CLI lab default remains 30 ms; the desktop-characterization runner selects 150 ms and exposes `-StrictRecovery` for 30 ms. Live RTT-driven policy selection remains pending.

The 30 ms run stopped even in healthy cases. At 1 kHz it applied 37,037 of 60,000 scheduled transitions before stopping, leaving 22,963 ungenerated; at 8 kHz it generated 112,729 of 480,000, with 40 generated transitions missing. Its 4 kHz case completed 240,000 transitions. Successful-event timing percentiles from stopped cases cannot certify acceptance. Fixed initial RTT assumptions, timer/pacer scheduling, instrumentation and background load need profiling; the evidence does not isolate a single cause of the stalls.

## Completed 150 ms rate characterization

Every row below ran for approximately 60.004 seconds and had zero missing generated transitions, ungenerated scheduled transitions, critical application fidelity errors, cleanup failures and owned keys after cleanup. Each applied transition had a recorded latency sample.

| Nominal rate | Applied / scheduled | Achieved rate | p50 | p95 | p99 | Generator lateness p99 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 kHz | 60,000 / 60,000 | 999.94 Hz | 0.2582 ms | 0.5112 ms | 0.7325 ms | 2.7990 ms |
| 4 kHz | 240,000 / 240,000 | 3,999.73 Hz | 0.2361 ms | 0.4625 ms | 0.6378 ms | 2.7164 ms |
| 8 kHz | 480,000 / 480,000 | 7,999.51 Hz | 0.2414 ms | 0.4686 ms | 0.6482 ms | 2.5246 ms |

The five healthy 10,000-transition blocks also completed. Their p95 values ranged from 0.3330 to 0.5219 ms and p99 from 0.5890 to 0.7620 ms. These are synthetic generation-to-recording results; they cannot pass the C0-to-I0 or paired-overhead targets merely because their numbers are small. Do not add unrelated handler percentiles to obtain an overhead percentile.

| Nominal rate | Combined CPU average, one core | CPU p95 sample window | Total-machine average, 8 logical CPUs | Private peak | Estimated IPv4 traffic |
| --- | --- | --- | --- | --- | --- |
| 1 kHz | 30.779% | 37.492% | 3.847% | 5.238 MiB | 1.3943 Mbit/s |
| 4 kHz | 31.534% | 42.139% | 3.942% | 11.465 MiB | 2.9373 Mbit/s |
| 8 kHz | 32.290% | 40.615% | 4.036% | 20.918 MiB | 5.0165 Mbit/s |

CPU includes both endpoints, synthetic pacing and instrumentation. Each rate has sixty resource intervals; Windows CPU accounting is quantized and interval lengths vary. Neither endpoint has an isolated active CPU result. Report buffers contribute to private memory. The healthy 1 kHz run retried 9,255 transitions, which were deduplicated rather than applied twice: retransmission/feedback timing is a clear optimization candidate. Fixed RTT assumptions and bounded feedback timers require measurement before tuning. The occasional synchronous resource sample and other instrumentation also need separate cost characterization.

Bandwidth estimates use both endpoints' QUIC UDP counters plus IPv4/UDP headers and include handshake traffic. They omit Ethernet/tunnel overhead and do not replace actual packet capture or packet-rate reporting.

## Loss, faults and cleanup

Each loss condition scheduled 100,000 transitions over five independently seeded blocks. Scheduled counts include ungenerated work when a case stops.

| Application loss in each direction | 30 ms profile: delivered before deadline / scheduled | 150 ms profile: delivered before deadline / scheduled |
| --- | --- | --- |
| 0.5% | 86,404 / 100,000 (86.404%) | 100,000 / 100,000 (100%) |
| 1% | 87,161 / 100,000 (87.161%) | 100,000 / 100,000 (100%) |
| 2% | 72,974 / 100,000 (72.974%) | 100,000 / 100,000 (100%) |

All five block delivery fractions in each 150 ms condition were 1.0; the machine-readable analysis retains every fraction. This meets the empirical count goal in this synthetic experiment. It does not establish a 99.99% reliability guarantee: events are correlated, there are only five independent seeded blocks per condition, and a block bootstrap would be degenerate at [1,1]. A useful rare-failure confidence bound and real-network acceptance remain inconclusive; do not apply an independent-event binomial confidence claim.

The 150 ms run also completed lost-final-release, 0.1/1% duplication/reordering, 1/5/10 ms delay variation and 2/5/10 packet bursts. The 10 ms delay-variation case had p95 9.9545 ms and p99 11.3874 ms. Impaired timing is reported separately from the healthy profile. Deduplicated retransmissions are visible in raw counts; zero duplicate **applications** does not mean zero duplicate packets.

Recorded silence cleanup took 257.7821 ms, below the 250+50 ms goal for this recording fixture. The 250/1,000 ms outage cases reached recorded neutral in 261.6888/260.2697 ms. Disconnect took 0.1083 ms. The ten-second hold produced exactly one down and one up, with no repeat injection. A deliberately failed release stayed owned and visible until a retry succeeded. Approval/permission denial submitted no input and has no fabricated cleanup-latency sample.

These observations concern a recording injector. Host-task abort uses an external recording trace but is not process death. The actual agent-process crash test checks metadata and feedback only. An external Windows release supervisor, independently observed OS-neutral state and real injector-process termination remain required before live input capability is enabled. Lifecycle cases use their own declared policy; their outage times must not be attributed to the 150 ms stress profile.

## Target assessment

| Existing target | Evidence / assessment |
| --- | --- |
| Healthy critical fidelity | Zero missing/duplicate/out-of-order recorded applications in completed 150 ms cases; 30 ms completeness failed. Real input gate pending. |
| 1 kHz sustained work; 4/8 kHz stress | Three full synthetic rate cases completed. Mouse gate pending. |
| At least 99.99% delivered before recovery deadline under <=1% loss | 100,000/100,000 at each tested loss level with 150 ms; statistical and physical-network qualification pending. Failed with 30 ms. |
| Idle CPU <=0.5% of one logical core | Separate headless agent: 0.4378% over 60.668 seconds; earlier run 0.1802%. Within budget in these runs, with ordinary desktop variability. No UI or event trace included. |
| Active CPU <=2% of one logical core per endpoint | Unverified per endpoint; combined harness is approximately 31% at 1 kHz. An active CPU pass is not established. |
| Private memory <=50 MiB, 30 MiB stretch per endpoint | Combined instrumented process peaks 5.238/11.465/20.918 MiB; isolated steady-state endpoint qualification pending. Idle agent peak 1.191 MiB, delta -32 KiB. |
| Installed input binaries <=10 MiB, 5 MiB stretch | Agent alone 4,337,152 bytes (4.136 MiB). Current UI+agent executables total 33,452,032 bytes (31.902 MiB): exceeds the installed-binary goal. UI is a development build; optimized UI size remains to be measured. |
| Input bandwidth <=1 Mbit/s at 1 kHz | Estimate 1.3943 Mbit/s for this critical-transition workload: above budget before Ethernet/tunnels. Production mouse format and physical packet capture pending. |
| Silence cleanup <=300 ms | Recording fixture met it. Actual OS and supervisor acceptance pending. |
| C0-to-I0, hold error, LAN/tunnel, physical I2P and Sunshine/Moonlight comparison | Unmeasured. No superiority or gaming-readiness claim. |

## Functional validation and portable app

Workspace tests passed: 63 tests across the libraries; formatting, all-target builds and Clippy with warnings denied passed. After adding the selectable recovery cutoff, the 37 core/transport tests and Clippy passed again and all targets built. Tests include authenticated QUIC, ownership, malformed frames, snapshot overflow/reconciliation, stale edits and cancellation ownership. Windows DPAPI tests ran under the normal account with isolated stores.

The packaged desktop passed ten rendered workflow checks against its bundled optimized agent: add/edit, settings/default port, invalid feedback, recovery, export, forget, persisted final state and confirmed network cancellation. Actual Windows UI Automation also exercised named fields/buttons. Screenshots and JSON evidence are preserved. Cross-account/session negative tests, signed installation and broad accessibility/high-DPI validation remain pending.

The portable folder is `dist/BricksDesktop`; double-click **Start Bricks** in the workspace. Keep `Bricks.exe`, `bricks-agent.exe` and notices together when copying it. No Rust installation is needed to run the folder. The full folder is 33,458,067 bytes including instructions/licenses. Setup remains portable; it is not a signed installer. Live connection controls remain disabled until the input and safety capabilities exist.

The packaged UI and agent hashes match the validated build and preserved agent binary respectively. The Start Bricks shortcut target was verified, and the normal app opened with title Bricks and a separate bundled agent process. Launch evidence is `artifacts/agent-desktop-launch-result.json`. Three PowerShell scripts parsed successfully; relative links and code fences passed across the root README, top-level docs and this report (22 Markdown files).

Validation encountered and corrected a timestamp-only temporary-directory collision and MSVC symbol-file disk pressure. Unique test counters and workspace-owned `/DEBUG:NONE` build helpers address these without removing sources, saved PCs or benchmark evidence. Earlier unsuccessful runs remain available.

## Evidence and reproduction

Primary evidence roots are relative to the repository; generated artifacts are local and ignored by source control:

- `artifacts/single-pc-20261023-185559/`: failing 30 ms profile, per-case JSON, lifecycle, SDK crash, idle sample, environment, exact binaries and `analysis.json`.
- `artifacts/single-pc-20261023-190516/`: completed 150 ms profile with the same categories; 35 complete cases and 1,300,000 recorded transitions.
- `artifacts/packaged-desktop-72bad9b576a14a8c95302a43d22e24ff/`: packaged smoke JSON and rendered screenshots.
- `artifacts/desktop-native-42596b3156974111b7fbe1ceeafe93f2/`: native accessibility/persisted-field evidence.

Every network case carries effective lab options and `config_sha256`. `environment.json` records hardware/power and binary/lockfile hashes. Post-run `inputs/` snapshots are explicitly labelled in `input-provenance.json`; numeric targets did not change. The completed run additionally preserves `source-snapshot/`, copied after measurement from unchanged Rust source. There is no Git revision; the strict run's preserved executable, rather than the later source snapshot, identifies exactly what it executed.

Completed-run SHA-256 identifiers:

```text
Cargo.lock       293376D34C1AC84F2B21C655FE0533B7D7E226E22EC2DF9118B85F0F69D7DB83
bricks-agent.exe F8DC1FCD2BFBB946DB5301AC21277BF340A739E00B391577D0D3D792F28345CB
bricks-lab.exe   577DF9161FDC0FC4329C33706F0970C038397124B50A0BD6E72A785861FFBC2E
```

Run the full 150 ms characterization from the project folder:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\run-single-pc-bench.ps1
```

Add `-StrictRecovery` for 30 ms or `-Quick` for development screening. A quick run is not sustained acceptance. The analyzer can regenerate summaries using `tools/analyze-single-pc.ps1 -RunDirectory <artifact-folder>` through the same PowerShell invocation. The benchmark script now captures input snapshots/toolchain metadata before future runs.

Next performance work is to profile timer/pacer wakeups, separate per-endpoint processing from instrumentation, measure packet rate and eliminate unnecessary healthy retries using validated feedback/RTT policy. Follow with compact production mouse transport, optimized UI packaging, and finally real capture/injection plus external cleanup supervision. Physical two-PC tests and matched Sunshine/Moonlight measurements remain necessary before judging the project's gaming goal.
