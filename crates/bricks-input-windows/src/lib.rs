//! Windows adapters and opt-in foreground interactive fixture.
//! Normalization is independently testable; tests never submit OS input.

use thiserror::Error;

#[cfg(windows)]
pub mod fixture;
#[cfg(windows)]
pub mod live;
pub mod mapping;
#[cfg(all(windows, target_pointer_width = "64"))]
pub mod watchdog;

/// Checked host adapter. Live callers must authenticate and grant ownership first.
#[cfg(windows)]
pub struct WindowsInjector;

#[cfg(windows)]
impl bricks_core::Injector for WindowsInjector {
    type Error = InputError;
    fn submit(&mut self, key: bricks_protocol::PhysicalKey, down: bool) -> Result<(), Self::Error> {
        send_records(&[key_record(key, down)?])
    }
}
#[cfg(windows)]
impl bricks_core::motion::MotionInjector for WindowsInjector {
    fn submit_motion(&mut self, delta: [i32; 4]) -> Result<(), InputError> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        let mut records = [INPUT::default(); 3];
        let mut count = 0;
        if delta[0] != 0 || delta[1] != 0 {
            records[count] = mouse_record(
                delta[0],
                delta[1],
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_MOVE_NOCOALESCE,
            );
            count += 1;
        }
        for (value, flag) in [
            (delta[2], MOUSEEVENTF_WHEEL),
            (delta[3], MOUSEEVENTF_HWHEEL),
        ] {
            if value != 0 {
                records[count] = mouse_record(0, 0, value as u32, flag);
                count += 1;
            }
        }
        if count == 0 {
            return Ok(());
        }
        send_records(&records[..count])
    }
}
#[cfg(windows)]
fn mouse_record(
    x: i32,
    y: i32,
    data: u32,
    flags: u32,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: x,
                dy: y,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTION_MARKER,
            },
        },
    }
}
#[cfg(windows)]
pub fn key_record(
    key: bricks_protocol::PhysicalKey,
    down: bool,
) -> Result<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT, InputError> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    if key.usage_page == 7 {
        return input_record(mapping::scancode(key, down)?);
    }
    if key.usage_page != 9 {
        return Err(InputError::InvalidRecord);
    }
    let (flag, data) = match (key.usage, down) {
        (1, true) => (MOUSEEVENTF_LEFTDOWN, 0),
        (1, false) => (MOUSEEVENTF_LEFTUP, 0),
        (2, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (2, false) => (MOUSEEVENTF_RIGHTUP, 0),
        (3, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (3, false) => (MOUSEEVENTF_MIDDLEUP, 0),
        (4 | 5, true) => (MOUSEEVENTF_XDOWN, u32::from(key.usage - 3)),
        (4 | 5, false) => (MOUSEEVENTF_XUP, u32::from(key.usage - 3)),
        _ => return Err(InputError::InvalidRecord),
    };
    Ok(mouse_record(0, 0, data, flag))
}

pub const INJECTION_MARKER: usize = 0x4252_4b53;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Prefix {
    None,
    E0,
    E1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanTransition {
    pub make_code: u16,
    pub prefix: Prefix,
    pub down: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum InputError {
    #[error("live input protection stopped: {0}")]
    Protection(&'static str),
    #[error("host key/button is already down locally; release it before sharing")]
    AlreadyDown,
    #[error("unsupported or invalid raw keyboard record")]
    InvalidRecord,
    #[error("E1/Pause scan-code injection needs an explicit mapping; not implemented")]
    UnsupportedE1,
    #[error(
        "SendInput inserted no event (OS error {0}); privilege restrictions may not be reported"
    )]
    Submission(u32),
}

/// Decode RAWKEYBOARD make/break and prefix flags without text translation.
/// A zero make code needs a separate mapping path and is rejected explicitly.
pub fn normalize_keyboard(make_code: u16, flags: u16) -> Result<ScanTransition, InputError> {
    if make_code == 0 || make_code > 0x7f || flags & !7 != 0 || flags & 6 == 6 {
        return Err(InputError::InvalidRecord);
    }
    let prefix = match flags & 6 {
        2 => Prefix::E0,
        4 => Prefix::E1,
        _ => Prefix::None,
    };
    Ok(ScanTransition {
        make_code,
        prefix,
        down: flags & 1 == 0,
    })
}

#[cfg(windows)]
fn input_record(
    event: ScanTransition,
) -> Result<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT, InputError> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    if event.make_code == 0 || event.make_code > 0x7f {
        return Err(InputError::InvalidRecord);
    }
    if event.prefix == Prefix::E1 {
        return Err(InputError::UnsupportedE1);
    }
    let mut flags = KEYEVENTF_SCANCODE;
    if event.prefix == Prefix::E0 {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !event.down {
        flags |= KEYEVENTF_KEYUP;
    }
    Ok(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: event.make_code,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTION_MARKER,
            },
        },
    })
}

/// Explicit OS submission. No CLI invokes this in M0 and tests do not inject input.
/// The future interactive agent must own/clean up every successfully submitted down.
#[cfg(windows)]
pub fn submit_scancode(event: ScanTransition) -> Result<(), InputError> {
    send_records(&[input_record(event)?])
}
#[cfg(windows)]
fn send_records(
    inputs: &[windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT],
) -> Result<(), InputError> {
    use windows_sys::Win32::{
        Foundation::{GetLastError, SetLastError},
        UI::Input::KeyboardAndMouse::SendInput,
    };
    // SAFETY: clearing this thread's diagnostic error has no pointer preconditions.
    unsafe {
        SetLastError(0);
    }
    // SAFETY: input is a fully initialized INPUT with the keyboard union variant;
    // its pointer and the exact structure size remain valid for the synchronous call.
    let inserted = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>() as i32,
        )
    };
    if inserted != inputs.len() as u32 {
        // SAFETY: GetLastError has no pointer or lifetime preconditions.
        return Err(InputError::Submission(unsafe { GetLastError() }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_make_break_and_extended_identity() {
        let down = normalize_keyboard(0x1d, 2).unwrap();
        let up = normalize_keyboard(0x1d, 3).unwrap();
        assert_eq!(down.prefix, Prefix::E0);
        assert!(down.down);
        assert!(!up.down);
        assert_eq!(down.make_code, up.make_code);
        assert_eq!(normalize_keyboard(0x45, 4).unwrap().prefix, Prefix::E1);
    }

    #[test]
    fn rejects_overrun_missing_codes_and_contradictory_prefixes() {
        for (code, flags) in [(0, 0), (0xff, 0), (0x1e, 6), (0x1e, 8)] {
            assert_eq!(
                normalize_keyboard(code, flags),
                Err(InputError::InvalidRecord)
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn builds_discrete_scancode_record_without_os_injection() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
        let record = input_record(normalize_keyboard(0x1d, 3).unwrap()).unwrap();
        assert_eq!(record.r#type, INPUT_KEYBOARD);
        // SAFETY: input_record constructs and selects the keyboard union variant.
        let key = unsafe { record.Anonymous.ki };
        assert_eq!(key.wScan, 0x1d);
        assert_eq!(key.wVk, 0);
        assert_eq!(
            key.dwFlags,
            KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP
        );
        assert_eq!(key.dwExtraInfo, INJECTION_MARKER);
        assert!(matches!(
            input_record(normalize_keyboard(0x45, 4).unwrap()),
            Err(InputError::UnsupportedE1)
        ));
    }
}
