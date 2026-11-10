# Third user-run Windows fixture

Source: user-pasted attachment on 2026-10-23 containing the previous v1 report followed by a new v2 report. Only the second JSON object is the new run; it is saved as [run3.json](2026-10-23-interactive-fixture-run3.json). Debug build, local I0->A0 window receipt measurement, uncalibrated clock, unknown hardware/load. No network or physical-display measurement.

All 46 submissions produced a marked keydown timing sample. Zero reported submission/cleanup failures, raw-read errors, dropped timing samples, pending receipt, or unresolved possible test-key down. This checks the fixture's reported behavior, not all physical input/release correctness.

| Duration (ms) | Mean | Median | Observed p95, nearest rank | Maximum |
| --- | --- | --- | --- | --- |
| Submission to receipt, I0->A0 | 5.884 | 6.715 | 10.551 | 15.007 |
| Submission to API return, I0->I1 | 1.502 | 1.414 | 3.708 | 5.664 |
| API return to receipt, I1->A0 | 4.382 | 5.178 | 6.725 | 9.343 |

All 46 per-sample breakdowns sum to their corresponding total. Column percentiles describe separate empirical distributions; their percentiles must not be added. Forty-six human-triggered samples are exploratory and insufficient to establish tail guarantees or acceptance at another boundary.

The longest measured component on average is the interval after the OS call returns. That interval includes application/message dispatch, possible scheduling delay, and any fixture work before receipt. The v2 UI posted status formatting/redraw work immediately after submission, which could run before queued keyboard input. This is a concrete source of possible measurement interference; the data alone cannot attribute all observed delay to it or prove a Windows/network bottleneck.

After this run, status rendering was deferred until receipt, with a 250 ms fallback for a pending test. The report identifies this as `fixture_revision = deferred-feedback-v1`. No speedup is established until a new run is measured. Native button behavior, other queued messages, scheduling, and debug-build overhead remain in the diagnostic path.

Capture counts: 95 raw keyboard records, two unsupported records categorized as `scan_code_not_in_mapping_table`, 187 legacy keyboard receipts, 112 raw mouse records, two button records, relative displacement (-468,-649), and zero vertical wheel units. The actual unmapped key identities remain unknown; do not guess or silently treat them as supported. Zero wheel units in this run does not contradict the earlier run's registered wheel activity.
