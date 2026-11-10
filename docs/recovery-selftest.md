# Single-PC recovery check

Run this from the Bricks folder:

```powershell
.\target\debug\bricks-host.exe recovery-selftest
```

No second PC, administrator rights, certificate exchange or setup is needed. Look for **`PASS: 12/12 recovery scenarios`** and JSON with `"status": "PASS"`. No keys are captured or injected into Windows. The command opens no network socket, changes no identities, and uses no real-time sleeps.

The binary has already been built in this workspace. To rebuild it after source changes:

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --locked --offline -p bricks-host
```

## What is implemented

`bricks-core::recovery` contains a transport-independent sender and receiver, layered over the existing ordered key state/injector contract:

- Send new transitions immediately. Retain their original IDs, epochs and capture timestamps in bounded history until cumulative application ACK.
- Retry an oldest due transition even when no new input is captured. A call returns at most one due retry; the runtime caller must pace repeated calls.
- Buffer a bounded window of out-of-order critical transitions. Apply only contiguous history, without an intentional healthy-path delay.
- Deduplicate applied and buffered records. Reject contradictory buffered copies. Feedback means highest contiguous transition accepted by the state/injector, not game consumption.
- Stop on queue/window overflow, recovery deadline, invalid state, clock regression or injection failure. Stop the receiver on silence too. Cleanup attempts releases for all recorded owned keys, retaining failed releases so the caller can retry.
- Reject old-epoch input and ACKs without applying them to fresh state. Old input does not refresh the silence watchdog.
- Require a fresh, separately authorized epoch after a terminal fault; this version does not automatically resume or reconstruct missing actions.

The 12 scenarios cover a healthy sequence, lost press, lost final release, reordered release, duplicate packets, lost ACK, unrecoverable gap, complete silence, sender overflow, receiver-window overflow, fresh-session isolation, and a deliberately failed cleanup followed by a successful retry. They compare exact recorded submissions and applied progress, not just final key state. Fault cases check cleanup before the final explicit stop so teardown cannot hide a missing automatic release.

The unit suite additionally exhausts all 24 packet permutations and 16 initial-drop patterns for four transitions (384 combinations), with duplicate deliveries. It tests deadlines, conflicting buffered copies, impossible ACKs, counter exhaustion, malformed events, clock regression, and ambiguous injection failure.

## Configuration

No-argument runs use the host example embedded at build time. To inspect a different current file:

```powershell
.\target\debug\bricks-host.exe recovery-selftest docs/examples/host.toml
```

Both role configurations are accepted because this is a local sender/receiver simulation; their identity/address fields are reported but never opened. The report includes the complete resolved configuration and its SHA-256 hash.

| `[recovery]` field | Default | Validation / use |
| --- | --- | --- |
| `send_history_capacity` | 256 | 1..=4096 unacknowledged critical transitions |
| `receive_buffer_capacity` | 64 | 1..=1024 buffered transitions and maximum distance ahead of the expected ID |
| `retry_min_ms`, `retry_max_ms` | 2, 20 | Clamp the retry interval computed from SRTT / 4 |
| `deadline_min_ms`, `deadline_max_ms` | 30, 150 | Clamp the recovery interval computed from 2 * SRTT + 10 ms |
| `silence_timeout_ms` | 250 | 100..=1000; receiver must be ticked while idle |
| `snapshot_interval_ms` | 20 | Reserved for future snapshot/heartbeat integration; not used by this engine |

Require `0 < retry_min <= retry_max < deadline_min <= deadline_max <= silence_timeout`. Old schema-v1 files can omit the added fields and receive the same defaults. Their resolved hashes now include these fields; do not compare a hash from an older executable as though the resolved schema were unchanged. Profiles still override only the previously supported motion-aggregation field.

This suite fixes simulated SRTT at 8 ms: defaults resolve to 2 ms retries and a 30 ms recovery deadline. Values are virtual-time test inputs, not measured network RTT or Windows timing. The public core accepts an immutable validated `Policy`; the adapter derives the above values from typed configuration. Live RTT measurement, policy negotiation and bandwidth pacing remain future runtime work. The CLI suite needs send capacity at least two for reordering scenarios; capacity-one behavior is covered by unit tests.

## Report interpretation and remaining work

Class: `deterministic_recovery_simulation`. No latency distribution is reported. `expected_unapplied_transitions` records deliberately unrecovered scenario history (including a rejected overflow capture); a clean final state never converts those actions into successful delivery. One cleanup failure is intentionally injected, observed while the key remains owned, and then resolved by retry. All reported cases must finish with zero owned keys. Any unexpected outcome exits nonzero rather than printing a successful report.

This is an M3 foundation completed while waiting for two-PC M2 testing. It does **not** finish M3, and it does not change the existing [QUIC probe](quic-probe.md), which still stops on gaps/loss. The engine does not authenticate peers itself: a future runtime must construct it only after authenticated authorization and poll that authorization while active.

Still pending: a feedback wire codec and its parser tests, live QUIC integration, accepted-versus-applied/selective feedback, recent-history redundancy, bounded retry bandwidth, heartbeat, snapshots and a neutral-baseline resync handshake, motion/wheel barriers, real capture queues and Windows injection integration, an external crash supervisor, two-PC measurements, and physical gaming proof. A library destructor cannot run after process termination; recorded cleanup is not evidence of real OS release or a crash watchdog bound. The engine allocates bounded standard collections; allocation-free hot-path behavior has not been established.

Next single-PC work can connect authenticated loopback recovery to a foreground receiver fixture and deliberately interrupt the connection, followed by mouse/barrier support and high-rate queue tests. Two-PC LAN and tunnel tests remain separate gates when hardware is available.
