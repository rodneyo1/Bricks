//! Per-session Windows x64 crash protection. The input path only publishes an
//! atomic owned-state bitmap; no per-event pipe roundtrip or disk write occurs.
//! Unnamed handles are duplicated into our own child over its private stdin.
use crate::{InputError, Prefix, ScanTransition, WindowsInjector, key_record, mapping};
use bricks_core::{Injector, motion::MotionInjector};
use bricks_protocol::PhysicalKey;
use std::{
    cell::RefCell,
    io::{self, Read, Write},
    mem::size_of,
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
        process::CommandExt,
    },
    path::Path,
    process::{Child, Command, Stdio},
    ptr,
    rc::Rc,
    sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Memory::*, SystemInformation::GetTickCount64, Threading::*},
    UI::WindowsAndMessaging::GetForegroundWindow,
};

const MAGIC: u64 = 0x314c_4e52_4a4b_5242;
const ACTIVE: u32 = 1;
const STOP: u32 = 2;
#[repr(C, align(64))]
struct Shared {
    magic: u64,
    state: AtomicU32,
    heartbeat: AtomicU64,
    watcher_heartbeat: AtomicU64,
    target: AtomicUsize,
    keys: [AtomicU64; 5],
    failures: AtomicU32,
}
struct Mapping {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}
impl Mapping {
    fn shared(&self) -> &Shared {
        // SAFETY: Our unnamed mapping is exactly Shared, initialized before the
        // child receives its handle, and remains mapped for this object's life.
        unsafe { &*self.view.Value.cast::<Shared>() }
    }
    fn create() -> io::Result<Self> {
        // SAFETY: Anonymous pagefile-backed shared memory. No inheritable or
        // named object is exposed. The parent is its sole initializing writer.
        unsafe {
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                ptr::null(),
                PAGE_READWRITE,
                0,
                size_of::<Shared>() as u32,
                ptr::null(),
            );
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mapping = Self::open(handle)?;
            ptr::write(
                mapping.view.Value.cast::<Shared>(),
                Shared {
                    magic: MAGIC,
                    state: AtomicU32::new(0),
                    heartbeat: AtomicU64::new(GetTickCount64()),
                    watcher_heartbeat: AtomicU64::new(0),
                    target: AtomicUsize::new(0),
                    keys: std::array::from_fn(|_| AtomicU64::new(0)),
                    failures: AtomicU32::new(0),
                },
            );
            Ok(mapping)
        }
    }
    unsafe fn open(handle: HANDLE) -> io::Result<Self> {
        // SAFETY: Handle is created here or duplicated by our parent. Mapping
        // size/alignment is checked by Windows; protocol magic is checked next.
        unsafe {
            let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size_of::<Shared>());
            if view.Value.is_null() {
                let error = io::Error::last_os_error();
                CloseHandle(handle);
                return Err(error);
            }
            Ok(Self { handle, view })
        }
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.view);
            CloseHandle(self.handle);
        }
    }
}
pub struct Guard {
    mapping: Mapping,
    child: RefCell<Child>,
    _exclusive: std::fs::File,
}
impl Guard {
    pub fn start(executable: &Path, report: Option<&Path>) -> io::Result<Rc<Self>> {
        // One OS-input owner per Windows account, independent of identity,
        // address and port. The child retains a duplicated handle until cleanup
        // ends, so parent death cannot admit a replacement before release.
        let base = std::path::PathBuf::from(
            std::env::var_os("LOCALAPPDATA")
                .ok_or_else(|| io::Error::other("LOCALAPPDATA unavailable"))?,
        )
        .join("Bricks");
        std::fs::create_dir_all(&base)?;
        let until = Instant::now() + Duration::from_millis(300);
        let exclusive = loop {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(base.join("live-input.lock"))
            {
                Ok(file) => break file,
                Err(error) if error.raw_os_error() == Some(32) && Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("Another live-input owner or cleanup is active: {error}"),
                    ));
                }
            }
        };
        let mapping = Mapping::create()?;
        let mut command = Command::new(executable);
        command
            .arg("--internal-input-watchdog")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000);
        if let Some(report) = report {
            command.arg(report);
        }
        let mut child = command.spawn()?;
        let setup = (|| -> io::Result<()> {
            let mut mapped = ptr::null_mut();
            let mut parent = ptr::null_mut();
            let mut owner_lock = ptr::null_mut();
            // SAFETY: Duplicates only our mapping and our own process into the
            // just-created child. No PID lookup, arbitrary process or ACL bypass.
            unsafe {
                if DuplicateHandle(
                    GetCurrentProcess(),
                    mapping.handle,
                    child.as_raw_handle(),
                    &mut mapped,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                ) == 0
                    || DuplicateHandle(
                        GetCurrentProcess(),
                        GetCurrentProcess(),
                        child.as_raw_handle(),
                        &mut parent,
                        PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
                        0,
                        0,
                    ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                if DuplicateHandle(
                    GetCurrentProcess(),
                    exclusive.as_raw_handle(),
                    child.as_raw_handle(),
                    &mut owner_lock,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                ) == 0
                {
                    return Err(io::Error::last_os_error());
                }
            }
            let mut input = child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("missing watchdog startup pipe"))?;
            input.write_all(b"BJRN0002")?;
            input.write_all(&(mapped as usize as u64).to_le_bytes())?;
            input.write_all(&(parent as usize as u64).to_le_bytes())?;
            input.write_all(&(owner_lock as usize as u64).to_le_bytes())?;
            drop(input);
            let mut output = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("missing watchdog ready pipe"))?;
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let mut ready = [0; 5];
                let result = output.read_exact(&mut ready).and_then(|_| {
                    if &ready == b"READY" {
                        Ok(())
                    } else {
                        Err(io::Error::other("invalid watchdog startup"))
                    }
                });
                let _ = tx.send(result);
            });
            rx.recv_timeout(Duration::from_secs(3))
                .map_err(|_| io::Error::other("watchdog did not become ready"))??;
            Ok(())
        })();
        if let Err(error) = setup {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Rc::new(Self {
            mapping,
            child: RefCell::new(child),
            _exclusive: exclusive,
        }))
    }
    pub fn pulse(&self) -> Result<(), InputError> {
        let now = unsafe { GetTickCount64() };
        let shared = self.mapping.shared();
        if shared.state.load(Ordering::Acquire) == ACTIVE
            && now.saturating_sub(shared.heartbeat.load(Ordering::Acquire)) >= 250
        {
            return Err(InputError::Protection("input actor heartbeat expired"));
        }
        if !self
            .child
            .borrow_mut()
            .try_wait()
            .ok()
            .is_some_and(|status| status.is_none())
        {
            return Err(InputError::Protection("watchdog exited"));
        }
        self.mapping
            .shared()
            .heartbeat
            .store(unsafe { GetTickCount64() }, Ordering::Release);
        self.mapping.shared().state.store(ACTIVE, Ordering::Release);
        self.check()
    }
    pub fn check(&self) -> Result<(), InputError> {
        let shared = self.mapping.shared();
        let now = unsafe { GetTickCount64() };
        if shared.state.load(Ordering::Acquire) != ACTIVE
            || now.saturating_sub(shared.heartbeat.load(Ordering::Acquire)) >= 250
            || now.saturating_sub(shared.watcher_heartbeat.load(Ordering::Acquire)) >= 250
        {
            return Err(InputError::Protection(
                "watchdog unavailable or session inactive",
            ));
        }
        let target = shared.target.load(Ordering::Acquire);
        if target != 0 && unsafe { GetForegroundWindow() } as usize != target {
            return Err(InputError::Protection("host focus changed"));
        }
        Ok(())
    }
    fn bind_target(&self) -> Result<(), InputError> {
        self.check()?;
        let foreground = unsafe { GetForegroundWindow() } as usize;
        if foreground == 0 {
            return Err(InputError::Protection("no foreground target"));
        }
        let target = self.mapping.shared().target.load(Ordering::Acquire);
        if target == 0 {
            self.mapping
                .shared()
                .target
                .store(foreground, Ordering::Release);
        } else if target != foreground {
            return Err(InputError::Protection("host focus changed"));
        }
        Ok(())
    }
    pub fn owned(&self) -> u32 {
        self.mapping
            .shared()
            .keys
            .iter()
            .map(|bits| bits.load(Ordering::Acquire).count_ones())
            .sum()
    }
    pub fn child_id(&self) -> u32 {
        self.child.borrow().id()
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        // No future down is possible: the last Rc belongs to the stopped actor.
        release_owned(self.mapping.shared());
        self.mapping.shared().state.store(STOP, Ordering::Release);
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until {
            if self.child.get_mut().try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.owned() != 0 {
            return;
        } // A living watcher retains the journal and owner lock while retrying failed releases.
        let _ = self.child.get_mut().kill();
        let _ = self.child.get_mut().wait();
    }
}
pub struct SupervisedInjector(pub Rc<Guard>);
fn index(key: PhysicalKey) -> Result<usize, InputError> {
    if key.usage_page == 9 && (1..=5).contains(&key.usage) {
        return Ok(255 + usize::from(key.usage));
    }
    let scan = mapping::scancode(key, true)?;
    Ok(usize::from(scan.make_code) + if scan.prefix == Prefix::E0 { 128 } else { 0 })
}
impl Injector for SupervisedInjector {
    type Error = InputError;
    fn submit(&mut self, key: PhysicalKey, down: bool) -> Result<(), InputError> {
        let slot = index(key)?;
        let record = key_record(key, down)?;
        let bits = &self.0.mapping.shared().keys[slot / 64];
        let mask = 1u64 << (slot % 64);
        if down {
            self.0.bind_target()?;
            let vk = if key.usage_page == 9 {
                match key.usage {
                    1 => 1,
                    2 => 2,
                    3 => 4,
                    4 => 5,
                    5 => 6,
                    _ => 0,
                }
            } else {
                let scan = mapping::scancode(key, true)?;
                // MAPVK_VSC_TO_VK_EX preserves left/right modifier identity.
                unsafe {
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyW(
                        u32::from(scan.make_code)
                            | if scan.prefix == Prefix::E0 { 0xe000 } else { 0 },
                        3,
                    )
                }
            };
            if vk != 0
                && unsafe {
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk as i32)
                } < 0
            {
                return Err(InputError::AlreadyDown);
            }
            bits.fetch_or(mask, Ordering::SeqCst);
        } else if bits.load(Ordering::Acquire) & mask == 0 {
            // A rejected down never reached SendInput. Do not release a host's
            // pre-existing physical hold while cleaning that rejected request.
            return Ok(());
        }
        crate::send_records(&[record])?;
        if !down {
            bits.fetch_and(!mask, Ordering::Release);
        }
        Ok(())
    }
}
impl MotionInjector for SupervisedInjector {
    fn submit_motion(&mut self, delta: [i32; 4]) -> Result<(), InputError> {
        self.0.bind_target()?;
        WindowsInjector.submit_motion(delta)
    }
}
fn release_owned(shared: &Shared) -> u32 {
    let mut failures = 0;
    for (word, bits) in shared.keys.iter().enumerate() {
        let pending = bits.load(Ordering::Acquire);
        for bit in 0..64 {
            let mask = 1u64 << bit;
            if pending & mask == 0 {
                continue;
            }
            let slot = word * 64 + bit;
            let result = if slot < 256 {
                crate::submit_scancode(ScanTransition {
                    make_code: (slot % 128) as u16,
                    prefix: if slot >= 128 {
                        Prefix::E0
                    } else {
                        Prefix::None
                    },
                    down: false,
                })
            } else {
                WindowsInjector.submit(
                    PhysicalKey {
                        usage_page: 9,
                        usage: (slot - 255) as u16,
                    },
                    false,
                )
            };
            if result.is_ok() {
                bits.fetch_and(!mask, Ordering::Release);
            } else {
                failures += 1;
            }
        }
    }
    let _ = shared
        .failures
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            Some(value.saturating_add(failures))
        });
    failures
}
pub fn run(report: Option<&Path>) -> io::Result<()> {
    let mut startup = [0; 32];
    io::stdin().read_exact(&mut startup)?;
    if &startup[..8] != b"BJRN0002" {
        return Err(io::Error::other("invalid internal watchdog startup"));
    }
    let mapping = unsafe {
        Mapping::open(u64::from_le_bytes(startup[8..16].try_into().unwrap()) as usize as HANDLE)
    }?;
    let parent = u64::from_le_bytes(startup[16..24].try_into().unwrap()) as usize as HANDLE;
    let _exclusive = unsafe {
        std::fs::File::from_raw_handle(
            u64::from_le_bytes(startup[24..].try_into().unwrap()) as usize as HANDLE
        )
    };
    if mapping.shared().magic != MAGIC {
        unsafe {
            CloseHandle(parent);
        };
        return Err(io::Error::other("invalid input journal"));
    }
    mapping
        .shared()
        .watcher_heartbeat
        .store(unsafe { GetTickCount64() }, Ordering::Release);
    io::stdout().write_all(b"READY")?;
    io::stdout().flush()?;
    let reason;
    let started = Instant::now();
    loop {
        let shared = mapping.shared();
        let now = unsafe { GetTickCount64() };
        shared.watcher_heartbeat.store(now, Ordering::Release);
        let wait = unsafe { WaitForSingleObject(parent, 10) };
        if wait == WAIT_OBJECT_0 {
            reason = "host_process_exit";
            break;
        }
        if wait == WAIT_FAILED {
            unsafe {
                CloseHandle(parent);
            };
            return Err(io::Error::last_os_error());
        }
        if shared.state.load(Ordering::Acquire) == STOP {
            reason = "normal_stop";
            break;
        }
        if shared.state.load(Ordering::Acquire) != ACTIVE {
            continue;
        }
        let target = shared.target.load(Ordering::Acquire);
        let focus_lost = target != 0 && unsafe { GetForegroundWindow() } as usize != target;
        let escape = unsafe {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
            GetAsyncKeyState(0x7b) < 0
                && [0x10, 0x11, 0x12]
                    .iter()
                    .all(|&key| GetAsyncKeyState(key) < 0)
        };
        if focus_lost
            || escape
            || now.saturating_sub(shared.heartbeat.load(Ordering::Acquire)) >= 250
        {
            reason = if escape {
                "host_escape_chord"
            } else if focus_lost {
                "host_focus_loss"
            } else {
                "host_unresponsive"
            };
            // Fence the old injector before releasing state. Otherwise a stalled
            // actor could resume and submit an unowned down after our cleanup.
            if unsafe { TerminateProcess(parent, 70) } == 0
                && unsafe { WaitForSingleObject(parent, 0) } != WAIT_OBJECT_0
            {
                unsafe {
                    CloseHandle(parent);
                };
                return Err(io::Error::last_os_error());
            }
            while unsafe { WaitForSingleObject(parent, 50) } != WAIT_OBJECT_0 {
                std::thread::sleep(Duration::from_millis(10));
            }
            break;
        }
    }
    let cleanup_start = Instant::now();
    let before: u32 = mapping
        .shared()
        .keys
        .iter()
        .map(|bits| bits.load(Ordering::Acquire).count_ones())
        .sum();
    let mut last_reported = None;
    let remaining = loop {
        release_owned(mapping.shared());
        let remaining: u32 = mapping
            .shared()
            .keys
            .iter()
            .map(|bits| bits.load(Ordering::Acquire).count_ones())
            .sum();
        if remaining == 0 {
            break remaining;
        }
        if cleanup_start.elapsed() >= Duration::from_millis(250) && last_reported != Some(remaining)
        {
            if let Some(report) = report {
                let result = serde_json::json!({"schema_version":1,"status":"retrying_cleanup","reason":reason,"owned_before":before,"unresolved_owned":remaining,"release_failures":mapping.shared().failures.load(Ordering::Relaxed)});
                if let Ok(bytes) = serde_json::to_vec_pretty(&result) {
                    let _ = std::fs::write(report, bytes);
                }
            }
            last_reported = Some(remaining);
        }
        std::thread::sleep(Duration::from_millis(
            if cleanup_start.elapsed() >= Duration::from_millis(250) {
                100
            } else {
                10
            },
        ));
    };
    unsafe {
        CloseHandle(parent);
    }
    if let Some(report) = report {
        let result = serde_json::json!({"schema_version":1,"status":"complete","measurement_class":"windows_crash_cleanup_api_submission","reason":reason,"watcher_lifetime_ns":started.elapsed().as_nanos(),"cleanup_ns":cleanup_start.elapsed().as_nanos(),"owned_before":before,"unresolved_owned":remaining,"release_failures":mapping.shared().failures.load(Ordering::Relaxed),"limitations":"SendInput result and shared journal only; verify marked application receipts externally. Simultaneous loss of both processes and OS failure are outside this watchdog."});
        std::fs::write(report, serde_json::to_vec_pretty(&result)?)?;
    }
    if remaining != 0 {
        return Err(io::Error::other("input cleanup remains unresolved"));
    }
    Ok(())
}
