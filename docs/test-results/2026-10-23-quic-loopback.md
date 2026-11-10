# First compiled QUIC loopback self-test

Executed by the agent on 2026-10-23 under normal Windows profile access:

```powershell
.\target\debug\bricks-host.exe probe-selftest 100
```

This was a debug build with DPAPI-protected temporary identities and actual Quinn/Tokio UDP loopback endpoints. The local generated report is at `artifacts/quic-loopback-selftest.json` (generated, git-ignored). No OS input was captured or injected.

| Observation | Result |
| --- | --- |
| Measurement class | `loopback_quic_synthetic` |
| Boundary | Before datagram send to receipt of matching application ACK |
| Synthetic transitions / host-applied transitions | 200 / 200 |
| Recording injector submissions | 200 |
| Duplicates / unchanged transitions | 0 / 0 |
| Owned keys before / after cleanup | 0 / 0 |
| Cleanup failures | 0 |
| Mean roundtrip | 0.498184 ms |
| Observed p50 / p95 / p99 | 0.4501 / 0.8075 / 1.2384 ms |

This establishes a functional authenticated loopback datagram session and a complete acknowledged synthetic sequence. It is a single uncalibrated exploratory run with no full hardware/build manifest or statistical confidence interval. No acceptance threshold, hardware input response, WAN behavior, physical latency, or advantage over another transport/system is established.
