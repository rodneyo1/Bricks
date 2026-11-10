# Single-PC deterministic recovery evidence

Executed by the agent on 2026-10-23:

```powershell
.\target\debug\bricks-host.exe recovery-selftest
```

| Observation | Result |
| --- | --- |
| Measurement class | `deterministic_recovery_simulation` |
| Status / scenarios | PASS / 12 of 12 |
| Simulated SRTT | 8 ms, a fixed test input |
| Resolved retry interval / history deadline | 2 / 30 virtual milliseconds |
| Silence timeout | 250 virtual milliseconds |
| Send / receive limits | 256 / 64 critical transitions |
| Final owned keys | Zero in every scenario |
| Expected deliberate cleanup failure | One; reported while still owned, then released by retry |
| Embedded host config SHA-256 | `33e87900b2da9d949801bb7e88e9a8f09814b3098053ebd83ceb60ac435da50e` |
| Raw generated report | `artifacts/recovery-selftest.json` (git-ignored) |

Scenarios: healthy immediate delivery; lost press; lost final release with no newer input; reordered press/release; duplicate packets; lost application ACK; unrecoverable gap; complete silence; send-history overflow; receive-window overflow; old-epoch input/feedback after a fresh epoch; and cleanup failure/retry. Exact recorded injection history and applied progress are checked. Fault-induced loss and overflow are deliberately expected failures of delivery, not successful event recovery. `expected_unapplied_transitions` preserves their known scenario counts; releases used for cleanup do not count as delivery of missing history.

The same compiled CLI passes with `docs/examples/client.toml` supplied explicitly, reporting config hash `818018bd31ff6d250b314e4355842c99378f79beefd6eaff22ebb41021913320`. Tests additionally run nondefault send capacity 2, receive capacity 1, retry clamps 3..10 ms, deadline clamps 40..100 ms and silence 500 ms. Extra CLI arguments exit with code 1.

Complete workspace verification: **48 tests passed**, none skipped; formatting and Clippy with warnings denied passed; workspace/all-target build passed. A core test exhausts 24 packet permutations times 16 initial-drop patterns (384 combinations), with duplicate deliveries. Other new tests check ACK bounds, deadline boundaries, clock regression, conflicting buffered records, counter exhaustion, malformed events, ambiguous injection and retryable cleanup, and typed queue/timer validation. These are correctness checks, not statistical samples or a recovery-probability benchmark.

As with the preceding identity/QUIC work, the restricted execution profile could not access current-user DPAPI for seven existing identity-dependent tests. The entire suite was rerun successfully with normal Windows profile access. The recovery CLI itself needs no DPAPI access.

No real packet impairment, OS capture/injection, game acceptance, scheduler timing, physical key release, process-crash supervision, one-way latency, two-PC LAN/tunnel performance or gaming improvement was measured. The implementation is a reusable key-recovery foundation; the existing QUIC probe still uses its previous stop-on-loss behavior. M2 and M3 acceptance remain incomplete. See [recovery scope and remaining work](../recovery-selftest.md).
