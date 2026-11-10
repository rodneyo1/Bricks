//! Foreground-only interactive diagnostic. No remote listener or background capture.
use crate::{INJECTION_MARKER, Prefix, ScanTransition, input_record, mapping};
use serde_json::{Value, json};
use std::{io, mem::size_of, ptr, time::Instant};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{COLOR_WINDOW, InvalidateRect},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::{KeyboardAndMouse::*, *},
        WindowsAndMessaging::*,
    },
};

const RUN_TEST: u32 = WM_APP + 1;
const UPDATE_STATUS: u32 = WM_APP + 2;
const TEST_BUTTON: usize = 100;
const STATUS_TIMER: usize = 1;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[derive(Default)]
struct Counts {
    raw_keys: u64,
    unsupported_keys: u64,
    unsupported_reasons: std::collections::BTreeMap<&'static str, u64>,
    unsupported_codes: std::collections::BTreeMap<(u16, u16, u16), u64>,
    unsupported_code_drops: u64,
    raw_mouse: u64,
    relative_x: i64,
    relative_y: i64,
    button_records: u64,
    wheel_units: i64,
    absolute_records: u64,
    receipt_keys: u64,
    focus_losses: u64,
    read_errors: u64,
}
struct State {
    diagnose_unmapped: bool,
    status_window: HWND,
    notice: &'static str,
    start: Instant,
    counts: Counts,
    submissions: u64,
    failures: u64,
    possible_down: bool,
    pending: Option<Instant>,
    // (submission id, I0->A0, I0->I1), captured together for each marked receipt.
    latencies: Vec<(u64, u64, Option<u64>)>,
    trace_drops: u64,
    last_api_duration_ns: Option<u64>,
}
impl State {
    fn new() -> Self {
        Self {
            diagnose_unmapped: false,
            status_window: ptr::null_mut(),
            notice: "Ready. Click Run test or press F8 (Fn+F8 on some laptops).",
            start: Instant::now(),
            counts: Counts::default(),
            submissions: 0,
            failures: 0,
            possible_down: false,
            pending: None,
            latencies: Vec::with_capacity(1024),
            trace_drops: 0,
            last_api_duration_ns: None,
        }
    }
    fn record_unsupported(&mut self, scan: u16, flags: u16, virtual_key: u16) {
        if let Some(reason) = mapping::unsupported_reason(scan, flags) {
            self.counts.unsupported_keys += 1;
            *self.counts.unsupported_reasons.entry(reason).or_default() += 1;
            if self.diagnose_unmapped {
                let key = (scan, flags, virtual_key);
                if self.counts.unsupported_codes.contains_key(&key)
                    || self.counts.unsupported_codes.len() < 64
                {
                    *self.counts.unsupported_codes.entry(key).or_default() += 1;
                } else {
                    self.counts.unsupported_code_drops += 1;
                }
            }
        }
    }
    fn report(&self) -> Value {
        json!({
            "report_schema_version": 2, "measurement_class": "local_interactive_fixture",
            "fixture_revision": "deferred-feedback-v1",
            "feedback_policy": "defer status rendering until receipt, or 250 ms pending timeout",
            "duration_ns": self.start.elapsed().as_nanos(),
            "raw_keyboard_records": self.counts.raw_keys, "unsupported_keyboard_records": self.counts.unsupported_keys,
            "unsupported_keyboard_reasons": self.counts.unsupported_reasons,
            "unmapped_key_diagnostics_enabled": self.diagnose_unmapped,
            "unsupported_key_codes": self.counts.unsupported_codes.iter().map(|(&(scan,flags,vkey),&count)| json!({
                "make_code_hex": format!("0x{scan:04x}"), "raw_flags_hex": format!("0x{flags:04x}"),
                "virtual_key_hex": format!("0x{vkey:04x}"), "count": count
            })).collect::<Vec<_>>(),
            "unsupported_code_drops": self.counts.unsupported_code_drops,
            "raw_mouse_records": self.counts.raw_mouse,
            "relative_totals": [self.counts.relative_x,self.counts.relative_y],
            "mouse_button_records": self.counts.button_records, "vertical_wheel_units": self.counts.wheel_units,
            "absolute_mouse_records_not_accumulated": self.counts.absolute_records,
            "legacy_keyboard_receipts": self.counts.receipt_keys,
            "focus_losses": self.counts.focus_losses, "raw_read_errors": self.counts.read_errors,
            "explicit_test_submissions": self.submissions, "submission_or_cleanup_failures": self.failures,
            "unresolved_possible_test_key_down": self.possible_down,
            "submit_to_keydown_receipt_ns": self.latencies.iter().map(|s| s.1).collect::<Vec<_>>(),
            "receipt_timing_samples": self.latencies.iter().map(|s| json!({
                "submission_id": s.0,
                "submit_to_keydown_receipt_ns": s.1,
                "submit_to_api_return_ns": s.2,
                "api_return_to_keydown_receipt_ns": s.2.and_then(|api| s.1.checked_sub(api))
            })).collect::<Vec<_>>(),
            "latency_sample_drops": self.trace_drops,
            "last_submit_to_api_return_ns": self.last_api_duration_ns,
            "awaiting_marked_receipt_at_exit": self.pending.is_some(),
            "boundary": "I0 immediately before SendInput to A0 handling marked WM_KEYDOWN",
            "uncertainty": "uncalibrated local Instant; scheduler/message-loop delays included",
            "limitations": ["Not capture-to-submit, two-PC, game acceptance, or input-to-photon",
                "Basic WM_INPUT reader; buffered high-rate draining still pending",
                "Aggregate capture only; no physical-key timestamps or typed text saved"]
        })
    }
}

/// Run only on an interactive desktop. The caller explicitly opts into opening a
/// visible fixture. F8 is a separate user action authorizing the synthetic test.
pub fn run() -> io::Result<Value> {
    run_with_diagnostics(false)
}

/// Explicit diagnostic opt-in: aggregate unsupported key identifiers only.
pub fn run_with_diagnostics(diagnose_unmapped: bool) -> io::Result<Value> {
    let mut state = Box::new(State::new());
    state.diagnose_unmapped = diagnose_unmapped;
    if diagnose_unmapped {
        state.notice =
            "UNMAPPED KEY DIAGNOSTICS ON - unsupported code counts will be printed at exit.";
    }
    let class: Vec<u16> = "BricksInputFixture\0".encode_utf16().collect();
    let title: Vec<u16> = "Bricks input test - F8 or click Run test\0"
        .encode_utf16()
        .collect();
    // SAFETY: all Win32 pointers refer to live strings/structs for the call duration.
    // State is owned here until the window and message loop are destroyed.
    unsafe {
        let instance = GetModuleHandleW(ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW as usize + 1) as _,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            850,
            330,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if hwnd.is_null() {
            UnregisterClassW(class.as_ptr(), instance);
            return Err(io::Error::last_os_error());
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *state as *mut State) as isize);
        let button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Run test (F8)").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            24,
            24,
            220,
            46,
            hwnd,
            TEST_BUTTON as _,
            instance,
            ptr::null(),
        );
        let status = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            ptr::null(),
            WS_CHILD | WS_VISIBLE,
            24,
            88,
            770,
            170,
            hwnd,
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if button.is_null() || status.is_null() {
            let error = io::Error::last_os_error();
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
            return Err(error);
        }
        state.status_window = status;
        PostMessageW(hwnd, UPDATE_STATUS, 0, 0);
        // No INPUTSINK/NOLEGACY: capture only while this application is foreground,
        // retain ordinary messages to observe actual application receipt.
        let devices = [
            RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 6,
                dwFlags: 0,
                hwndTarget: hwnd,
            },
            RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 2,
                dwFlags: 0,
                hwndTarget: hwnd,
            },
        ];
        if RegisterRawInputDevices(devices.as_ptr(), 2, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
            let error = io::Error::last_os_error();
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
            return Err(error);
        }
        let mut msg = MSG::default();
        let mut loop_error = None;
        loop {
            let result = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
            if result == -1 {
                loop_error = Some(io::Error::last_os_error());
                break;
            }
            if result == 0 {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        let remove = devices.map(|d| RAWINPUTDEVICE {
            dwFlags: RIDEV_REMOVE,
            hwndTarget: ptr::null_mut(),
            ..d
        });
        RegisterRawInputDevices(remove.as_ptr(), 2, size_of::<RAWINPUTDEVICE>() as u32);
        UnregisterClassW(class.as_ptr(), instance);
        if let Some(error) = loop_error {
            return Err(error);
        }
    }
    Ok(state.report())
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA is set to the stable boxed State by run, on the same
    // UI thread; never dereferenced until installed. No panicking operations or
    // recursive message dispatch occur while this mutable reference is held.
    unsafe {
        // Restore parent keyboard focus before scheduling a button-triggered test.
        // Do this before borrowing State: SetFocus can dispatch messages reentrantly.
        if message == WM_COMMAND && w & 0xffff == TEST_BUTTON && (w >> 16) == 0 {
            SetFocus(hwnd);
            PostMessageW(hwnd, RUN_TEST, 0, 0);
            return 0;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, message, w, l);
        }
        if message == WM_TIMER && w == STATUS_TIMER {
            KillTimer(hwnd, STATUS_TIMER);
            PostMessageW(hwnd, UPDATE_STATUS, 0, 0);
            return 0;
        }
        if message == UPDATE_STATUS {
            // Posted messages can precede queued keyboard input. Do not format or
            // redraw feedback inside the measured I0->A0 interval. If receipt never
            // comes, allow the pending status to become visible after 250 ms.
            if (&*pointer)
                .pending
                .is_some_and(|start| start.elapsed().as_millis() < 250)
            {
                SetTimer(hwnd, STATUS_TIMER, 250, None);
                return 0;
            }
            KillTimer(hwnd, STATUS_TIMER);
            // Snapshot first; hold no State reference across control-window calls.
            let (status_window, text) = {
                let state = &*pointer;
                let last = state
                    .latencies
                    .last()
                    .map(|s| format!("Last confirmed receipt: {:.3} ms", s.1 as f64 / 1_000_000.0))
                    .unwrap_or_else(|| "No confirmed test yet.".into());
                (
                    state.status_window,
                    wide(&format!(
                        "{}\r\n\r\nTests sent: {}    Confirmed receipts: {}    Failures: {}\r\n{}\r\n{}\r\n\r\nKeep this window focused. Escape closes it and prints the report.",
                        state.notice,
                        state.submissions,
                        state.latencies.len() as u64 + state.trace_drops,
                        state.failures,
                        last,
                        if state.diagnose_unmapped {
                            "UNMAPPED KEY DIAGNOSTICS ON - aggregate unsupported codes will be printed."
                        } else {
                            "Unmapped key identifiers are not being recorded."
                        }
                    )),
                )
            };
            SetWindowTextW(status_window, text.as_ptr());
            InvalidateRect(status_window, ptr::null(), 1);
            return 0;
        }
        let state = &mut *pointer;
        match message {
            RUN_TEST => {
                trigger_test(hwnd, state);
                PostMessageW(hwnd, UPDATE_STATUS, 0, 0);
                return 0;
            }
            WM_INPUT => {
                let captured = Instant::now();
                let mut raw = RAWINPUT::default();
                let mut bytes = size_of::<RAWINPUT>() as u32;
                let count = GetRawInputData(
                    l as HRAWINPUT,
                    RID_INPUT,
                    (&mut raw as *mut RAWINPUT).cast(),
                    &mut bytes,
                    size_of::<RAWINPUTHEADER>() as u32,
                );
                if count == u32::MAX || count < size_of::<RAWINPUTHEADER>() as u32 {
                    state.counts.read_errors += 1;
                } else if GetForegroundWindow() == hwnd {
                    match raw.header.dwType {
                        RIM_TYPEKEYBOARD => {
                            let key = raw.data.keyboard;
                            if key.ExtraInformation != INJECTION_MARKER as u32 {
                                state.counts.raw_keys += 1;
                                state.record_unsupported(key.MakeCode, key.Flags, key.VKey);
                            }
                        }
                        RIM_TYPEMOUSE => {
                            let mouse = raw.data.mouse;
                            state.counts.raw_mouse += 1;
                            if mouse.usFlags & MOUSE_MOVE_ABSOLUTE == 0 {
                                state.counts.relative_x += i64::from(mouse.lLastX);
                                state.counts.relative_y += i64::from(mouse.lLastY);
                            } else {
                                state.counts.absolute_records += 1;
                            }
                            let buttons = mouse.Anonymous.Anonymous;
                            if buttons.usButtonFlags & 0x03ff != 0 {
                                state.counts.button_records += 1;
                            }
                            if buttons.usButtonFlags & 0x0400 != 0 {
                                state.counts.wheel_units += i64::from(buttons.usButtonData as i16);
                            }
                        }
                        _ => {}
                    }
                }
                let _ = captured; // C0 boundary; individual capture traces intentionally not retained.
                return DefWindowProcW(hwnd, message, w, l); // required foreground raw-input cleanup
            }
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                let receipt = Instant::now();
                state.counts.receipt_keys += 1;
                if message == WM_KEYDOWN
                    && GetMessageExtraInfo() as usize == INJECTION_MARKER
                    && w == 0x41
                {
                    if let Some(submitted) = state.pending.take() {
                        if state.latencies.len() < 1024 {
                            state.latencies.push((
                                state.submissions,
                                receipt.duration_since(submitted).as_nanos() as u64,
                                state.last_api_duration_ns,
                            ));
                        } else {
                            state.trace_drops += 1;
                        }
                        state.notice = "SUCCESS - test key received by this window.";
                        PostMessageW(hwnd, UPDATE_STATUS, 0, 0);
                    }
                } else if message == WM_KEYDOWN && l & (1 << 30) == 0 {
                    if w == usize::from(VK_ESCAPE) {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                    if w == usize::from(VK_F8) {
                        trigger_test(hwnd, state);
                        PostMessageW(hwnd, UPDATE_STATUS, 0, 0);
                    }
                }
            }
            WM_KILLFOCUS => {
                if IsChild(hwnd, w as HWND) != 0 {
                    return 0;
                }
                state.counts.focus_losses += 1;
                state.pending = None;
                cleanup(state);
                state.notice = "Focus lost. Click Run test to resume here.";
                PostMessageW(hwnd, UPDATE_STATUS, 0, 0);
            }
            WM_DESTROY => {
                KillTimer(hwnd, STATUS_TIMER);
                cleanup(state);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                PostQuitMessage(0);
                return 0;
            }
            _ => {}
        }
        DefWindowProcW(hwnd, message, w, l)
    }
}

unsafe fn trigger_test(hwnd: HWND, state: &mut State) {
    // SAFETY: invoked only by this fixture's UI thread with its live State/window.
    // Input submission is the explicit user-requested test, never an automatic loop.
    unsafe {
        if GetForegroundWindow() != hwnd {
            state.notice = "Test requested, but this window is not foreground.";
            return;
        }
        if GetAsyncKeyState(0x41) < 0 {
            state.notice = "Test requested - release the A key first.";
            return;
        }
        if state.pending.is_some() {
            state.notice = "Test requested - still waiting for the previous receipt.";
            return;
        }
        let down = ScanTransition {
            make_code: 0x1e,
            prefix: Prefix::None,
            down: true,
        };
        if let (Ok(a), Ok(b)) = (
            input_record(down),
            input_record(ScanTransition {
                down: false,
                ..down
            }),
        ) {
            state.possible_down = true;
            let records = [a, b];
            SetLastError(0);
            state.last_api_duration_ns = None;
            let submitted = Instant::now();
            state.pending = Some(submitted);
            let inserted = SendInput(2, records.as_ptr(), size_of::<INPUT>() as i32);
            state.last_api_duration_ns = Some(submitted.elapsed().as_nanos() as u64);
            state.submissions += 1;
            if inserted == 2 {
                state.possible_down = false;
                state.notice = "Test sent - waiting for the window to receive it.";
            } else {
                state.failures += 1;
                state.pending = None;
                cleanup(state);
                state.notice = "FAILED - Windows did not accept the complete test key pair.";
            }
        }
    }
}
unsafe fn cleanup(state: &mut State) {
    if state.possible_down {
        let release = ScanTransition {
            make_code: 0x1e,
            prefix: Prefix::None,
            down: false,
        };
        if crate::submit_scancode(release).is_ok() {
            state.possible_down = false;
        } else {
            state.failures += 1;
        }
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn unsupported_identifiers_are_not_collected_without_opt_in() {
        let mut state = State::new();
        state.record_unsupported(0x7f, 0, 0xff);
        assert_eq!(state.counts.unsupported_keys, 1);
        assert!(state.counts.unsupported_codes.is_empty());
    }
    #[test]
    fn diagnostic_groups_identifiers_and_caps_unique_records() {
        let mut state = State::new();
        state.diagnose_unmapped = true;
        state.record_unsupported(0x7f, 0, 0xff);
        state.record_unsupported(0x7f, 0, 0xff);
        assert_eq!(state.counts.unsupported_codes[&(0x7f, 0, 0xff)], 2);
        for key in 0..100 {
            state.record_unsupported(0, key + 8, 0);
        }
        assert_eq!(state.counts.unsupported_codes.len(), 64);
        assert_eq!(state.counts.unsupported_code_drops, 37);
    }
}
