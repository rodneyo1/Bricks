use crate::auth::AuthError;
use serde::Serialize;
pub const MAX_POINTS: usize = 600;
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Sample {
    pub private_bytes: u64,
    pub working_set_bytes: u64,
    pub cpu_ns: u64,
    pub user_cpu_ns: u64,
    pub kernel_cpu_ns: u64,
}

#[derive(Serialize)]
pub struct Point {
    pub elapsed_ns: u64,
    pub sample: Sample,
}

/// Resource queries can block in Windows/accounting hooks. Keep them off the
/// packet/timer actor, including when measuring that actor's own process.
pub struct Monitor {
    stop: std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    worker: Option<std::thread::JoinHandle<Vec<Point>>>,
}
impl Monitor {
    pub fn start(origin: std::time::Instant) -> std::io::Result<Self> {
        let stop = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let signal = stop.clone();
        let worker = std::thread::Builder::new()
            .name("bricks-resources".into())
            .spawn(move || {
                let mut points = std::collections::VecDeque::with_capacity(MAX_POINTS);
                loop {
                    if let Ok(sample) = sample(std::process::id()) {
                        if points.len() == MAX_POINTS {
                            points.pop_front();
                        }
                        points.push_back(Point {
                            elapsed_ns: origin.elapsed().as_nanos() as u64,
                            sample,
                        });
                    }
                    let guard = signal.0.lock().unwrap();
                    let (guard, _) = signal
                        .1
                        .wait_timeout_while(guard, std::time::Duration::from_secs(1), |stopped| {
                            !*stopped
                        })
                        .unwrap();
                    if *guard {
                        break;
                    }
                }
                points.into_iter().collect()
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
    pub fn finish(mut self) -> Vec<Point> {
        self.signal();
        self.worker
            .take()
            .and_then(|worker| worker.join().ok())
            .unwrap_or_default()
    }
    fn signal(&self) {
        *self.stop.0.lock().unwrap() = true;
        self.stop.1.notify_one();
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.signal();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(windows)]
pub fn sample(pid: u32) -> Result<Sample, AuthError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::{
            ProcessStatus::{
                K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
            },
            Threading::{
                GetCurrentProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION,
                PROCESS_VM_READ,
            },
        },
    };
    // SAFETY: The process handle is checked/closed, initialized buffers have the
    // required sizes, and FILETIME integers are combined without pointer casts.
    unsafe {
        let is_current = pid == std::process::id();
        let handle = if is_current {
            GetCurrentProcess()
        } else {
            OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid)
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut memory: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        memory.cb = std::mem::size_of_val(&memory) as u32;
        let mut created: FILETIME = std::mem::zeroed();
        let mut ended = created;
        let mut kernel = created;
        let mut user = created;
        let ok = K32GetProcessMemoryInfo(
            handle,
            (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            memory.cb,
        ) != 0
            && GetProcessTimes(handle, &mut created, &mut ended, &mut kernel, &mut user) != 0;
        let error = std::io::Error::last_os_error();
        // The current-process pseudo handle is borrowed and must not be closed.
        if !is_current {
            CloseHandle(handle);
        }
        if !ok {
            return Err(error.into());
        }
        let ticks =
            |time: FILETIME| ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;
        Ok(Sample {
            private_bytes: memory.PrivateUsage as u64,
            working_set_bytes: memory.WorkingSetSize as u64,
            cpu_ns: (ticks(kernel) + ticks(user)) * 100,
            user_cpu_ns: ticks(user) * 100,
            kernel_cpu_ns: ticks(kernel) * 100,
        })
    }
}
#[cfg(not(windows))]
pub fn sample(_: u32) -> Result<Sample, AuthError> {
    Err("resource sampling currently requires Windows".into())
}
