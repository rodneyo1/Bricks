# First Slint desktop validation — 2026-10-23

Implemented the initial native product frontend and UI-independent controller in `bricks-ui` and `bricks-control`. Windows x64, Rust 1.99.1, Slint 1.18.1/winit/software renderer. Development/test builds use no Rust debug information or incremental compilation to reduce disk use. This is functional desktop validation, not an input latency benchmark.

- Workspace tests: 54 passed, including six new controller tests. Windows DPAPI/authenticated QUIC tests ran with the normal user profile and isolated stores.
- Formatting and Clippy with warnings denied: passed.
- Workspace/all-targets development build: passed.
- Rendered UI smoke: nine workflow checks passed: add, edit, settings, saved default port, invalid-input feedback, real recovery adapter, report export, forget and persistent final state. Six rendered screen captures were produced and the home/settings/dialog/share/About layouts were inspected. A transparent modal backdrop issue was corrected and the rendered smoke was rerun successfully.
- Native Windows accessibility test: actual Add PC / named input fields / Save / Edit controls added and edited an isolated saved PC; the persisted name and numeric endpoint matched the entered values. This uses UI Automation, without OS key injection. The first test lookup matched an adjacent text label; constraining field lookup to Edit controls resolved that test issue.
- Identity setup: repeated setup preserved identity bytes, exported only matching public DER certificates, created no peer trust, and refused to overwrite damaged identity metadata.
- State integrity: stale revisions, invalid values, duplicate endpoints, capacity, a second writer, corrupt/future/oversized data, and a staged-write failure preserved the previous authoritative state.

Latest rendered evidence: `artifacts/desktop-smoke-eeeeb11bbb9248638661034a5fd92ef4/`. Native-widget evidence: `artifacts/desktop-native-6e9ba55b644846bf9cc5504983b20f10/desktop.json`. Artifacts are generated local evidence and excluded from source control. Normal account device entries and identities were not created by these automated tests.

The app is packaged as `dist/BricksDesktop/Bricks.exe`, with notices and instructions; Start Bricks targets this frontend. The previous WinForms developer launcher remains separately available. A clean second-machine install, broad keyboard/screen-reader/high-DPI acceptance and signed installer validation are pending.

Packaged/source executable SHA-256 matched: `6c39a514c48d2adc5e06f442085e99d626823d41beec745b1cc69270cd237c03`. The packaged app launched successfully with window title Bricks; the Start Bricks shortcut target was verified. Local links and code fences passed for the root README and 19 top-level documentation files.

The initial Slint download encountered a full system disk. Regenerable artifacts were cleaned and development/test profiles reduced. After validation, generated compiler intermediates and symbol files were removed again, restoring approximately 2.01 GiB free while keeping the packaged app and host/client/fixture executables. The next Cargo build will recompile dependencies from its cache; no source, app data or benchmark evidence was removed.

No live remote OS control, pairing-code exchange, video/audio or independently running agent/IPC SDK was implemented by this slice. Those actions remain disabled or absent. Local diagnostic timings are labelled synthetic/recording-only and cannot establish gaming performance, one-way latency or superiority to Sunshine + Moonlight.
