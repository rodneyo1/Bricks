//! Benchmark-only idle-sleep request. Never enabled by the desktop or idle agent.
//! This does not override a user pressing Sleep, closing the lid, or OS policy.
pub struct KeepAwake {
    #[cfg(windows)]
    previous: u32,
    _same_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    // Documented Win32 EXECUTION_STATE is a DWORD; return value is previous state.
    fn SetThreadExecutionState(flags: u32) -> u32;
    fn GetCurrentProcess() -> *mut core::ffi::c_void;
    fn GetProcessInformation(
        process: *mut core::ffi::c_void,
        class: i32,
        state: *mut core::ffi::c_void,
        bytes: u32,
    ) -> i32;
    fn SetProcessInformation(
        process: *mut core::ffi::c_void,
        class: i32,
        state: *const core::ffi::c_void,
        bytes: u32,
    ) -> i32;
}

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PowerState {
    version: u32,
    control_mask: u32,
    state_mask: u32,
}

/// Keep active timer requests effective on Windows 11; HighQoS is a separate
/// opt-in. Restore the whole process policy afterward, without a power-plan edit.
pub struct ProcessPolicy {
    #[cfg(windows)]
    previous: Option<PowerState>,
    pub timer_override: bool,
}
impl ProcessPolicy {
    pub fn request(high_qos: bool) -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            let mut previous = PowerState {
                version: 1,
                control_mask: 0,
                state_mask: 0,
            };
            // SAFETY: Documented PROCESS_POWER_THROTTLING_STATE, class 4, 12-byte
            // buffer and borrowed current-process pseudo handle. No handle close.
            unsafe {
                if GetProcessInformation(
                    GetCurrentProcess(),
                    4,
                    (&mut previous as *mut PowerState).cast(),
                    12,
                ) == 0
                {
                    let error = std::io::Error::last_os_error();
                    if !high_qos && error.raw_os_error() == Some(87) {
                        return Ok(Self {
                            previous: None,
                            timer_override: false,
                        });
                    }
                    return Err(error);
                }
                // EXECUTION_SPEED=1, IGNORE_TIMER_RESOLUTION=4. Controlling the
                // latter with its state bit clear honors active timeBeginPeriod.
                let mask = 4 | u32::from(high_qos);
                let state = PowerState {
                    control_mask: previous.control_mask | mask,
                    state_mask: previous.state_mask & !mask,
                    ..previous
                };
                if SetProcessInformation(
                    GetCurrentProcess(),
                    4,
                    (&state as *const PowerState).cast(),
                    12,
                ) == 0
                {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() != Some(87) {
                        return Err(error);
                    }
                    // Older Windows builds do not implement the timer bit.
                    if !high_qos {
                        return Ok(Self {
                            previous: None,
                            timer_override: false,
                        });
                    }
                    let fallback = PowerState {
                        control_mask: previous.control_mask | 1,
                        state_mask: previous.state_mask & !1,
                        ..previous
                    };
                    if SetProcessInformation(
                        GetCurrentProcess(),
                        4,
                        (&fallback as *const PowerState).cast(),
                        12,
                    ) == 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    return Ok(Self {
                        previous: Some(previous),
                        timer_override: false,
                    });
                }
            }
            Ok(Self {
                previous: Some(previous),
                timer_override: true,
            })
        }
        #[cfg(not(windows))]
        {
            if high_qos {
                Err(std::io::Error::other("HighQoS experiment requires Windows"))
            } else {
                Ok(Self {
                    timer_override: false,
                })
            }
        }
    }
}
impl Drop for ProcessPolicy {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: Restore the exact process power-policy state saved at entry.
        unsafe {
            if let Some(previous) = &self.previous {
                SetProcessInformation(
                    GetCurrentProcess(),
                    4,
                    (previous as *const PowerState).cast(),
                    12,
                );
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn process_timer_policy_is_explicit_and_restored() {
        let read = || {
            let mut state = PowerState {
                version: 1,
                control_mask: 0,
                state_mask: 0,
            };
            // SAFETY: Initialized documented 12-byte current-process policy.
            let ok = unsafe {
                GetProcessInformation(
                    GetCurrentProcess(),
                    4,
                    (&mut state as *mut PowerState).cast(),
                    12,
                )
            };
            (ok != 0).then_some(state)
        };
        let before = read();
        {
            let guard = ProcessPolicy::request(false).unwrap();
            if guard.timer_override {
                let active = read().unwrap();
                assert_eq!(active.control_mask & 4, 4);
                assert_eq!(active.state_mask & 4, 0);
            }
        }
        assert_eq!(read(), before);
    }
    #[test]
    fn idle_sleep_request_restores_own_thread_state_after_unwinding() {
        std::thread::spawn(|| {
            // SAFETY: This fresh test-only thread owns no other power requests.
            let original = unsafe { SetThreadExecutionState(0x8000_0000) };
            assert_ne!(original, 0);
            let outcome = std::panic::catch_unwind(|| {
                let _guard = KeepAwake::request().unwrap();
                // Setting the same state returns the active prior request.
                let active = unsafe { SetThreadExecutionState(0x8000_0001) };
                assert_ne!(active & 1, 0);
                panic!("exercise scoped power cleanup");
            });
            assert!(outcome.is_err());
            let restored = unsafe { SetThreadExecutionState(0x8000_0000) };
            assert_eq!(restored, original | 0x8000_0000);
        })
        .join()
        .unwrap();
    }
}
impl KeepAwake {
    pub fn request() -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            // ES_CONTINUOUS | ES_SYSTEM_REQUIRED. No display, away-mode or
            // permanent power-scheme change. The guard stays on this thread.
            let previous = unsafe { SetThreadExecutionState(0x8000_0001) };
            if previous == 0 {
                return Err(std::io::Error::other(
                    "Windows rejected the benchmark idle-sleep request",
                ));
            }
            Ok(Self {
                previous,
                _same_thread: std::marker::PhantomData,
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                _same_thread: std::marker::PhantomData,
            })
        }
    }
}
impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: Restore the execution state returned on this same thread.
        unsafe {
            SetThreadExecutionState(self.previous | 0x8000_0000);
        }
    }
}
