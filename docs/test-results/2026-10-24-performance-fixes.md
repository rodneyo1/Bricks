# Recovery and resource optimization, 2026-10-24

The strict 30 ms profile completed **35/35 network cases and 1,300,000 transitions**, after correcting premature gap expiry. All transitions were applied and acknowledged, with no missing history, fidelity errors or network-case cleanup failures. Estimated 1 kHz IPv4 traffic fell below 1 Mbit/s, and installed binaries shrank from 31.902 to 11.296 MiB. Active CPU did not improve in the final sustained comparison and the complete package still exceeds 10 MiB. This is recording-only encrypted loopback work, not a remote gaming acceptance result.

## Changes and their purpose

1. **Correct gap ages.** A single oldest-gap timestamp survived even after all history belonging to that gap was repaired. A later overlapping hole could therefore expire before it was 30 ms old. The receiver now retains increasing produced-history ranges with their first-noticed times, retires fully applied ranges, and uses the oldest remaining range. Partial repair, duplicates and heartbeats cannot renew genuinely old history. The preallocated range queue is bounded by the receive window plus one; healthy in-order delivery does not enqueue ranges. The deadline remains 30 ms.
2. **Reduce unnecessary feedback and retries.** The lab coalesces cumulative application feedback after eight events or at most 5 ms, and requests corresponding QUIC ACK frequency. Input still applies immediately. Retry timing includes the maximum feedback wait and measured handshake RTT, within the existing 2–20 ms limits and no more than one third of the selected recovery cutoff. The grant communicates the effective policy. No transition is silently omitted to reduce bandwidth.
3. **Reduce timer and resource-monitor work.** Persistent timers wake at the next retry, feedback, impaired-link or silence deadline, rather than polling maintenance every millisecond. The recording actor and QUIC driver use one current-thread runtime. Process-resource sampling runs on a separate monitor thread. The sender still checks unacknowledged history age and the receiver still enforces silence cleanup while idle.
4. **Make Windows timing experiments explicit.** Active lab work requests timer resolution; a scoped process policy explicitly allows Windows to honor that request. Each case records actual policy support and generator lateness. Optional MMCSS `Games` scheduling applies to the current lab thread and is reverted on exit. Optional `HighQoS` remains a separate experiment, disabled for the final run. A scoped system-required sleep request avoids idle sleep during the CLI benchmark; it does not override closing the lid or deliberately selecting Sleep. Idle product agents make none of these lab requests.
5. **Shrink the portable package.** The `desktop` profile uses size optimization, full LTO, symbol stripping and abort-on-panic for presentation. The independent agent retains unwinding for cleanup, speed optimization in the core/transport dependencies, and full LTO at link time. Size-oriented optimization is confined to entry/control/configuration code. Accessibility, Unicode support and required Slint attribution remain present. No executable packer or shared runtime installation was added.

These transport optimizations are implemented in the recording diagnostic adapter; enabling the future live input adapter still requires its own correctness and resource checks. LAN/Tailscale/WireGuard remain IP routes to the same authenticated transport. There is no NAT traversal implementation or dependency on a particular tunnel service.

## Gap-age evidence

The corrected-timer run before the range fix completed 33/35 network cases. The 2% loss seed-3 case stopped at 5.437 seconds with 5,424 applied transitions, although the next missing transition could not yet be 30 ms old. Its maximum actor gap was 2.943 ms. The 10 ms jitter case also stopped. Both failures and all missing/ungenerated counts are retained in [that complete run](../../artifacts/single-pc-20261024-085300/network/summary.json).

Two regression tests reproduced the error before the fix: ordinary out-of-order delivery and heartbeat-discovered history both incorrectly retained a deadline of 31 ms after repairing the older range, rather than the correct 50 ms deadline for the remaining range. The [before-fix test log](../../artifacts/gap-age-regression-20261024-090629/before-fix.log) preserves both failures. Existing tests still verify that partially repaired old history expires on time, including forced release of recorded held state. The fix does not suppress a deadline error or allow old input after expiry.

## Sustained validation

The [final network summary](../../artifacts/single-pc-20261024-091321/network/summary.json) records 35/35 passes. All 1,300,000 scheduled transitions were generated, applied, acknowledged and recorded before their selected recovery deadline. Missing and ungenerated counts, fidelity errors, cleanup failures and owned keys after cleanup were all zero. No case was interrupted. Maximum actor gap was 15.533 ms; send-history/reorder peaks were 32/23, within the 256/64 bounds.

The selected profile is strict 30 ms history recovery, MMCSS Games scheduling, supported explicit timer-resolution policy, OS power/QoS heuristics, and the default eight-event/5-ms feedback policy. [Environment metadata](../../artifacts/single-pc-20261024-091321/environment.json) and [suite policy](../../artifacts/single-pc-20261024-091321/network/manifest.json) are preserved. This is not a controlled A/B trial against the older 150 ms run.

| Bidirectional application loss | Seeded blocks | Scheduled | Delivered before 30 ms | Empirical fraction |
| --- | ---: | ---: | ---: | ---: |
| 0.5% | 5 | 100,000 | 100,000 | 100% |
| 1% | 5 | 100,000 | 100,000 | 100% |
| 2% | 5 | 100,000 | 100,000 | 100% |

These observations exceed the 99.99% descriptive goal under <=1% loss in this synthetic run. Events within a block are correlated, and five perfect blocks do not establish a rare-failure probability or a useful independent-event confidence guarantee. Router/NIC impairment, repeated controlled blocks and run-level uncertainty remain required.

Each sustained stress case completed 60 seconds. Timings below are **same-clock synthetic generation to recording submission**, excluding generator lateness; they are not C0-to-I0, application consumption or physical input latency.

| Nominal load | Applied events | p50 / p95 / p99, ms | Generator lateness p99, ms | Maximum actor gap, ms |
| --- | ---: | --- | ---: | ---: |
| 1 kHz | 60,000 | 0.3252 / 0.9360 / 1.4522 | 2.1320 | 3.5328 |
| 4 kHz | 240,000 | 0.3247 / 0.9005 / 1.3228 | 2.4476 | 3.5260 |
| 8 kHz | 480,000 | 0.3142 / 0.9558 / 1.3724 | 2.6059 | 3.3369 |

The older 150 ms profile's 1 kHz p99 was 0.7325 ms. The final observation of 1.4522 ms is worse; **no latency improvement or accepted latency regression waiver is claimed**. Scheduling, timer policy, runtime topology and desktop conditions differ, so a controlled alternating comparison is needed to attribute the change. The historical numbers remain visible rather than selecting only the fastest new block. The healthy path has no added input batching delay, but that fact alone does not prove equivalent latency tails.

All [14 lifecycle checks](../../artifacts/single-pc-20261024-091321/network/lifecycle.json) and [9 separate-agent SDK/crash checks](../../artifacts/single-pc-20261024-091321/sdk-crash/sdk-crash-result.json) passed. Recorded silence cleanup was 253.026 ms; 250/1,000 ms outage cases reached recorded neutrality in 251.165/263.787 ms, within the 300 ms watchdog-plus-allowance goal. The intentional cleanup-failure case observed one release failure, retained ownership and then completed its explicit retry. These are recorded-state checks, not OS-neutrality measurements.

## CPU, memory and bandwidth

| Sustained load | CPU, % of one logical core | CPU p95 sample window, % | CPU, % of eight-thread machine | Private peak, MiB | IPv4 estimate, Mbit/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 kHz | 32.212 | 42.117 | 4.027 | 5.289 | 0.9641 |
| 4 kHz | 39.557 | 54.609 | 4.945 | 9.852 | 2.5869 |
| 8 kHz | 48.202 | 64.014 | 6.025 | 18.238 | 4.8177 |

CPU includes both endpoints, generation, timing instrumentation and the resource monitor. It cannot certify **<=2% of one core per endpoint**. The old combined 1 kHz reading was 30.779%, so these changes have not demonstrated a CPU reduction. The final 1 kHz split was 13.958% user and 18.254% kernel time; these accounting totals do not identify a specific hot function or kernel subsystem. The much lower application-handler timings do not account for all transport/runtime work. P95 resource windows include the initial/terminal partial windows alongside roughly one-second samples.

The 1 kHz bandwidth estimate fell from 1.3943 to 0.9641 Mbit/s. New UDP counts were 51,446 client and 14,149 host datagrams over the 60-second case. It still retried 1,585 transitions; 4/8 kHz had zero retries. Maintenance woke 9,490 times at 1 kHz and zero times at 4/8 kHz, where input-driven ACKs kept up. Lower polling/feedback work is not sufficient evidence of a CPU win.

The bandwidth estimate includes QUIC UDP bytes and IPv4/UDP headers, including handshake traffic, but excludes Ethernet and tunnels. Generator lateness means traffic is not perfectly uniform at the nominal input rate. **Actual packet capture on the reference path is still needed to pass the bandwidth gate**, particularly with tunnel encapsulation. The memory figures include both endpoints and diagnostic vectors; they show headroom against 50 MiB, but isolated steady-state endpoint qualification remains pending.

The separate idle agent averaged **0.129% of one logical core** over 60.664 seconds, below the 0.5% idle goal. Private peak was 1,327,104 bytes / 1.266 MiB, with a -24 KiB end-to-start delta. [Raw idle samples](../../artifacts/single-pc-20261024-091321/agent-idle-resources.json), [derived analysis](../../artifacts/single-pc-20261024-091321/analysis.json) and [final executable/source integrity checks](../../artifacts/single-pc-20261024-091321/final-integrity.json) are retained. Code and build inputs were unchanged during measurement; only documentation changed after the frozen snapshot.

## Installed size

| Component | Bytes | MiB |
| --- | ---: | ---: |
| Slint frontend, `Bricks.exe` | 8,264,704 | 7.882 |
| Independent `bricks-agent.exe` | 3,579,904 | 3.414 |
| Both installed input executables | 11,844,608 | 11.296 |
| Complete folder with instructions/notices | 11,850,643 | 11.302 |

The executable total fell from 33,452,032 bytes / 31.902 MiB to 11.296 MiB, a 64.6% reduction. It still exceeds the **10 MiB** target by **1,358,848 bytes**. The agent alone fits the 5 MiB stretch value, but that does not qualify the complete input package. Debug symbols and lab/developer executables are excluded from this installed-size scope; Windows system dependencies are not bundled. The portable folder remains unsigned and is not an installer.

## Build, UI and provenance checks

The post-fix workspace passed **69 tests**, workspace Clippy for all targets with warnings denied, and formatting checks. [Validation logs](../../artifacts/gap-age-validation-20261024-090934/result.json) and [optimized backend build logs](../../artifacts/gap-age-build-20261024-091051/result.json) are retained. The two Windows lab tests verify that sleep and process power requests restore state, including unwind cleanup.

The actual portable package passed **10 rendered workflow checks**, including the agent-backed recovery operation, report export, persistence and confirmed cancellation. [Smoke results and screenshots](../../artifacts/performance-packaged-desktop-c965c3417a164e1ebae8606a3846b515/smoke-result.json) and [native Windows accessibility actions](../../artifacts/desktop-native-f992118b002f4c3686383a52ffe0f10d/desktop.json) are retained. These tests use fresh isolated data directories and do not inject OS input.

The rebuilt application was reopened with its separate bundled agent; the **Start Bricks** shortcut target and executable hashes matched. [Launch evidence](../../artifacts/performance-desktop-launch-20261024.json) is retained. [Documentation checks](../../artifacts/single-pc-20261024-091321/documentation-checks.json) found no broken relative links or unbalanced fences across 22 Markdown files, and all nine PowerShell scripts parsed successfully.

The unchanged, previously validated frontend binary is reused with the rebuilt backend; SDK wire version 1 is unchanged. Its original build source is [preserved separately](../../artifacts/performance-source-20261024-085255/source-manifest.json). The final backend/lab [source snapshot](../../artifacts/performance-source-20261024-091320/source-manifest.json) and [component provenance](../../artifacts/performance-provenance-20261024-091320.json) identify this run without requiring a Git repository.

```text
Bricks.exe        C103F6DF9403479D36371A09241C431D9850347B02658945B6AE477C8B516F22
bricks-agent.exe  A73C6046136E3D96A5034AA31D0E6AFB4776477986DEB5944AC1DB64952FB145
bricks-lab.exe    387AD26D1A813D85963E378D1BE5DC0AD7D3E9C8EBC95A0660454CA8F3FB371B
Cargo.lock        293376D34C1AC84F2B21C655FE0533B7D7E226E22EC2DF9118B85F0F69D7DB83
```

The development laptop is an Intel Core i5-8365U, 4 cores/8 logical processors, Intel UHD 620 driver 31.0.101.2135, Windows build 26200, Balanced power plan. The runner preserves the contemporaneous OS, CPU, GPU, power/battery, Rust 1.99.1, lockfile, configuration, executable hashes and selected policy metadata. The network is IPv4 loopback, with no NIC/tunnel or real input device in the measured path. Ordinary desktop background load and thermals are uncontrolled. All timings use one uncalibrated monotonic clock; nanosecond units do not establish nanosecond accuracy.

## Preserved exclusions and limits

- [Original strict-30-ms run](../../artifacts/single-pc-20261023-185559/network/summary.json): 11/35 passed; 24 stopped. The [historical report](2026-10-23-agent-recovery.md) remains unchanged. Its 150 ms results are historical characterization, not a substitute strict-profile pass.
- [Overnight run](../../artifacts/single-pc-20261023-202247/network/summary.json): incomplete, with multi-hour Modern Standby interruptions confirmed by [power events](../../artifacts/single-pc-20261023-202247/power-events.json). Suspended time invalidates its CPU/latency comparison. Interrupted cases are not labelled protocol passes.
- [Coarse-pacer/storage-full run](../../artifacts/single-pc-20261024-083947/interruption-note.json): roughly 15 ms generator lateness and a later disk-full error. Its superficially low CPU/bandwidth does not qualify a uniform 1 kHz result. Only generated build caches were removed to free space; evidence was preserved.
- New reports expose actor gaps, paced-generation lateness, missing and ungenerated events, bounded queue peaks, cleanup errors and interrupted status. End-to-end latency is never inferred by summing unrelated stage percentiles. A passed recording cleanup test does not establish physical Windows key release after an injecting process crash.

The resource and gaming targets in [targets and benchmarks](../targets-and-benchmarks.md) are unchanged. The next performance work is separate endpoint CPU profiling under a paced load and further frontend/dependency size reduction with the existing accessibility checks. Actual network packet capture must account for IP/tunnel overhead; the loopback estimate does not certify the 1 Mbit/s gate on Tailscale or WireGuard. Real capture/injection, mouse fidelity, external Windows release supervision, two-PC LAN/tunnel timing and the matched-quality Sunshine/Moonlight physical comparison remain open.

Windows implementation references: [timer/power process policy](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessinformation), [quality of service](https://learn.microsoft.com/en-us/windows/win32/procthread/quality-of-service), [system-required execution state](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate), [MMCSS](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service) and [reverting the thread task](https://learn.microsoft.com/en-us/windows/win32/api/avrt/nf-avrt-avrevertmmthreadcharacteristics). These APIs define requests and restoration semantics, not a hard latency guarantee.
