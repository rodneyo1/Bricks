# Input protocol and recovery

This is a semantic specification. Freeze byte layout, IDs, and conformance vectors during M1 before interoperable binaries ship.

Implemented subset: the [transport-independent key recovery engine](recovery-selftest.md) has bounded sender history, timer retries, cumulative applied ACKs, reorder/deduplication, terminal fault handling and recorded cleanup. The [QUIC recovery adapter and lab](single-pc-agent-and-recovery.md) now carry grants, datagram feedback, heartbeats and critical transitions over authenticated loopback connections. The older stop-and-wait probe remains separate. Live OS input, motion, selective feedback, redundancy and snapshots/resync below remain specifications.

## Transport

Use one authenticated QUIC connection per control session: reliable streams for configuration/lifecycle and unreliable datagrams for time-sensitive input. Input must wait for completed authentication and ownership grant; disable input in 0-RTT data. QUIC datagrams are not retransmitted as application input by the transport; application ACKs described below are separate from QUIC packet ACKs. [RFC 9221](https://www.rfc-editor.org/rfc/rfc9221.html) defines the datagram extension.

Reliable stream delivery is suitable for control but can delay later messages on the same stream after loss. Never route latency-critical releases solely through a stream behind other control payloads. Build a small loopback-only UDP reference early, then compare an equally protected/authenticated UDP alternative during the transport bake-off; do not write custom cryptography. The initial reference echo never accepts remote peers or injects input and cannot establish production transport superiority.

Live timing follows [immediate forwarding](timing-policy.md): residual differential jitter is measured, not replaced by sampled hold times. Authentication is mandatory before any live ownership grant, starting at M1.

## Identity and event representation

Proposed datagram envelope: protocol version, type, session epoch, packet sequence, base client timestamp, payload length, and event count. Each record carries a type, device ID, global capture event ID, timestamp delta, and typed payload. All IDs and timestamps have explicit fixed-width integer limits and byte order in the later codec spec; wire decoding must not depend on Rust memory layout or enum representation.

Use independent monotonically increasing 64-bit packet, event, and critical-transition counters. A new authenticated ownership grant creates a new random epoch and resets counters; do not allow wrap within an epoch. The epoch binds to the connection and prevents delayed application records from a prior session being accepted.

| Record | Payload semantics |
| --- | --- |
| Key | Physical usage page/usage ID, down/up, device, extended metadata where needed |
| Mouse button | Button ID and down/up |
| Relative motion | Signed 64-bit cumulative X/Y totals per motion segment, sample identity and time |
| Absolute motion | Negotiated coordinate space, monitor/desktop geometry generation, position |
| Wheel | Signed cumulative wheel totals with negotiated units, segment identity |
| State snapshot | Snapshot generation, last critical transition, currently owned pressed keys/buttons |
| ACK/state feedback | Highest contiguous critical transition plus bounded selective ACK bitmap and applied snapshot generation |

The global event ID provides correlation, not a rule that all missing mouse samples must block later keys. Critical transitions have their own causal ordering. Physical usages need a published Windows/Linux mapping table and explicit unsupported-key handling before cross-platform support.

Keep application datagrams within the negotiated QUIC maximum; initially cap at 1,200 application bytes or a smaller discovered limit. Keep common input packets much smaller. Measure payload, encrypted UDP datagram, IP, and tunnel overhead separately. Neither a 16-byte event target nor a 24-byte complete-packet limit is adopted.

## Delivery policies

**Keys and buttons:** Send immediately and retain unacknowledged transitions. Include a bounded recent transition history in following packets and retry on timers even when no newer input exists. ACK delivery means the receiver accepted a transition into its bounded application state; also report applied progress for diagnosis. Deduplicate by epoch and transition ID. Apply critical transitions in order, never inject a late duplicate.

**Motion and wheel:** Use cumulative counters so losing an intermediate packet need not lose total displacement. Each critical transition closes a motion segment and carries its final counters as an ordering barrier. A new segment begins after the barrier. Apply unseen counter differences exactly once, never across a missing barrier. Intermediate motion packets may be superseded; a delayed old packet must never move the pointer backwards. Wheel packets follow the same barrier rules. This recovers totals, not the precise time history of motion during a loss burst.

**Dependency gaps:** Healthy sessions have no reorder delay. If a critical barrier is missing, buffer dependent records within configured bounds and solicit/retry missing transitions. An initial experiment allows at most 2 ms local reorder waiting before entering explicit recovery. This is not a guarantee that recovery finishes in 2 ms. Motion unrelated to a blocked segment must not be guessed into the wrong ordering.

**Snapshot repair:** Send current pressed-state snapshots every 20 ms while active and immediately after ownership/state changes. Send authenticated heartbeat/feedback at the same cadence while controlling. A snapshot has a generation, critical cutoff, and cumulative-motion baseline. If the missing history cannot be recovered by its deadline, suspend normal injection, discard older pending work, release Bricks-owned state, acknowledge a resync generation, and resume from an agreed fresh neutral baseline. Never replay obsolete clicks or presses to catch up. Snapshots can repair held state but cannot reconstruct a missed tap; count all such history loss as a fault/degraded run.

## Provisional bounds

These defaults must be tuned with traces and recorded per run:

| Parameter | Initial policy |
| --- | --- |
| Recent critical history | Up to 8 transitions, subject to packet size |
| Critical send history | At most 256 transitions; overflow triggers resync |
| Retry timer | `clamp(SRTT / 4, 2 ms, 20 ms)` plus feedback-triggered retry |
| Late critical recovery deadline | `clamp(2 * SRTT + 10 ms, 30 ms, 150 ms)` |
| Critical receive reorder capacity | 64 transitions; overflow triggers resync |
| Motion queue | One pending cumulative sample per unblocked segment |
| Heartbeat/state cadence | 20 ms |
| Silence watchdog | 250 ms since last valid authenticated session packet |
| Session control lease | Explicit renewal, revoked on stop or watchdog expiry |

Pace retries and redundancy inside a small negotiated input bandwidth cap. High-rate mice must not cause unbounded retries, packet storms, or queues. No delivery or latency guarantee is possible during arbitrary loss or outage. Long-RTT paths may enter degraded mode under these gaming-oriented deadlines; report that outcome rather than claiming success.

## Lifecycle and malformed traffic

1. Pair and verify the peer; negotiate version, capabilities, caps, key map, and timestamp scale.
2. Host grants exclusive ownership and a new epoch from neutral state.
3. Capture only after the client enters active mode. Require release of pre-held local keys.
4. On focus loss or escape, send release-all and suspend; host watchdog independently covers failed delivery.
5. On loss of authorization, disconnect, process failure, or watchdog expiry, release owned keys/buttons and invalidate the epoch. A best-effort cleanup handler cannot cover every crash; test whether the chosen backend needs a separate supervising process to meet the release bound.
6. Reconnect starts a new epoch and neutral state. No queued input survives it.

Validate all lengths/counts before allocation, bound decomposed records and histories, reject unknown required capabilities, rate-limit invalid traffic, and terminate on inconsistent state. Authentication failure, replay, wrong epoch, and duplicate packets must not cause injection. Exhaustive parser fuzzing and deterministic state-machine tests are required before live exposure.
