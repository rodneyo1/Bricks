# Fourth user-run Windows fixture

Source: user-pasted attachment on 2026-10-23, reporting `fixture_revision = deferred-feedback-v1`. The extracted [JSON report](2026-10-23-interactive-fixture-run4.json) preserves the supplied values, including its feedback-policy string. Debug build, uncalibrated local I0->A0 measurement, unknown hardware/load. This is not capture-to-submit, two-PC, game, or physical latency.

All 49 submissions produced timing records with consecutive IDs. No reported read errors, submission/cleanup failures, timing-sample drops, pending receipt, or unresolved possible test-key down. Every per-sample API duration plus post-return interval equals its corresponding total, and the summary sample array matches the detailed records.

| Submission-to-receipt duration (ms) | Previous visual-feedback run, n=46 | Deferred-feedback run, n=49 |
| --- | --- | --- |
| Mean | 5.884 | 2.759 |
| Median | 6.715 | 2.963 |
| Observed p95, nearest rank | 10.551 | 5.508 |
| Maximum | 15.007 | 5.644 |

| Mean component duration (ms) | Previous | Deferred feedback |
| --- | --- | --- |
| I0->I1, time through OS submission call | 1.502 | 1.518 |
| I1->A0, interval after API return | 4.382 | 1.240 |

The improvement is primarily in the measured post-return component, while average API-call duration is similar. This is consistent with reducing UI interference. These are separate human-triggered runs, not randomized matched trials: trigger method, load, and input mix are not controlled. No exclusive causal attribution, confidence interval, general speedup, or accepted latency gate follows from this comparison. Preserve all samples, including slow ones; no warm-up exclusions were introduced retrospectively.

There were 108 raw keyboard records, 206 legacy keyboard receipts, and four unsupported raw records categorized as `scan_code_not_in_mapping_table`. The identities of those keys are still unknown, so complete keyboard coverage is not accepted. Raw/legacy counts are different streams and are not an event-loss calculation. This run registered no mouse, wheel, or button activity; it supplies no new evidence about those paths.

Further repetitions of the same small diagnostic are not a substitute for the planned controlled benchmark. Next correctness work is identifying unmapped inputs through explicitly enabled diagnostics and completing the M1 identity/session lifecycle; performance acceptance still needs repeatable loads, calibrated boundaries, and the appropriate local/two-PC fixtures.
