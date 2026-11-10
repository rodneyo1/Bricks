# Windows desktop UI verification

Executed on the development PC on 2026-10-23. This is functional UI/controller evidence, not a performance benchmark or a two-PC result.

| Check | Result |
| --- | --- |
| Build | Framework compiler, x64, optimization enabled, warnings treated as errors |
| Packaged entry point | `dist/Bricks/Bricks.exe` version 0.1.0.0 opened successfully and was left ready for the user |
| Controller assertions | 43 passed |
| GUI integration assertions | 13 passed |
| UI automatic recovery check | PASS, 12 simulated scenarios |
| UI automatic local connection check | PASS, 200 recording-only transitions |
| UI Host path | Actual authenticated QUIC loopback connection, 200 transitions; successful GUI result |
| UI Client path | Actual authenticated QUIC loopback connection, 200 transitions; successful GUI result |
| GUI cancellation | Waiting host exits and releases its listening socket |
| Fixture launch/cancel | Owned fixture opens, receives graceful close, UI reports CANCELLED |
| OS keys injected by automation | None |
| Production identities modified by tests | None; test-only identities in isolated GUID-named workspace profile |

The controller tests run actual child processes with empty, quoted, Unicode, whitespace, trailing-backslash and shell-metacharacter arguments and verify exact argv preservation. They exercise concurrent 200,000-character stdout/stderr streams, output overflow, timeout, cancellation, nonzero exit, and rejection of force-kill timeout for the interactive fixture. They also cover numeric IPv4/IPv6/tunnel addresses, port/address rejection, allowed tool paths, UI settings round-trip/schema validation, certificate size bounds, fingerprint display, owned work-file cleanup, incomplete/failed reports, and generated role configurations accepted by the real Rust parser.

GUI tests instantiate the same window as the packaged app, invoke its actual buttons, check visible status labels, and verify that the UI event loop continues ticking while checks run. They exercise both connection roles with only newly generated, test-owned DPAPI identities pretrusted inside the test setup. The production file-picker/fingerprint confirmation is not bypassed by any runtime flag. Those manual dialogs still need user acceptance testing. Role switching clears the prepared-identity state; setup must load the selected role's identity before a connection is enabled.

The suite writes `artifacts/ui-unit-results.json`, `artifacts/ui-integration-results.json`, and window renders `artifacts/ui-home.png`, `artifacts/ui-checks.png`, `artifacts/ui-connection.png` (generated/git-ignored). Screenshots were inspected and button clipping/focus-induced scrolling corrected. The final bundle is `dist/Bricks`, with the workspace shortcut `Start Bricks.lnk`. Source/build steps are documented in [desktop UI](../desktop-ui.md).

A separate launch check verified the packaged production executable's actual window title and version; its result is `artifacts/ui-launch-result.json`. Startup was idle: no capture or listening session was automatically started.

GUI integration was run with normal desktop/profile access for DPAPI, as required by the existing identity subsystem. The UI/controller tests use a separate executable with internal path injection to isolate state; the shipped app exposes no test-root/trust-bypass CLI. Before recursive removal, the test harness checks the canonical profile target is beneath `workspace/target/ui-tests/profiles` and has the expected GUID basename. It never recursively removes a production identity directory.

The Rust input/transport implementation and its locked dependencies are unchanged by this UI addition. No new capture, OS injection, loss recovery on live QUIC, media or installer feature is implied. Remaining acceptance: real user-triggered fixture receipts through this launcher, file dialogs and fingerprint comparison, screen readers and multiple DPI/monitor combinations, a clean second-PC setup, physical LAN/tunnel testing, user usability study, signed packaging and physical gaming latency.
