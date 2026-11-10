# Targets and benchmarks

Status: all Bricks numbers below are provisional engineering targets, not measured results. First platform is Windows-to-Windows. Wired LAN, 1080p SDR/120 FPS, GPU models, and display hardware remain provisional. Preserve the [original targets](sources/original-targets-and-benchmarks.txt) for provenance; their collector constants and timing model are not adopted.

Implementation/characterization now includes a [single-PC agent/QUIC recovery suite](single-pc-agent-and-recovery.md). Its synthetic generation-to-recording, resource and fault results are reported separately; they do not replace the C0-to-I0, real cleanup, mouse, two-PC or physical gates in this document. No target is relaxed by that work.

Measured [2026-10-23 results](test-results/2026-10-23-agent-recovery.md) preserve both the failing 30 ms and completed 150 ms recovery profiles, including CPU, bandwidth and packaging gaps.

The [2026-10-24 performance work](test-results/2026-10-24-performance-fixes.md) records subsequent recovery, feedback, timer and packaging changes against the same targets.

## Measurement definitions

| Metric | Start and end | Limit |
| --- | --- | --- |
| Capture-to-submit | C0: client begins handling raw input, to I0: immediately before host OS injection call | Cross-clock correction required; does not prove game consumption |
| Capture-to-API-return | C0 to I1: injection API returns, with checked insertion count | Submission success is distinct from fixture/game receipt |
| Capture-to-consume | Client capture to observable host fixture receiving corresponding input | Cross-clock correction required; fixture and game behavior may differ |
| Software overhead | Client capture-to-send plus host receive-to-submit, measured locally | Sum paired components per event; excludes network and OS delivery to game |
| Input-to-photon (I2P) | Physical stimulus to changed pixels on client display | Primary gaming outcome; external measurement includes device and display |
| Added streaming latency | Remote I2P compared with local-host I2P | Interpretation depends on matched display/device paths; list all differences |
| Hold error | Host consumed up/down interval minus corresponding client captured interval | Paired event error, not comparison of unrelated distributions |
| Motion fidelity | Delivered cumulative displacement and barrier ordering | Total fidelity alone does not establish temporal fidelity |
| Frame age | Source frame timestamp to client presentation/visible display boundary | State whether software presentation or actual photons were measured |

Record p50, p95, p99, mean, count, run duration, losses, stale discards, resyncs, and uncertainty where applicable. Do not report latency only for successful events without reporting missing ones. Negative corrected one-way latencies indicate a clock/model problem, not exceptional speed.

Exact timestamp markers are defined in [timing policy](timing-policy.md). Every number also carries a measurement class:

| Class | Path and interpretation |
| --- | --- |
| Local | Capture and injection on the same PC without a network path |
| Loopback | Capture and injection through the same PC's network stack |
| Two-PC LAN | Capture on client A to injection on host B, over declared wired LAN |
| Tunnel | Capture on A to injection on B, over a declared direct/relayed tunnel route |
| Hybrid physical | Physical input to client display photons with the baseline video path held constant |
| Full-stack physical | Physical input to photons through Bricks video |
| Loopback software reference | Codec/UDP request-echo only; no capture or injection, never an input-latency result |
| Loopback QUIC recovery recording | Authenticated synthetic critical transitions and recording injector; real recovery/fault traffic, no OS capture/injection or physical gaming acceptance |

## Milestone acceptance targets by class

The adopted consolidated plan separates initial acceptance from the stricter optimization goals below. These are hypotheses pending hardware measurements, not performance guarantees.

| Capture-to-submit metric | Local | Two-PC wired LAN | Tunnel |
| --- | --- | --- | --- |
| Mean | <= 2 ms | <= 8 ms | Characterize; provisional planning budget RTT reference + 8 ms |
| p95 | <= 3 ms | <= 12 ms | Characterize; provisional planning budget RTT reference + 20 ms |
| p99 | <= 5 ms | <= 20 ms | Characterize; provisional planning budget RTT reference + 40 ms |
| Latency standard deviation | <= 0.5 ms | <= 2 ms | Characterize; provisional aim <= 5 ms on a stable link |
| Absolute signed mean paired hold error | <= 1 ms | <= 2 ms | Characterize; provisional aim <= 3 ms |

Use a contemporaneous p95 ping RTT as the stated tunnel RTT reference, measured under the same load. It is a conservative planning input, not a one-way delay estimate or a bound on latency tails. Tunnel acceptance must declare the measured network envelope and account for asymmetry/loss; the RTT-based expressions are not universal gates. Loopback input results are characterized separately and cannot substitute for a two-PC LAN pass. Distribution shape ratios are diagnostic only; per-event errors remain primary, and a dataset with negligible client hold variance makes a ratio uninformative.

## Stricter LAN optimization goals

Applies to a healthy, warmed-up wired LAN with measured RTT p95 at most 1 ms, normal CPU/GPU load, encryption enabled, default zero motion-aggregation delay, and no induced loss. Test keyboard transitions, clicks, and motion independently. These stricter goals are retained as optimization targets, not substituted for the adopted milestone gates above. API submission and fixture consumption are measured separately.

| Metric | Initial target |
| --- | --- |
| Capture-to-submit, all input classes | p50 <= 1.5 ms; p95 <= 3 ms; p99 <= 5 ms |
| Capture-to-consume in host fixture | p95 <= 4 ms; p99 <= 6 ms |
| Combined locally measured software overhead | p95 <= 1 ms; p99 <= 2 ms |
| Absolute paired hold error | p95 <= 2 ms; p99 <= 5 ms |
| Signed mean hold error | Absolute mean <= 0.5 ms |
| Healthy-path intentional jitter delay | 0 ms |
| Optional motion aggregation | Default 0; experiments 0.5 ms and 1 ms |
| Critical event fidelity | Zero missing/duplicate/out-of-order critical applications in fault-free test |
| Relative displacement | Exact final cumulative totals, no cross-barrier application |
| Event load | 1,000 Hz mouse gate; 4,000/8,000 Hz stress characterization |

These are deliberately stricter than the original 20-40 ms input goals. They may need revision after the first trustworthy baseline; revisions require recorded evidence and must not be relabeled as achieved results. Timestamp uncertainty must be <= 0.25 ms for sub-millisecond mean-error conclusions. If not, report the result as inconclusive and use same-clock overhead measurements or external instrumentation.

A provisional p95 engineering allocation is 0.2 ms capture/normalization, 0.3 ms client queue/send, 1 ms network transit, 0.3 ms host receive/queue, and 0.5 ms injection submission: 2.3 ms of planned headroom usage within a 3 ms target. This is an allocation, not measured data or a claim that the sum of stage p95 values equals the end-to-end p95. Measure end-to-end distributions directly.

## Gaming targets and competitor criterion

Full-stack stretch goals on the declared 1080p/120 profile: I2P p50 <= 35 ms, p95 <= 50 ms, p99 <= 65 ms. These apply only to selected low-latency test scenes and hardware, not every game or Internet path. A 120 Hz frame interval is about 8.33 ms; game tick phase, scanout, and queues consume that budget quickly.

The competitive release claim requires full Bricks versus tuned Sunshine + Moonlight, on identical hardware and route, with matched resolution, FPS, codec, display mode, audio behavior, and a declared comparable visual-quality threshold:

- I2P p50 improves by at least `max(1 ms, 10% of baseline p50)`.
- I2P p95 improves by at least `max(1 ms, 10% of baseline p95)`.
- A 95% confidence interval on each paired improvement excludes zero, and the observed improvement exceeds the measurement uncertainty.
- I2P p99 does not regress beyond measurement uncertainty. No additional stuck input, increased stall/drop rate, or quality downgrade is accepted to obtain the latency result.

The 10% threshold is a project decision, not a sourced claim about achievable improvement. A hybrid victory establishes an input-path improvement under shared video, not a complete replacement victory. Publish results per tested profile; do not generalize a LAN win to every GPU, refresh rate, or network.

Before running the comparison, select fixed scenes and matched quality settings. Report bitrate plus objective quality measures against the same reference captures where available, and inspect text readability, fine motion, and artifacts. Predeclare an acceptable quality tolerance (for example, a chosen quality score range and no visible scene-specific regression); do not pick it after seeing latency. A result without this declared tolerance is exploratory, not a qualifying win.

## WAN, loss, and recovery

Use separate route classes; never require the same absolute latency on LAN and distant paths. One-way delay cannot be inferred exactly from RTT. Under symmetric conditions, input transmission adds roughly RTT/2; the full interactive input/video network loop adds roughly one RTT before processing and display costs.

| Condition | Required interpretation |
| --- | --- |
| RTT 5, 20, 40, 80, 120 ms | Report absolute latency, software overhead, offset uncertainty, and path symmetry assumptions |
| Independent loss 0.5%, 1%, 2% in each direction | Report critical recovery probability, application latency, retry traffic, and missed taps |
| Bursts of 2, 5, 10 packets; 50/250/1,000 ms outages | Report time to neutral state, resync count, state correctness, and resume behavior |
| Reordering/duplication at 0.1% and 1% | No duplicate critical application; no old-epoch input |
| Added delay variation 1, 5, 10 ms | Publish timing error and queue growth; no fabricated timing to mask it |
| Network saturation | Input remains bounded; media rate adapts; report tails under bidirectional load |

Under <= 1% independent loss on the reference LAN, aim for >= 99.99% of critical transitions to reach the host before the configured recovery deadline. This is an empirical goal, not a mathematical delivery guarantee. Report denominator, confidence interval, and all resyncs; use at least 100,000 critical transitions for this recovery characterization. Snapshots repairing final state do not count as delivery of missing transitions.

On total silence, aim to release owned state within the 250 ms watchdog timeout plus 50 ms OS scheduling allowance on the reference hardware. Test peer crash, host worker failure, and supervising-process behavior separately; if the injector process crashes before releasing state, an in-process timer cannot satisfy this goal. Final cleanup must be externally observed. No fixed p99-to-LAN ratio is imposed on WAN/loss tests.

## Resource targets

| Resource | Input-only initial goal | Scope |
| --- | --- | --- |
| Idle CPU | <= 0.5% of one logical core | 60-second average, no event trace |
| Active CPU per endpoint | <= 2% of one logical core at 1 kHz input | Report p95 windows and total-machine normalization too |
| Private memory per endpoint | <= 50 MiB; 30 MiB stretch | Steady-state after warm-up; not GPU memory |
| Installed input binaries | <= 10 MiB; 5 MiB stretch | Release binaries excluding symbols; list shared dependencies |
| Input bandwidth | <= 1 Mbit/s at 1 kHz | Report actual packet rate and network/tunnel overhead |

A 24-byte payload at 200 packets/s is 38.4 kbit/s before headers, security, ACKs, and tunnel overhead. It is not an appropriate upper bound for high-rate mice. Media CPU/GPU time, surface memory, decode buffers, binary/dependency size, and bandwidth receive separate budgets after capability probing; input-only budgets do not constrain a full video stack to 30 MB.

## Benchmark protocol

1. Record source revision, build flags, exact toolchain/lockfile, resolved config/hash, backend IDs, Windows build, CPU/GPU, driver, NIC/link, display model/refresh, device polling, power settings, and capture/codec/presentation settings. Record Sunshine/Moonlight versions and full comparison settings.
2. Verify an observable host receiver fixture and physical timing rig. Warm up for 60 seconds; record at least five alternating A/B runs per condition. Randomize run order and control background load and thermals.
3. Exercise at least 10,000 critical transitions per healthy condition. Include 5,000 ordinary key transitions, modifiers/chords, rapid taps, repeated press/release, 20-400 ms holds plus 10-second holds, both left/right modifiers, extended keys, and all supported mouse buttons. Explicitly test repeat ownership.
4. Run at least 60 seconds of 1 kHz motion plus 200 clicks; add wheel/drag and motion-click-motion barriers. Characterize 4/8 kHz, multi-device input, focus loss, escape, suspend/resume, reconnect, and queue overflow. Clipboard/paste transport is out of scope.
5. For physical I2P collect at least 1,000 stimuli per condition; use larger sets when assessing tails. Record instrument resolution and its confidence limits. A 1,000 FPS camera samples in 1 ms steps; use a calibrated photodiode rig when smaller improvements matter. Keep stimulus and display scanout positions consistent.
6. Compare local host, baseline Sunshine + Moonlight, hybrid where validated, and eventually full Bricks. Run idle and GPU/CPU-loaded cases, fullscreen and borderless, 60/120 Hz, and higher refresh only where supported. LAN, tunnel-direct, tunnel-relay, and Wi-Fi results are separate profiles.
7. Apply impairment in both directions on a controlled bridge/router and record observed delay, jitter, loss, duplication, reordering, throughput, and queueing. Trace gaps alone are not proof of network loss: distinguish unsent, sent, received, deduplicated, injected, consumed, and logger-dropped records.
8. Produce machine-readable results, latency CDFs/histograms, paired hold errors, frame-age/stall plots, and raw artifact references. Use run/block-level bootstrap or another justified method for confidence intervals; do not treat autocorrelated events as independent runs.

No hardware benchmark has been executed yet. Expected result files should include a run manifest, redacted resolved configuration, counters, optional binary traces, analysis summary, and report. Record exclusions before calculating results.

## Clock and instrumentation rules

Client records capture/send times; host records receive, validation, submission start/end, and fixture-consume times with event IDs and epochs. Full media records source frame IDs and capture/encode/decode/present boundaries. Use explicit markers for missing values rather than zero timestamps.

Local clocks are monotonic with recorded conversion factors. Estimate cross-clock offset using repeated four-timestamp exchanges, fit drift over the run, and publish round-trip delay and the offset uncertainty interval. Arbitrary path asymmetry remains unresolved by ping alone. Do not subtract two raw monotonic timestamps from different PCs or present nanosecond units as nanosecond accuracy. Hold durations can be compared using each PC's local intervals, with clock-rate drift considered.

Full traces must be asynchronous, bounded, opt-in, and visibly marked. Measure tracing overhead on/off. A valid completeness analysis requires no trace drops or an independent reliable counter source. External physical I2P does not depend on synchronizing the two PC clocks.

## Regression gates

Once a stable reference exists, CI runs deterministic codec/state/recovery checks; hardware runners execute the declared LAN profile. A candidate must meet absolute input correctness gates and current latency thresholds. A > 10% latency regression larger than instrument uncertainty requires investigation and a documented target change before acceptance. A performance exception cannot waive stuck-key, unauthorized-input, replay, or epoch-isolation failures.

Initial hardware gates: the two-PC LAN milestone acceptance table; zero stuck keys/buttons after every tested cleanup; no stale-session injection; memory/queue bounds; no silent transition loss. Healthy-path dropped critical events and reordered/duplicate critical applications must be exactly zero. Under induced faults, report missing history explicitly and apply the declared recovery criteria; do not assert zero drops under arbitrary outages. Full-media gates are added only when media exists. [Moonlight's measurement guidance](https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions) explains why overlay numbers alone are insufficient for a cross-client victory claim.
