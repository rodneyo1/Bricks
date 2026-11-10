# Review of the supplied decisions

The original input-first prototype is a useful starting point, but its timing model and benchmark logic conflict with the gaming objective. Keep the small Rust core and platform separation; replace synthetic timing with faithful forwarding and add a complete latency measurement plan before optimizing.

## Decisions

| Original choice | Verdict | Bricks decision |
| --- | --- | --- |
| Rust, edition 2024 | Keep, with corrected rationale | Native performance, memory safety, good FFI. Rust does not make scheduling deterministic. C++ could perform equally well; language alone does not establish an advantage. |
| Input first; video optional | Keep for early experiments | An input-only milestone can improve one component. A complete Sunshine + Moonlight competitor requires video, presentation, and audio. |
| Sample log-normal key holds | Reject | Preserve each actual press and release. No fabricated motor noise or statistical masking. |
| Default 8 ms mouse coalescing | Reject for gaming | Event-driven forwarding, with an optional bounded 0.5-1 ms aggregation experiment. Measure 1 kHz through 8 kHz devices. |
| Default 1-2 ms input jitter buffer | Reject as a universal default | No intentional delay on the healthy path. A bounded recovery wait is a separate loss-path operation. |
| UDP over an existing tunnel | Keep as an experiment | Production baseline uses authenticated QUIC datagrams and streams. Compare protected UDP over WireGuard if QUIC overhead proves material. |
| Static shared token | Replace | Explicit pairing, pinned peer identities, encrypted authenticated traffic, revocation, and session epochs. |
| NACK-only recovery | Reject | ACKed transitions, proactive redundancy, timers, state snapshots, and bounded resynchronization. |
| Arbitrary reordering handled by sequence numbers | Reject the guarantee | Sequence numbers detect order. They cannot recover absent events without waiting, retrying, or skipping history. |
| Global low-level Windows hooks as main capture | Change | Raw Input is the primary data source; hooks only where needed for local suppression or escape handling. |
| SendInput on Windows | Keep as a compatibility baseline | Measure acceptance in each game. Driver-level input is a separate, deferred compatibility project. |
| evdev plus a separate uinput crate | Simplify | Evaluate the evdev crate's existing uinput support before adding another wrapper. |
| quanta/TSC is inherently better than Instant | Reject the assumption | Start with OS-backed monotonic time; benchmark clock overhead before adding a clock dependency. |
| JSON for every event in the hot path | Change | Bounded binary trace records to an asynchronous writer; JSON conversion offline. |
| Custom reliable UDP in roughly 400 lines | Reject the estimate | Correct recovery, security, pacing, replay handling, and testing are substantial work. Reuse transport security. |
| 30 MB RAM / 5 MB binary for everything | Reclassify | Input-only stretch goals. Video surfaces, GPU memory, and codec dependencies require separate budgets. |

## The timing model is mathematically wrong for fidelity

Let client hold duration be `H`, down/up network-plus-processing delays be `D_down` and `D_up`, and the sampled extra release delay be `S`. The proposed immediate-down and delayed-up behavior produces:

```text
host_hold = H + (D_up - D_down) + S
```

It adds a sampled hold to the real hold; it does not replace the real hold with a fitted distribution. Even scheduling release relative to the injected down would alter actual player intent. Waiting for both events would delay the press itself. None of these is appropriate for live gaming.

Immediate forwarding produces `host_hold - H = D_up - D_down`. A constant path delay cancels; differential jitter remains. A playout buffer can smooth timing only by adding latency. It cannot recreate an event before that event arrives. This is a tradeoff, not a problem solved by drawing random numbers.

## The supplied targets need new foundations

- The cited `content.js` and `background.js` were not supplied. Their constants and purported provenance cannot be verified from these attachments.
- Rounding to 0.01 ms says how a value is represented, not how accurately it was measured. Sampling a mouse every 50 ms does not make shorter disturbances invisible: they can change subsequent sampled positions and motion.
- External collectors' batching and hover thresholds do not define gaming requirements. Remove those dependencies entirely from the implementation.
- RTT of 120 ms implies about 60 ms one-way only under symmetry. A universal 25 ms mouse target across such paths is infeasible; an RTT-derived estimate is not a one-way measurement.
- A key payload smaller than 16 bytes is incompatible with two independent 64-bit fields plus event data unless compressed. A payload size is also different from an encrypted UDP packet or on-wire size.
- NACK recovery can cost roughly another RTT after detection, and a missing final key-up may generate no gap at all. One percent loss does not imply full or timely recovery.
- A 100 ms recovery deadline and a fixed LAN-relative p99 bound cannot hold across arbitrary RTT, loss bursts, or outages.
- Matching two hold-time distributions does not show that corresponding events preserve timing. Use paired per-event errors; distribution plots are secondary.
- A KS p-value above 0.05 is not proof of equivalence. Standard KS significance tables also do not directly apply when distribution parameters were fitted to the same samples. Model fitting is removed from the live path, so it is not a release gate.
- Two monotonic clocks with nanosecond units still have unrelated origins and drift. Ping exchanges need uncertainty bounds; asynchronous path delays bias offset estimates.
- An injection API returning successfully does not mean the game consumed the event, and neither timestamp establishes when the player sees a response.

## Missing requirements now covered

Input ownership, focus loss, disconnect cleanup, reconnect epochs, key-repeat ownership, physical key mapping, high-rate relative mouse motion, simultaneous modifiers, state recovery, bounded queues, authenticated pairing, replay resistance, and privacy-safe tracing are required from the first live session.

Gaming additionally needs a compatibility matrix, fullscreen/cursor-lock testing, frame-age and presentation measurements, GPU-load testing, audio synchronization, equivalent image-quality comparisons, and physical input-to-photon measurements. Controller support is explicitly deferred rather than silently assumed.

## Evidence and limits

Microsoft documents [Raw Input](https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input) as a device-oriented API. [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput) has integrity-level restrictions, and [relative mouse injection](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-mouseinput) can be affected by Windows pointer settings. These facts motivate compatibility experiments; they do not establish compatibility with a particular game.

[Moonlight's FAQ](https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions) distinguishes network, decode, queue, and render statistics and recommends external hardware for the most accurate latency assessment. [Tailscale](https://tailscale.com/docs/reference/connection-types) distinguishes direct and relayed connectivity; benchmark them separately. The proposed Bricks architecture and target values are our engineering decisions, not conclusions established by those sources.
