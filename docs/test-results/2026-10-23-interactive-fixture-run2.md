# Second user-run Windows fixture

Source: user-pasted diagnostic JSON on 2026-10-23. Debug build, local interactive fixture, no hardware inventory or clock calibration supplied. Boundary is I0 before SendInput to A0 processing the marked WM_KEYDOWN, not capture-to-submit or physical latency.

| Observation | Reported value |
| --- | --- |
| Duration | 35.3876555 seconds |
| Explicit F8 submissions / marked receipts | 2 / 2 |
| Submit-to-receipt samples | 5.6933 ms, 0.9399 ms |
| Last submit-to-API-return duration | 0.5272 ms |
| Submission/cleanup failures; read errors; dropped latency samples | 0; 0; 0 |
| Unresolved possible test-key down; pending receipt at exit | false; false |
| Raw keyboard / unsupported records | 87 / 0 |
| Unsupported reason categories | none |
| Legacy keyboard receipts | 92 |
| Raw mouse / button records | 335 / 4 |
| Relative X/Y totals | -428 / -640 |
| Absolute mouse records | 0 |
| Vertical wheel units | -120 |
| Focus losses | 1 |

Observed keyboard records all fit the supported mapping in this run, and vertical wheel activity registered. The first run's four unsupported records remain unexplained; zero unsupported records here does not retrospectively diagnose them or establish all-key coverage.

The 5.6933 ms sample needs further characterization. Only the second injection call's API duration was retained by this report, so it cannot attribute the first sample's delay to SendInput versus later window dispatch. Per-submission I0/I1/A0 duration breakdowns were added after this run. No delay is subtracted from the reported end-to-end interval; scheduler/queueing effects remain part of it. Two samples provide no credible tail estimate or evidence that an input-path gate passed or failed.
