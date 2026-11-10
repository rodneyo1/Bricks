# Timing policy: immediate, faithful forwarding

Decision: use immediate forwarding and injection for live gaming. This resolves the timing question before live capture or networking is implemented.

## Healthy path

Timestamp when the client application begins handling a raw input record, send promptly, and submit each authenticated valid event to the host OS as soon as its ordering dependencies are satisfied. No intentional playout delay, minimum key-hold duration, fitted distribution, or random release timing is added. A configured motion-aggregation experiment may delay motion by its explicit bounded interval, never keyboard releases.

Let corresponding client events occur at `C1` and `C2`, and let their complete capture-to-host-submission delays be `D1` and `D2`. Then:

```text
host inter-event interval = (C2 - C1) + (D2 - D1)
```

Constant delay cancels. Differential network and processing delay changes the interval. We accept that residual error, reduce queueing where measurements justify it, and report paired interval errors honestly. This policy preserves causal player actions; it does not guarantee identical timing over a jittery network.

## Recovery path

An absent critical transition or motion barrier can block dependent events. The recovery layer may wait within the configured bounds, retry, and then suspend/resynchronize rather than inject obsolete history. Report recovered-event latency, blocked time, missed taps, resyncs, and dropped/superseded motion separately from healthy-path latency. Snapshots repair state; they cannot reconstruct the timing of missing actions.

The initial state core rejects a gap without applying the event. The later recovery layer owns buffering/timers. This separation lets deterministic tests prove ordering without pretending that retries or watchdogs already exist.

The [first recovery layer](recovery-selftest.md) now implements bounded key reordering and retry/deadline timers with explicit local monotonic time. Virtual-time tests establish state behavior only. QUIC integration, real-time timer scheduling, snapshots/resync and externally observed OS cleanup remain pending; a simulated timer is not a measured watchdog bound.

## Alternatives and why they are not defaults

Adding the same fixed delay to each packet's arrival time does not remove differential jitter; it merely adds latency. Timestamp-based playout against a translated client clock can absorb bounded jitter by delaying early events until scheduled deadlines. That is a different design, with clock uncertainty, late-event handling, and a latency/fidelity tradeoff; it may be evaluated later as an explicit optional backend policy.

The original sampled-keyup proposal yields `host_hold = client_hold + differential_delay + sampled_delay`. It adds an artificial hold to the actual action. It remains rejected; it is not an available configuration mode.

## Exact measurement boundaries

| Marker | Meaning |
| --- | --- |
| C0 | Client begins handling the raw input record; application receipt, not physical switch time |
| S0 | Immediately before submitting the encoded datagram to the socket/transport API |
| R0 | Host application obtains the received datagram |
| I0 | Immediately before calling the OS injection API |
| I1 | Immediately after the injection API returns, with insertion result |
| A0 | Receiver fixture handles the corresponding application-visible event |
| P0 / P1 | External physical stimulus / client display response |

`capture_to_submit = I0 - C0`. `capture_to_api_return = I1 - C0`. `capture_to_consume = A0 - C0`. `input_to_photon = P1 - P0`. Cross-machine differences require a stated clock model and uncertainty. Preserve these exact names in reports. Do not use “injection completed” to describe I0 or I1, or redefine “submit” to mean application receipt.

Scheduler and message-loop delays are part of the measured path. Clock offset, drift, timestamp placement uncertainty, and instrument resolution need separate bounds; they are not grounds to subtract real latency.
