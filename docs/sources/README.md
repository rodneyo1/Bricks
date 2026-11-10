# Source material and evidence

Reviewed on 2026-10-23. Primary references support API/protocol facts, not Bricks performance predictions. Live documentation and repository branches can change; implementation benchmarks must record actual tested versions/commits.

## Preserved attachments

| File | Original role | Caveat |
| --- | --- | --- |
| [original-design.txt](original-design.txt) | Input-focused agent proposal | Randomized release timing is rejected in the reviewed design |
| [original-targets-and-benchmarks.txt](original-targets-and-benchmarks.txt) | Source constants, latency targets, fitting tests | Referenced JavaScript was not supplied; constants/provenance unverified |
| [original-language-and-tooling.txt](original-language-and-tooling.txt) | Rust/tooling recommendation | General language rankings are not evidence of latency performance |

These files are byte-preserved copies of the pasted attachments, including any encoding artifacts and incomplete endings. Reviewed documents are newly written; no missing upstream content has been invented.

[consolidated-feedback.txt](consolidated-feedback.txt) preserves the final external review and amended milestone list. Its accepted sequencing is reflected in the roadmap. The timing policy resolves its remaining open decision; precise timestamp markers replace its ambiguous “submit” terminology. The consolidated LAN numbers are initial gates; the previously stricter targets remain optimization goals.

## Primary references consulted

- [Microsoft Raw Input overview](https://learn.microsoft.com/en-us/windows/win32/inputdev/about-raw-input): device-oriented capture model.
- [Microsoft SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput): injection API, insertion count, integrity restrictions.
- [Microsoft MOUSEINPUT](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-mouseinput): relative/absolute motion semantics and pointer settings.
- [Microsoft high-resolution timestamps](https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps): QPC, local intervals, synchronization limits.
- [Linux uinput](https://docs.kernel.org/input/uinput.html): userspace virtual input devices.
- [Quinn connection API](https://docs.rs/quinn/latest/quinn/struct.Connection.html): datagrams and streams.
- [RFC 9221](https://www.rfc-editor.org/rfc/rfc9221.html): unreliable QUIC datagrams and congestion/security context.
- [Tailscale connection types](https://tailscale.com/docs/reference/connection-types): direct versus relay paths.
- [Moonlight FAQ](https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions): latency statistic boundaries and external measurement.
- [Moonlight input source](https://github.com/moonlight-stream/moonlight-common-c/blob/master/src/InputStream.c): upstream code to inspect at a pinned revision when building a fair baseline; no source-level speedup established by this review.
- [Sunshine configuration](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2configuration.html): comparison settings and input controls; freeze actual version before testing.
- [DXGI Desktop Duplication](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nn-dxgi1_2-idxgioutputduplication): candidate capture interface.
- [NVIDIA NVENC programming guide](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.0/nvenc-video-encoder-api-prog-guide/index.html): candidate hardware encode interface and configuration.
- [Rust 2024 edition guide](https://doc.rust-lang.org/edition-guide/rust-2024/index.html): edition introduction and compiler baseline.
- [evdev uinput module](https://docs.rs/evdev/latest/evdev/uinput/index.html): existing Rust virtual-device wrapper.

Configuration schemas, setup UX, milestone choices, recovery defaults, and numerical acceptance thresholds are Bricks design proposals. They are not claims made by these references.
