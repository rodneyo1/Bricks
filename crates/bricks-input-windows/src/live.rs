//! Visible foreground-only v1 capture window. It forwards only after the user
//! clicks Start and an authenticated network worker reports ready.
use crate::{INJECTION_MARKER, mapping, normalize_keyboard};
use bricks_protocol::PhysicalKey;
use std::{
    io,
    mem::size_of,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{COLOR_WINDOW, ScreenToClient},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::{KeyboardAndMouse::*, *},
        WindowsAndMessaging::*,
    },
};

#[derive(Clone, Copy, Debug)]
pub enum CaptureEvent {
    Key {
        key: PhysicalKey,
        down: bool,
        captured_ns: u64,
    },
    Motion {
        delta: [i32; 4],
        captured_ns: u64,
    },
}
pub struct Signal {
    pub ready: AtomicBool,
    pub stopped: AtomicBool,
    pub close_on_stop: AtomicBool,
    status: Mutex<String>,
}
impl Default for Signal {
    fn default() -> Self {
        Self {
            ready: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            close_on_stop: AtomicBool::new(false),
            status: Mutex::new("Connecting to the verified host...".into()),
        }
    }
}
impl Signal {
    pub fn message(&self) -> String {
        self.status.lock().unwrap().clone()
    }
    pub fn status(&self, message: impl Into<String>) {
        *self.status.lock().unwrap() = message.into();
    }
    pub fn stop(&self, message: impl Into<String>) {
        self.status(message);
        self.stopped.store(true, Ordering::Release);
    }
}
/// Device-aware repeat ownership. A second keyboard's release cannot release a
/// key still held by the first. Removal terminates the epoch instead of guessing.
struct Pressed(Vec<(usize, PhysicalKey)>);
impl Pressed {
    fn note(&mut self, device: usize, key: PhysicalKey, down: bool) -> Result<Option<bool>, ()> {
        let existing = self.0.iter().position(|&(d, k)| d == device && k == key);
        if down {
            if existing.is_some() {
                return Ok(None);
            }
            if self.0.len() == 256 {
                return Err(());
            }
            let already = self.0.iter().any(|&(_, k)| k == key);
            self.0.push((device, key));
            Ok((!already).then_some(true))
        } else {
            let Some(index) = existing else {
                return Ok(None);
            };
            self.0.swap_remove(index);
            Ok((!self.0.iter().any(|&(_, k)| k == key)).then_some(false))
        }
    }
    fn key_down(&self, usage: u16) -> bool {
        self.0
            .iter()
            .any(|&(_, key)| key.usage_page == 7 && key.usage == usage)
    }
}
struct WindowState<'a> {
    signal: Arc<Signal>,
    origin: Instant,
    emit: Box<dyn FnMut(CaptureEvent) -> bool + 'a>,
    pressed: Pressed,
    armed: bool,
    status: HWND,
    last_status: String,
    events: u64,
    ignored: u64,
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
impl WindowState<'_> {
    fn send(&mut self, event: CaptureEvent) {
        if !(self.emit)(event) {
            self.signal
                .stop("Input queue full or connection closed. Session stopped.");
        } else {
            self.events += 1;
        }
    }
    fn key(&mut self, device: usize, key: PhysicalKey, down: bool, captured_ns: u64) {
        if down
            && key.usage_page == 7
            && key.usage == 0x45
            && [0xe0, 0xe2, 0xe1]
                .iter()
                .all(|&key| self.pressed.key_down(key) || self.pressed.key_down(key + 4))
        {
            self.signal.stop("Escape chord pressed. Input stopped.");
            return;
        }
        match self.pressed.note(device, key, down) {
            Ok(Some(down)) => self.send(CaptureEvent::Key {
                key,
                down,
                captured_ns,
            }),
            Ok(None) => {}
            Err(()) => self
                .signal
                .stop("Capture ownership limit reached. Session stopped."),
        }
    }
}
pub fn run(
    signal: Arc<Signal>,
    origin: Instant,
    emit: impl FnMut(CaptureEvent) -> bool,
) -> io::Result<serde_json::Value> {
    let mut state = Box::new(WindowState {
        signal: signal.clone(),
        origin,
        emit: Box::new(emit),
        pressed: Pressed(Vec::with_capacity(256)),
        armed: false,
        status: ptr::null_mut(),
        last_status: String::new(),
        events: 0,
        ignored: 0,
    });
    let class = wide("BricksLiveCaptureV1");
    // SAFETY: All window state is owned by this UI thread until destruction.
    // Registered Raw Input is foreground-only; there is no INPUTSINK or hook.
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
            wide("Bricks - live keyboard and mouse").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            820,
            360,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if hwnd.is_null() {
            UnregisterClassW(class.as_ptr(), instance);
            return Err(io::Error::last_os_error());
        }
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            (&mut *state as *mut WindowState<'_>) as isize,
        );
        state.status = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("Connecting...").as_ptr(),
            WS_CHILD | WS_VISIBLE,
            20,
            20,
            750,
            150,
            hwnd,
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        let start = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Start forwarding (F8)").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            20,
            200,
            250,
            44,
            hwnd,
            1usize as _,
            instance,
            ptr::null(),
        );
        let stop = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide("Stop and disconnect").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            300,
            200,
            220,
            44,
            hwnd,
            2usize as _,
            instance,
            ptr::null(),
        );
        let devices = [
            RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 6,
                dwFlags: RIDEV_DEVNOTIFY,
                hwndTarget: hwnd,
            },
            RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 2,
                dwFlags: RIDEV_DEVNOTIFY,
                hwndTarget: hwnd,
            },
        ];
        let registered = RegisterRawInputDevices(
            devices.as_ptr(),
            devices.len() as u32,
            size_of::<RAWINPUTDEVICE>() as u32,
        ) != 0;
        if !registered || state.status.is_null() || start.is_null() || stop.is_null() {
            let error = io::Error::last_os_error();
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
            return Err(error);
        }
        SetTimer(hwnd, 1, 100, None);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            if result <= 0 {
                if result < 0 {
                    signal.stop("Capture message pump failed.");
                }
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        let remove = [
            RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 6,
                dwFlags: RIDEV_REMOVE,
                hwndTarget: ptr::null_mut(),
            },
            RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 2,
                dwFlags: RIDEV_REMOVE,
                hwndTarget: ptr::null_mut(),
            },
        ];
        RegisterRawInputDevices(remove.as_ptr(), 2, size_of::<RAWINPUTDEVICE>() as u32);
        UnregisterClassW(class.as_ptr(), instance);
    }
    signal.stopped.store(true, Ordering::Release);
    Ok(
        serde_json::json!({"measurement_class":"windows_foreground_live_capture","forwarded_records":state.events,"ignored_marked_or_virtual_records":state.ignored,"status":signal.status.lock().unwrap().clone(),"limits":"Supported keyboard mappings, relative physical mice, five buttons and wheels; no background capture, global keyboard suppression or absolute tablet/touch input"}),
    )
}
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: State pointer is installed after CreateWindow, exclusively accessed
    // on this thread and removed during WM_DESTROY before the Box is dropped.
    unsafe {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState<'_>;
        if state.is_null() {
            return DefWindowProcW(hwnd, message, w, l);
        }
        let state = &mut *state;
        match message {
            WM_TIMER => {
                let status = state.signal.status.lock().unwrap().clone();
                // Session feedback only while forwarding: counter redraws would
                // compete with Raw Input on this message thread.
                let text = if state.armed {
                    format!(
                        "{status}\r\n\r\nCtrl+Alt+Shift+F12 stops forwarding. Switching windows also stops."
                    )
                } else {
                    format!(
                        "{status}\r\n\r\nCtrl+Alt+Shift+F12 stops forwarding. Switching windows also stops.\r\nForwarded records: {}",
                        state.events
                    )
                };
                if text != state.last_status {
                    SetWindowTextW(state.status, wide(&text).as_ptr());
                    state.last_status = text;
                }
                if state.signal.stopped.load(Ordering::Acquire) {
                    state.armed = false;
                    if state.signal.close_on_stop.load(Ordering::Acquire) {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                }
            }
            WM_COMMAND | WM_KEYDOWN => {
                let command = if message == WM_COMMAND {
                    w & 0xffff
                } else if w == usize::from(VK_F8)
                    && GetMessageExtraInfo() as usize != INJECTION_MARKER
                {
                    1
                } else {
                    0
                };
                if command == 1
                    && !state.armed
                    && state.signal.ready.load(Ordering::Acquire)
                    && !state.signal.stopped.load(Ordering::Acquire)
                {
                    state.armed = true;
                    state.signal.status("FORWARDING - keep this window focused. Switching windows ends the session.");
                }
                if message == WM_COMMAND && command == 2 {
                    state
                        .signal
                        .stop("Stopped by user. Reconnect to start a new session.");
                    state.armed = false;
                }
            }
            WM_ACTIVATEAPP if w == 0 => {
                if state.armed {
                    state
                        .signal
                        .stop("Focus lost. Input stopped; reconnect to resume.");
                    state.armed = false;
                }
            }
            WM_INPUT_DEVICE_CHANGE => {
                if state.armed {
                    state
                        .signal
                        .stop("Input device changed. Session stopped for cleanup.");
                }
            }
            WM_INPUT => {
                let captured_ns = state.origin.elapsed().as_nanos() as u64;
                if !state.armed || state.signal.stopped.load(Ordering::Acquire) {
                    return DefWindowProcW(hwnd, message, w, l);
                }
                if GetForegroundWindow() != hwnd
                    && GetAncestor(GetForegroundWindow(), GA_ROOT) != hwnd
                {
                    state.signal.stop("Focus lost. Input stopped.");
                    return DefWindowProcW(hwnd, message, w, l);
                }
                let mut input = RAWINPUT::default();
                let mut bytes = size_of::<RAWINPUT>() as u32;
                let count = GetRawInputData(
                    l as HRAWINPUT,
                    RID_INPUT,
                    (&mut input as *mut RAWINPUT).cast(),
                    &mut bytes,
                    size_of::<RAWINPUTHEADER>() as u32,
                );
                if count == u32::MAX || count < size_of::<RAWINPUTHEADER>() as u32 {
                    state.signal.stop("Raw Input read failed. Session stopped.");
                    return DefWindowProcW(hwnd, message, w, l);
                }
                if input.header.hDevice.is_null() {
                    state.ignored += 1;
                    return DefWindowProcW(hwnd, message, w, l);
                }
                match input.header.dwType {
                    RIM_TYPEKEYBOARD => {
                        let keyboard = input.data.keyboard;
                        if keyboard.ExtraInformation as usize == INJECTION_MARKER {
                            state.ignored += 1;
                        } else if let Ok(scan) =
                            normalize_keyboard(keyboard.MakeCode, keyboard.Flags)
                        {
                            if let Ok(key) = mapping::physical(scan) {
                                state.key(
                                    input.header.hDevice as usize,
                                    key,
                                    scan.down,
                                    captured_ns,
                                );
                            } else {
                                state
                                    .signal
                                    .stop("Unsupported keyboard scan code. Input stopped.");
                            }
                        } else {
                            state.signal.stop("Invalid keyboard record. Input stopped.");
                        }
                    }
                    RIM_TYPEMOUSE => {
                        let mouse = input.data.mouse;
                        if mouse.ulExtraInformation as usize == INJECTION_MARKER {
                            state.ignored += 1;
                        } else if mouse.usFlags & MOUSE_MOVE_ABSOLUTE != 0 {
                            state.signal.stop("Absolute pointing devices are unavailable in v1. Use a relative mouse.");
                        } else {
                            if mouse.lLastX != 0 || mouse.lLastY != 0 {
                                state.send(CaptureEvent::Motion {
                                    delta: [mouse.lLastX, mouse.lLastY, 0, 0],
                                    captured_ns,
                                });
                            }
                            let buttons = mouse.Anonymous.Anonymous;
                            let mut cursor = POINT::default();
                            GetCursorPos(&mut cursor);
                            ScreenToClient(hwnd, &mut cursor);
                            let hit = ChildWindowFromPoint(hwnd, cursor);
                            let local_control = hit != hwnd && matches!(GetDlgCtrlID(hit), 1 | 2);
                            for button in 0..5 {
                                for (offset, down) in [(0, true), (1, false)] {
                                    if buttons.usButtonFlags & (1 << (button * 2 + offset)) != 0
                                        && (!down || !local_control)
                                    {
                                        state.key(
                                            input.header.hDevice as usize,
                                            PhysicalKey {
                                                usage_page: 9,
                                                usage: button + 1,
                                            },
                                            down,
                                            captured_ns,
                                        );
                                    }
                                }
                            }
                            let wheel = i32::from(buttons.usButtonData as i16);
                            if buttons.usButtonFlags & 0x0400 != 0 && !local_control {
                                state.send(CaptureEvent::Motion {
                                    delta: [0, 0, wheel, 0],
                                    captured_ns,
                                });
                            }
                            if buttons.usButtonFlags & 0x0800 != 0 && !local_control {
                                state.send(CaptureEvent::Motion {
                                    delta: [0, 0, 0, wheel],
                                    captured_ns,
                                });
                            }
                        }
                    }
                    _ => state
                        .signal
                        .stop("Unexpected input device type. Session stopped."),
                }
                return DefWindowProcW(hwnd, message, w, l);
            }
            WM_DESTROY => {
                state.signal.stop("Capture window closed. Input stopped.");
                KillTimer(hwnd, 1);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                PostQuitMessage(0);
                return 0;
            }
            _ => {}
        }
        DefWindowProcW(hwnd, message, w, l)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeat_and_multi_device_releases_preserve_logical_ownership() {
        let mut pressed = Pressed(Vec::with_capacity(256));
        let key = PhysicalKey {
            usage_page: 7,
            usage: 4,
        };
        assert_eq!(pressed.note(1, key, true), Ok(Some(true)));
        assert_eq!(pressed.note(1, key, true), Ok(None));
        assert_eq!(pressed.note(2, key, true), Ok(None));
        assert_eq!(pressed.note(1, key, false), Ok(None));
        assert_eq!(pressed.note(2, key, false), Ok(Some(false)));
        assert_eq!(pressed.note(2, key, false), Ok(None));
    }
}
