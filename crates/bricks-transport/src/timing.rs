//! Scoped timer-resolution request for active paced lab/session work only.
//! This improves wait precision, not QPC accuracy, and is never a timing guarantee.
pub struct ActiveTimerResolution;
impl ActiveTimerResolution {
    pub fn request() -> Result<Self, crate::auth::AuthError> {
        #[cfg(windows)]
        // SAFETY: A successful process timer request is paired in Drop, including
        // error/cancellation paths. No persistent system configuration is changed.
        if unsafe { windows_sys::Win32::Media::timeBeginPeriod(1) } != 0 {
            return Err("Windows rejected the active 1 ms timer-resolution request".into());
        }
        Ok(Self)
    }
}
impl Drop for ActiveTimerResolution {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: Exactly matches the successful request made by this instance.
        unsafe {
            windows_sys::Win32::Media::timeEndPeriod(1);
        }
    }
}

/// Optional Windows Games task registration. This is scoped to the current
/// thread (never process-wide realtime priority), and is reverted on that thread.
pub struct GamesTask {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    _same_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl GamesTask {
    pub fn request() -> Result<Self, crate::auth::AuthError> {
        #[cfg(windows)]
        {
            let mut index = 0;
            // SAFETY: NUL-terminated immutable UTF-16 task name; index is writable.
            let handle = unsafe {
                windows_sys::Win32::System::Threading::AvSetMmThreadCharacteristicsW(
                    windows_sys::core::w!("Games"),
                    &mut index,
                )
            };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Self {
                handle,
                _same_thread: std::marker::PhantomData,
            })
        }
        #[cfg(not(windows))]
        {
            Err("Games scheduling requires Windows MMCSS".into())
        }
    }
}
impl Drop for GamesTask {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: PhantomData<Rc> prevents moving this registration to another
        // thread; callers drive the lab on a current-thread runtime.
        unsafe {
            windows_sys::Win32::System::Threading::AvRevertMmThreadCharacteristics(self.handle);
        }
    }
}
