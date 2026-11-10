//! Explicit visible OS cleanup verification. Never invoked by cargo test, a
//! listener, normal startup or an untrusted peer. Synthetic source; real TLS,
//! input submission, marked application receipts and external crash observation.
use crate::{auth::AuthError, probe, recovery_lab::EphemeralPair, reliable};
use bricks_input_windows::{
    INJECTION_MARKER,
    live::{CaptureEvent, Signal},
};
use bricks_protocol::{
    Epoch, PhysicalKey, Transition,
    live::{Critical, Motion, Totals},
};
use std::{
    fs, io,
    mem::size_of,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    ptr,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicIsize, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::ClientToScreen,
    System::{Diagnostics::ToolHelp::*, Threading::*},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const ROOT_FOCUS: u32 = WM_APP + 40;
const OTHER_FOCUS: u32 = WM_APP + 41;
static ROOT: AtomicUsize = AtomicUsize::new(0);
static OTHER: AtomicUsize = AtomicUsize::new(0);
static OLD: AtomicIsize = AtomicIsize::new(0);
static OTHER_OLD: AtomicIsize = AtomicIsize::new(0);
struct Observed {
    origin: Instant,
    key_down: AtomicU64,
    key_up: AtomicU64,
    mouse_down: AtomicU64,
    mouse_up: AtomicU64,
    last_key_up: AtomicU64,
    last_mouse_up: AtomicU64,
}
static OBSERVED: OnceLock<Arc<Observed>> = OnceLock::new();
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
unsafe fn activate(hwnd: HWND) {
    // SAFETY: Only our own two visible fixture windows are passed here. Sharing
    // queues temporarily enables ordinary desktop test automation, then detaches.
    unsafe {
        let foreground = GetForegroundWindow();
        let foreground_thread = GetWindowThreadProcessId(foreground, ptr::null_mut());
        let own = GetCurrentThreadId();
        let attached = foreground_thread != 0
            && foreground_thread != own
            && AttachThreadInput(own, foreground_thread, 1) != 0;
        ShowWindow(hwnd, SW_RESTORE);
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        if attached {
            AttachThreadInput(own, foreground_thread, 0);
        }
    }
}
unsafe extern "system" fn observe(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: Subclassed only our own windows, retaining their original WNDPROCs.
    unsafe {
        if message == ROOT_FOCUS {
            activate(hwnd);
            return 0;
        }
        if message == OTHER_FOCUS {
            let mut other = OTHER.load(Ordering::Acquire) as HWND;
            if other.is_null() {
                other = CreateWindowExW(
                    0,
                    wide("STATIC").as_ptr(),
                    wide("Bricks cleanup focus target").as_ptr(),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    100,
                    100,
                    500,
                    250,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                if other.is_null() {
                    return 0;
                }
                OTHER.store(other as usize, Ordering::Release);
                OTHER_OLD.store(GetWindowLongPtrW(other, GWLP_WNDPROC), Ordering::Release);
                SetWindowLongPtrW(other, GWLP_WNDPROC, observe as *const () as isize);
            }
            activate(other);
            return 0;
        }
        if GetMessageExtraInfo() as usize == INJECTION_MARKER
            && let Some(observed) = OBSERVED.get()
        {
            let ns = observed.origin.elapsed().as_nanos() as u64;
            match message {
                WM_KEYDOWN | WM_SYSKEYDOWN if ((l >> 16) & 0xff) == 0x36 => {
                    observed.key_down.fetch_add(1, Ordering::Release);
                }
                WM_KEYUP | WM_SYSKEYUP if ((l >> 16) & 0xff) == 0x36 => {
                    observed.key_up.fetch_add(1, Ordering::Release);
                    observed.last_key_up.store(ns, Ordering::Release);
                }
                WM_MBUTTONDOWN => {
                    observed.mouse_down.fetch_add(1, Ordering::Release);
                }
                WM_MBUTTONUP => {
                    observed.mouse_up.fetch_add(1, Ordering::Release);
                    observed.last_mouse_up.store(ns, Ordering::Release);
                }
                _ => {}
            }
        }
        if message == WM_CLOSE && hwnd as usize == ROOT.load(Ordering::Acquire) {
            let other = OTHER.swap(0, Ordering::AcqRel) as HWND;
            if !other.is_null() {
                DestroyWindow(other);
            }
        }
        let original = if hwnd as usize == ROOT.load(Ordering::Acquire) {
            OLD.load(Ordering::Acquire)
        } else {
            OTHER_OLD.load(Ordering::Acquire)
        };
        if original != 0 {
            CallWindowProcW(
                std::mem::transmute::<isize, WNDPROC>(original),
                hwnd,
                message,
                w,
                l,
            )
        } else {
            DefWindowProcW(hwnd, message, w, l)
        }
    }
}
unsafe extern "system" fn find_own(hwnd: HWND, parameter: LPARAM) -> i32 {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == std::process::id() && IsWindowVisible(hwnd) != 0 {
            *(parameter as *mut usize) = hwnd as usize;
            return 0;
        }
        1
    }
}
struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn ensure_fixture() -> Result<(), AuthError> {
    let root = ROOT.load(Ordering::Acquire) as HWND;
    if root.is_null() || unsafe { GetForegroundWindow() } != root {
        return Err("Cleanup check needs its own visible foreground window; no input was submitted outside it".into());
    }
    Ok(())
}
async fn wait_for(
    mut condition: impl FnMut() -> bool,
    connection: Option<(&quinn::Connection, Epoch)>,
    duration: Duration,
) -> Result<(), AuthError> {
    let until = Instant::now() + duration;
    loop {
        if condition() {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err("cleanup fixture observation timed out".into());
        }
        if let Some((connection, epoch)) = connection {
            let _ = connection.send_datagram(
                Motion {
                    epoch,
                    segment: 2,
                    captured_ns: 0,
                    totals: Totals::default(),
                }
                .encode(true)?
                .to_vec()
                .into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
fn terminate_owned_watcher(pid: u32, parent: u32, executable: &Path) -> Result<(), AuthError> {
    // SAFETY: Open then verify the held process handle against a fresh process
    // snapshot and exact executable path. Only our host child's watcher may die.
    unsafe {
        let process = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if process.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let checked = (|| -> Result<(), AuthError> {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error().into());
            }
            let mut entry = PROCESSENTRY32W {
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut found = false;
            if Process32FirstW(snapshot, &mut entry) != 0 {
                loop {
                    if entry.th32ProcessID == pid {
                        found = entry.th32ParentProcessID == parent;
                        break;
                    }
                    if Process32NextW(snapshot, &mut entry) == 0 {
                        break;
                    }
                }
            }
            CloseHandle(snapshot);
            if !found {
                return Err("watchdog is not a child of this test's host".into());
            }
            let mut buffer = [0u16; 32768];
            let mut length = buffer.len() as u32;
            if QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) == 0 {
                return Err(io::Error::last_os_error().into());
            }
            let path = PathBuf::from(String::from_utf16(&buffer[..length as usize])?);
            if path.canonicalize()? != executable.canonicalize()? {
                return Err("watchdog executable mismatch".into());
            }
            if TerminateProcess(process, 71) == 0 {
                return Err(io::Error::last_os_error().into());
            }
            if WaitForSingleObject(process, 1000) != WAIT_OBJECT_0 {
                return Err("watchdog termination was not confirmed".into());
            }
            Ok(())
        })();
        CloseHandle(process);
        checked
    }
}
fn suspend_owned_threads(pid: u32) -> Result<(), AuthError> {
    // Only threads belonging to our still-owned Child process are suspended.
    // Its separate watchdog must remain runnable and fence/clean the injector.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error().into());
        }
        let mut entry = THREADENTRY32 {
            dwSize: size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut suspended = 0;
        if Thread32First(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(
                        THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                        0,
                        entry.th32ThreadID,
                    );
                    if !thread.is_null() {
                        if GetProcessIdOfThread(thread) == pid && SuspendThread(thread) != u32::MAX
                        {
                            suspended += 1;
                        }
                        CloseHandle(thread);
                    }
                }
                if Thread32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        if suspended == 0 {
            return Err("test host had no suspendable threads".into());
        }
        Ok(())
    }
}
async fn phases(
    output: &Path,
    ui_signal: Arc<Signal>,
) -> Result<Vec<serde_json::Value>, AuthError> {
    let pair = EphemeralPair::new()?;
    let executable = std::env::current_exe()?;
    let observed = OBSERVED.get().unwrap();
    let mut results = Vec::new();
    for scenario in [
        "disconnect",
        "host_process_crash",
        "host_process_hang",
        "watchdog_process_crash",
        "host_focus_loss",
        "client_focus_loss",
    ] {
        ui_signal.status(format!("VERIFYING {scenario}: synthetic right Shift + middle mouse hold. Please leave the fixture in focus."));
        unsafe {
            PostMessageW(ROOT.load(Ordering::Acquire) as HWND, ROOT_FOCUS, 0, 0);
        }
        wait_for(
            || unsafe { GetForegroundWindow() } as usize == ROOT.load(Ordering::Acquire),
            None,
            Duration::from_secs(2),
        )
        .await?;
        ensure_fixture()?;
        if unsafe { GetAsyncKeyState(i32::from(VK_RSHIFT)) } < 0
            || unsafe { GetAsyncKeyState(i32::from(VK_MBUTTON)) } < 0
        {
            return Err(
                "Release right Shift and the middle mouse button before the explicit cleanup check"
                    .into(),
            );
        }
        // Keep every synthetic mouse event inside our bare fixture client area.
        unsafe {
            let hwnd = ROOT.load(Ordering::Acquire) as HWND;
            let mut rect = RECT::default();
            GetClientRect(hwnd, &mut rect);
            let mut point = POINT {
                x: rect.right - 35,
                y: rect.bottom - 35,
            };
            ClientToScreen(hwnd, &mut point);
            SetCursorPos(point.x, point.y);
        }
        let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
        let remote = socket.local_addr()?;
        drop(socket);
        let config_text = include_str!("../../../docs/examples/host.toml")
            .replace("127.0.0.1:47990", &remote.to_string());
        let config_path = output.join(format!("{scenario}.toml"));
        fs::write(&config_path, &config_text)?;
        let log_path = output.join(format!("{scenario}.log"));
        let report = output.join(format!("{scenario}.json"));
        let mut host = Worker(
            Command::new(&executable)
                .args(["live-host", "--config"])
                .arg(&config_path)
                .arg("--identity-dir")
                .arg(pair.directory().join("host"))
                .args([
                    "--peer",
                    &pair.client_fp,
                    "--approve-live",
                    "--owned-session",
                    "--report",
                ])
                .arg(&report)
                .stdin(Stdio::piped())
                .stdout(fs::File::create(&log_path)?)
                .stderr(Stdio::from(fs::File::create(
                    output.join(format!("{scenario}-stderr.log")),
                )?))
                .creation_flags(0x0800_0000)
                .spawn()?,
        );
        let endpoint = probe::client_endpoint(&pair.client, &pair.host_fp, remote)?;
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if fs::read_to_string(&log_path)
                .unwrap_or_default()
                .contains("listening on")
            {
                break;
            }
            if host.0.try_wait()?.is_some() || Instant::now() >= deadline {
                return Err("fixture host did not start listening".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let connection = tokio::time::timeout(
            Duration::from_secs(4),
            endpoint.connect(remote, "bricks-host")?,
        )
        .await??;
        let (epoch, _) = reliable::negotiate_client_protocol(&connection, b"BRKLIV01").await?;
        let kd = observed.key_down.load(Ordering::Acquire);
        let ku = observed.key_up.load(Ordering::Acquire);
        let md = observed.mouse_down.load(Ordering::Acquire);
        let mu = observed.mouse_up.load(Ordering::Acquire);
        ensure_fixture()?;
        if scenario == "client_focus_loss" {
            // Reconnect through the actual client loop and its capture window's
            // real WM_ACTIVATEAPP handler for this final phase. The source below
            // is synthetic; physical Raw Input is never fabricated or claimed.
            connection.close(0u32.into(), b"switch to client-loop fixture");
            host.0.wait()?;
            drop(host);
            if report.exists() {
                fs::rename(&report, output.join("client_focus_loss-setup.json"))?;
            }
            let watchdog_setup = report.with_extension("watchdog.json");
            if watchdog_setup.exists() {
                fs::rename(
                    watchdog_setup,
                    output.join("client_focus_loss-setup.watchdog.json"),
                )?;
            }
            fs::copy(&log_path, output.join("client_focus_loss-setup.log"))?;
            let mut restarted = Command::new(&executable);
            restarted
                .args(["live-host", "--config"])
                .arg(&config_path)
                .arg("--identity-dir")
                .arg(pair.directory().join("host"))
                .args([
                    "--peer",
                    &pair.client_fp,
                    "--approve-live",
                    "--owned-session",
                    "--report",
                ])
                .arg(&report)
                .stdin(Stdio::piped())
                .stdout(fs::File::create(&log_path)?)
                .stderr(Stdio::null())
                .creation_flags(0x0800_0000);
            host = Worker(restarted.spawn()?);
            let mut config =
                bricks_config::resolve(include_str!("../../../docs/examples/client.toml"), None)?;
            config.transport.peer = Some(remote);
            let (tx, rx) = tokio::sync::mpsc::channel(256);
            let signal = ui_signal.clone();
            let store = pair.client.clone();
            let peer = pair.host_fp.clone();
            let client_origin = Instant::now();
            let client_worker = tokio::task::spawn_blocking(move || {
                crate::live::runtime(&config)?.block_on(crate::live::client(
                    &config,
                    &store,
                    &peer,
                    signal,
                    rx,
                    client_origin,
                    None,
                ))
            });
            wait_for(
                || ui_signal.ready.load(Ordering::Acquire),
                None,
                Duration::from_secs(4),
            )
            .await?;
            unsafe {
                PostMessageW(ROOT.load(Ordering::Acquire) as HWND, WM_COMMAND, 1, 0);
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
            tx.send(CaptureEvent::Key {
                key: PhysicalKey {
                    usage_page: 7,
                    usage: 0xe5,
                },
                down: true,
                captured_ns: client_origin.elapsed().as_nanos() as u64,
            })
            .await?;
            tx.send(CaptureEvent::Key {
                key: PhysicalKey {
                    usage_page: 9,
                    usage: 3,
                },
                down: true,
                captured_ns: client_origin.elapsed().as_nanos() as u64,
            })
            .await?;
            // Retain tx until after focus loss so queue closure is not the cause.
            wait_for(
                || {
                    observed.key_down.load(Ordering::Acquire) > kd
                        && observed.mouse_down.load(Ordering::Acquire) > md
                },
                None,
                Duration::from_secs(2),
            )
            .await?;
            ensure_fixture()?;
            let trigger = observed.origin.elapsed().as_nanos() as u64;
            unsafe {
                PostMessageW(ROOT.load(Ordering::Acquire) as HWND, OTHER_FOCUS, 0, 0);
            }
            wait_for(
                || ui_signal.stopped.load(Ordering::Acquire),
                None,
                Duration::from_secs(1),
            )
            .await?;
            let up = || {
                observed.key_up.load(Ordering::Acquire) > ku
                    && observed.mouse_up.load(Ordering::Acquire) > mu
                    && unsafe { GetAsyncKeyState(i32::from(VK_RSHIFT)) } >= 0
                    && unsafe { GetAsyncKeyState(i32::from(VK_MBUTTON)) } >= 0
            };
            wait_for(up, None, Duration::from_millis(300)).await?;
            let latency = observed
                .last_key_up
                .load(Ordering::Acquire)
                .max(observed.last_mouse_up.load(Ordering::Acquire))
                .saturating_sub(trigger);
            drop(tx);
            client_worker.await??;
            let passed = latency <= 300_000_000;
            results.push(serde_json::json!({"scenario":scenario,"passed":passed,"trigger_to_marked_release_receipt_ns":latency,"right_shift_up":true,"middle_button_up":true,"source":"synthetic queued keys; real capture-window focus event"}));
        } else {
            for (id, key) in [
                (
                    1,
                    PhysicalKey {
                        usage_page: 7,
                        usage: 0xe5,
                    },
                ),
                (
                    2,
                    PhysicalKey {
                        usage_page: 9,
                        usage: 3,
                    },
                ),
            ] {
                connection.send_datagram(
                    Critical {
                        transition: Transition {
                            epoch,
                            id,
                            captured_ns: id,
                            key,
                            down: true,
                        },
                        barrier: Totals::default(),
                    }
                    .encode()?
                    .to_vec()
                    .into(),
                )?;
            }
            wait_for(
                || {
                    observed.key_down.load(Ordering::Acquire) > kd
                        && observed.mouse_down.load(Ordering::Acquire) > md
                },
                Some((&connection, epoch)),
                Duration::from_secs(2),
            )
            .await?;
            ensure_fixture()?;
            if unsafe { GetAsyncKeyState(i32::from(VK_RSHIFT)) } >= 0
                || unsafe { GetAsyncKeyState(i32::from(VK_MBUTTON)) } >= 0
            {
                return Err("marked downs did not establish observable held OS state".into());
            }
            let trigger = observed.origin.elapsed().as_nanos() as u64;
            match scenario {
                "disconnect" => connection.close(0u32.into(), b"intentional fixture disconnect"),
                "host_process_crash" => {
                    host.0.kill()?;
                    host.0.wait()?;
                }
                "host_process_hang" => suspend_owned_threads(host.0.id())?,
                "watchdog_process_crash" => {
                    let log = fs::read_to_string(&log_path)?;
                    let pid: u32 = log
                        .lines()
                        .find_map(|line| line.strip_prefix("Bricks input watchdog: "))
                        .ok_or("watchdog PID missing")?
                        .trim()
                        .parse()?;
                    terminate_owned_watcher(pid, host.0.id(), &executable)?;
                }
                "host_focus_loss" => unsafe {
                    PostMessageW(ROOT.load(Ordering::Acquire) as HWND, OTHER_FOCUS, 0, 0);
                },
                _ => unreachable!(),
            }
            wait_for(
                || {
                    observed.key_up.load(Ordering::Acquire) > ku
                        && observed.mouse_up.load(Ordering::Acquire) > mu
                        && unsafe { GetAsyncKeyState(i32::from(VK_RSHIFT)) } >= 0
                        && unsafe { GetAsyncKeyState(i32::from(VK_MBUTTON)) } >= 0
                },
                None,
                Duration::from_millis(300),
            )
            .await?;
            let latency = observed
                .last_key_up
                .load(Ordering::Acquire)
                .max(observed.last_mouse_up.load(Ordering::Acquire))
                .saturating_sub(trigger);
            results.push(serde_json::json!({"scenario":scenario,"passed":latency<=300_000_000,"trigger_to_marked_release_receipt_ns":latency,"right_shift_up":true,"middle_button_up":true}));
        }
        connection.close(0u32.into(), b"case ended");
        endpoint.close(0u32.into(), b"case ended");
        // Only this test-created worker is affected. A surviving root receives
        // owner-pipe EOF, then its ordinary cleanup finishes before next phase.
        drop(host.0.stdin.take());
        let until = Instant::now() + Duration::from_secs(2);
        while host.0.try_wait()?.is_none() && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(host);
    }
    Ok(results)
}
pub fn run(output: &Path) -> Result<(), AuthError> {
    fs::create_dir(output)?;
    let output = output.canonicalize()?;
    let observed = Arc::new(Observed {
        origin: Instant::now(),
        key_down: AtomicU64::new(0),
        key_up: AtomicU64::new(0),
        mouse_down: AtomicU64::new(0),
        mouse_up: AtomicU64::new(0),
        last_key_up: AtomicU64::new(0),
        last_mouse_up: AtomicU64::new(0),
    });
    OBSERVED
        .set(observed)
        .map_err(|_| "verification already started")?;
    let signal = Arc::new(Signal::default());
    signal.status("Explicit OS cleanup verification. This fixture uses synthetic right Shift and middle mouse holds only. Please leave this window focused.");
    let worker_signal = signal.clone();
    let worker_output = output.clone();
    let worker = std::thread::spawn(move || -> Result<(), AuthError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let result = rt.block_on(async {
            let until = Instant::now() + Duration::from_secs(5);
            let hwnd = loop {
                let mut found = 0usize;
                unsafe {
                    EnumWindows(Some(find_own), (&mut found as *mut usize) as isize);
                };
                if found != 0 {
                    break found as HWND;
                }
                if Instant::now() >= until {
                    return Err::<_, AuthError>("fixture window was not created".into());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            };
            ROOT.store(hwnd as usize, Ordering::Release);
            unsafe {
                OLD.store(GetWindowLongPtrW(hwnd, GWLP_WNDPROC), Ordering::Release);
                SetWindowLongPtrW(hwnd, GWLP_WNDPROC, observe as *const () as isize);
            }
            phases(&worker_output, worker_signal.clone()).await
        });
        let value = match &result {
            Ok(cases) => {
                serde_json::json!({"status":if cases.iter().all(|case|case["passed"]==true){"PASS"}else{"FAIL"},"measurement_class":"visible_windows_injection_cleanup_fixture","cases":cases,"limits":"Synthetic source, real authenticated QUIC, SendInput and marked application receipts plus OS held-state observations. Not physical Raw Input acceptance, two-PC timing or game acceptance."})
            }
            Err(error) => {
                serde_json::json!({"status":"FAIL","error":error.to_string(),"measurement_class":"visible_windows_injection_cleanup_fixture"})
            }
        };
        fs::write(
            worker_output.join("result.json"),
            serde_json::to_vec_pretty(&value)?,
        )?;
        unsafe {
            PostMessageW(ROOT.load(Ordering::Acquire) as HWND, WM_CLOSE, 0, 0);
        }
        result?;
        if value["status"] != "PASS" {
            return Err("cleanup verification exceeded its release bound".into());
        }
        Ok(())
    });
    let _capture = bricks_input_windows::live::run(signal.clone(), Instant::now(), |_| true);
    signal.stopped.store(true, Ordering::Release);
    worker.join().map_err(|_| "cleanup verifier panicked")??;
    println!(
        "PASS: marked keyboard/mouse cleanup observed. {}",
        output.join("result.json").display()
    );
    Ok(())
}
