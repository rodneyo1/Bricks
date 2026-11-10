# First user-run Windows fixture

Source: JSON pasted by the user on 2026-10-23 after running the debug fixture. This is user-supplied interactive evidence, not an automated agent run. OS/hardware inventory and instrument calibration were not provided.

Measurement class: `local_interactive_fixture`. Boundary: immediately before SendInput (I0) to handling the marked WM_KEYDOWN in the same window (A0). The two samples are 1,195,300 ns (1.1953 ms) and 1,016,300 ns (1.0163 ms). Last measured submission-to-API-return duration is 578,600 ns (0.5786 ms).

| Observation | Reported value |
| --- | --- |
| Run duration | 23.4502737 seconds |
| Explicit F8 submissions / marked keydown samples | 2 / 2 |
| Submission or cleanup failures | 0 |
| Unresolved possible test-key down | false |
| Pending marked receipt at exit | false |
| Raw-read errors / latency sample drops | 0 / 0 |
| Raw keyboard records / unsupported records | 11 / 4 |
| Legacy keyboard receipts | 15 |
| Raw mouse records / button records | 215 / 6 |
| Relative X/Y totals | -459 / -497 |
| Absolute mouse records | 0 |
| Vertical wheel units | 0 |
| Focus losses | 1 |

This confirms both explicitly requested test submissions produced a marked keydown receipt and that keyboard/mouse records reached the fixture. Cleanup reported no unresolved test-key state. It does not independently verify every physical release or multi-key behavior. Raw and legacy record counts are different event streams and cannot be equated to infer lost physical events.

Four unsupported keyboard records remain unexplained. Aggregate reason counters were added after this run; no cause is inferred from the old count alone. Re-run the updated fixture to distinguish unsupported mappings, E1 sequences, missing/overrun scan codes, or unexpected flags without recording typed text. Wheel behavior, full modifier/rollover coverage, load tests, and device-specific compatibility remain unverified.

Two uncalibrated debug-mode samples do not establish percentiles, M1 latency gates, two-PC performance, game acceptance, or input-to-photon results.
