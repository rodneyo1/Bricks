# Fifth user-run Windows fixture: unmapped-key diagnostic

Source: user-pasted diagnostic attachment on 2026-10-23. Extracted [JSON report](2026-10-23-interactive-fixture-run5.json). Diagnostics were enabled, with zero unsupported-code drops. Fixture revision: `deferred-feedback-v1`; local uncalibrated debug-mode I0->A0 window-receipt measurement.

The fixture processed 255 raw keyboard records and classified none as unsupported. Both unsupported-reason and unsupported-code collections were empty. The earlier unmapped records were not reproduced, so their identities and cause remain unknown. This supplies evidence for the inputs exercised in this run, not exhaustive keyboard/layout support. No mapping changes can be inferred from an empty diagnostic collection.

All 32 explicit test submissions produced consecutive timing records; all per-sample component sums and the summary array agree. Zero reported submission/cleanup failures, raw-read errors, dropped latency samples, pending receipts, or unresolved possible test-key down.

| Submission-to-window-receipt duration | Milliseconds |
| --- | --- |
| Mean | 3.037 |
| Median | 3.273 |
| Observed p95, nearest rank | 4.628 |
| Maximum | 4.747 |

The run registered 301 raw mouse records, 14 button records, relative displacement (-569,-773), and zero vertical wheel units. The latter does not establish wheel failure; earlier testing registered wheel activity. One focus loss was reported.

These measurements do not establish capture-to-submit gates, calibrated tail latency, two-PC behavior, game compatibility, or physical latency. Further repetitions of this same small test are not required to proceed with supported-input network integration. Retain the earlier unmapped issue as non-reproduced; use the existing opt-in diagnostic if it recurs rather than inventing mappings.
