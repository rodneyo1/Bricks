#![cfg_attr(windows, windows_subsystem = "windows")]

use bricks_control::{
    Diagnostic, Error, Result, Snapshot,
    protocol::{Command, Outcome},
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc::{Sender as SyncSender, channel};
mod files;

slint::include_modules!();

fn activity(ui: &AppWindow, message: &str, error: bool) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let stamp = format!(
        "{:02}:{:02}:{:02} UTC",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    );
    // One control event per line, newest first, with a fixed memory bound.
    let line = message.replace(['\n', '\r'], " ");
    let previous = ui.get_activity();
    let mut lines = vec![format!(
        "{stamp}  {}{line}",
        if error { "Error: " } else { "" }
    )];
    lines.extend(previous.lines().take(99).map(str::to_owned));
    ui.set_activity(lines.join("\n").into());
    ui.set_status(message.into());
    ui.set_error(error);
}

fn apply_snapshot(ui: &AppWindow, snapshot: &Snapshot) {
    let rows: Vec<ComputerRow> = snapshot
        .devices
        .iter()
        .map(|d| ComputerRow {
            id: d.id.to_string().into(),
            name: d.name.clone().into(),
            address: d.address.to_string().into(),
        })
        .collect();
    ui.set_computers(ModelRc::new(VecModel::from(rows)));
    ui.set_revision(snapshot.revision.to_string().into());
    ui.set_computer_name(snapshot.settings.computer_name.clone().into());
    ui.set_saved_computer_name(snapshot.settings.computer_name.clone().into());
    ui.set_default_port(snapshot.settings.default_port.to_string().into());
    ui.set_saved_default_port(snapshot.settings.default_port.to_string().into());
    select(ui, &ui.get_selected_id());
}
fn apply_agent(ui: &AppWindow, state: &bricks_control::protocol::AgentState) {
    ui.set_live_available(state.capabilities.live_input);
    ui.set_live_active(state.live_session.is_some());
    ui.set_live_role(state.live_session.clone().unwrap_or_default().into());
    if ui.get_revision().as_str() != state.snapshot.revision.to_string() {
        apply_snapshot(ui, &state.snapshot);
    }
    if ui.get_status().as_str() != state.status {
        activity(ui, &state.status, false);
    }
}
fn bind_live(ui: &AppWindow, sender: &SyncSender<Command>) {
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_start_client(move || {
        if let Some(ui) = weak.upgrade() {
            match ui.get_selected_id().parse() {
                Ok(id) => submit(
                    &ui,
                    &tx,
                    Command::StartLive {
                        device_id: Some(id),
                        peer_fingerprint: ui.get_client_fingerprint().trim().into(),
                        bind_ip: String::new(),
                    },
                    "Connecting input. Forwarding requires Start in the capture window...",
                ),
                Err(_) => activity(&ui, "Select a saved computer first.", true),
            }
        }
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_start_host(move || {
        if let Some(ui) = weak.upgrade() {
            submit(
                &ui,
                &tx,
                Command::StartLive {
                    device_id: None,
                    peer_fingerprint: ui.get_host_fingerprint().trim().into(),
                    bind_ip: ui.get_bind_ip().trim().into(),
                },
                "Starting sharing for the verified client...",
            );
        }
    });
    let weak = ui.as_weak();
    ui.on_browse_certificate(move || {
        if let Some(ui) = weak.upgrade() {
            match files::certificate() {
                Ok(Some(path)) => ui.set_certificate_file(path.into()),
                Ok(None) => {}
                Err(error) => activity(&ui, &error.to_string(), true),
            }
        }
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_trust_peer(move || {
        if let Some(ui) = weak.upgrade() {
            submit(
                &ui,
                &tx,
                Command::TrustPeer {
                    certificate_file: ui.get_certificate_file().into(),
                    expected_sha256: ui.get_expected_fingerprint().trim().into(),
                    incoming_client: ui.get_trust_incoming(),
                },
                "Verifying the peer certificate...",
            );
        }
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_revoke_peer(move || {
        if let Some(ui) = weak.upgrade() {
            submit(
                &ui,
                &tx,
                Command::RevokePeer {
                    fingerprint: ui.get_expected_fingerprint().trim().to_ascii_lowercase(),
                    incoming_client: ui.get_trust_incoming(),
                },
                "Revoking peer trust...",
            );
        }
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_refresh_peers(move || {
        if let Some(ui) = weak.upgrade() {
            submit(&ui, &tx, Command::Peers, "Refreshing verified peers...");
        }
    });
}

fn select(ui: &AppWindow, id: &str) {
    let rows = ui.get_computers();
    if let Some(row) = rows.iter().find(|row| row.id.as_str() == id) {
        ui.set_selected_id(row.id);
        ui.set_selected_name(row.name);
        ui.set_selected_address(row.address);
    } else {
        ui.set_selected_id(SharedString::default());
        ui.set_selected_name(SharedString::default());
        ui.set_selected_address(SharedString::default());
    }
}

fn submit(ui: &AppWindow, sender: &SyncSender<Command>, command: Command, message: &str) {
    if ui.get_busy() || !ui.get_agent_online() {
        return;
    }
    match sender.try_send(command) {
        Ok(()) => {
            ui.set_busy(true);
            ui.set_dialog_error(SharedString::default());
            activity(ui, message, false);
        }
        Err(_) => activity(
            ui,
            "The control worker is unavailable. Close and reopen Bricks.",
            true,
        ),
    }
}

fn revision(ui: &AppWindow) -> u64 {
    // Set only from a validated controller snapshot, never from user text.
    ui.get_revision().parse().expect("controller revision")
}

fn bind(
    ui: &AppWindow,
    sender: SyncSender<Command>,
    root: PathBuf,
    cancellation: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
) {
    let weak = ui.as_weak();
    ui.on_add_computer(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        ui.set_edit_name(SharedString::default());
        ui.set_editor_revision(ui.get_revision());
        ui.set_edit_ip(SharedString::default());
        ui.set_edit_port(ui.get_saved_default_port());
        ui.set_dialog_error(SharedString::default());
        ui.set_dialog(1);
    });
    let weak = ui.as_weak();
    ui.on_select_computer(move |id| {
        if let Some(ui) = weak.upgrade() {
            select(&ui, &id);
        }
    });
    let weak = ui.as_weak();
    ui.on_edit_computer(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Ok(address) = ui.get_selected_address().parse::<std::net::SocketAddr>() else {
            return;
        };
        ui.set_edit_name(ui.get_selected_name());
        ui.set_editor_revision(ui.get_revision());
        ui.set_edit_ip(address.ip().to_string().into());
        ui.set_edit_port(address.port().to_string().into());
        ui.set_dialog_error(SharedString::default());
        ui.set_dialog(2);
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_save_computer(move |name, ip, port| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let id = if ui.get_dialog() == 2 {
            ui.get_selected_id().parse().ok()
        } else {
            None
        };
        let command = Command::SaveDevice {
            revision: ui.get_editor_revision().parse().expect("editor revision"),
            id,
            name: name.into(),
            ip: ip.into(),
            port: port.into(),
        };
        submit(&ui, &tx, command, "Saving computer…");
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_forget_computer(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if let Ok(id) = ui.get_selected_id().parse() {
            submit(
                &ui,
                &tx,
                Command::Forget {
                    revision: ui.get_editor_revision().parse().expect("editor revision"),
                    id,
                },
                "Forgetting computer…",
            );
        }
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_save_settings(move |name, port| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        submit(
            &ui,
            &tx,
            Command::Settings {
                revision: revision(&ui),
                name: name.into(),
                port: port.into(),
            },
            "Saving settings…",
        );
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_setup_identities(move || {
        if let Some(ui) = weak.upgrade() {
            submit(&ui, &tx, Command::Setup, "Setting up protected identities…");
        }
    });
    let weak = ui.as_weak();
    let tx = sender.clone();
    ui.on_run_check(move |kind| {
        let requested_kind = kind;
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let (kind, message) = if kind == 0 {
            (Diagnostic::Recovery, "Checking recovery…")
        } else {
            (Diagnostic::Loopback, "Checking local QUIC…")
        };
        let (kind, message) = if requested_kind == 2 {
            (Diagnostic::RecoveryNetwork, "Checking network recovery...")
        } else {
            (kind, message)
        };
        submit(&ui, &tx, Command::Check { kind }, message);
    });
    let weak = ui.as_weak();
    ui.on_export_report(move || {
        if let Some(ui) = weak.upgrade() {
            submit(&ui, &sender, Command::Export, "Exporting report…");
        }
    });
    let weak = ui.as_weak();
    ui.on_open_data_folder(move || {
        if let Err(error) = std::process::Command::new("explorer.exe")
            .arg(&root)
            .spawn()
            && let Some(ui) = weak.upgrade()
        {
            activity(&ui, &format!("Could not open data folder: {error}"), true);
        }
    });
    let weak = ui.as_weak();
    let cancel = cancellation.clone();
    let cancel_weak = ui.as_weak();
    ui.on_cancel_operation(move || {
        cancel.store(true, Ordering::Release);
        if let Some(ui) = cancel_weak.upgrade() {
            activity(
                &ui,
                "Cancellation requested; waiting for confirmation.",
                false,
            );
        }
    });
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade()
            && ui.get_busy()
        {
            activity(
                &ui,
                "Stopping the operation before closing Bricks...",
                false,
            );
            cancellation.store(true, Ordering::Release);
            closing.store(true, Ordering::Release);
            return slint::CloseRequestResponse::KeepWindowShown;
        }
        shutdown.store(true, Ordering::Release);
        slint::CloseRequestResponse::HideWindow
    });
}

fn options() -> Result<(PathBuf, bool)> {
    let mut root = None;
    let mut smoke = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => root = Some(PathBuf::from(args.next().ok_or("--data-dir needs a directory.")?)),
            Some("--smoke-test") => smoke = true,
            _ => return Err("Unknown option. Supported: --data-dir PATH, --smoke-test (requires an empty explicit data directory).".into()),
        }
    }
    if smoke
        && (root.is_none()
            || root
                .as_ref()
                .is_some_and(|r| r.join("desktop.json").exists()))
    {
        return Err("UI smoke tests require an explicit data directory without desktop.json. Your existing data was not changed.".into());
    }
    Ok((
        root.map(Ok).unwrap_or_else(bricks_control::default_root)?,
        smoke,
    ))
}

fn apply_outcome(ui: &AppWindow, update: Outcome) {
    if let Some(peers) = update.peers {
        if peers.hosts.len() == 1 {
            ui.set_client_fingerprint(peers.hosts[0].clone().into());
        } else if !peers
            .hosts
            .iter()
            .any(|peer| peer == ui.get_client_fingerprint().as_str())
        {
            ui.set_client_fingerprint(SharedString::default());
        }
        if peers.clients.len() == 1 {
            ui.set_host_fingerprint(peers.clients[0].clone().into());
        } else if !peers
            .clients
            .iter()
            .any(|peer| peer == ui.get_host_fingerprint().as_str())
        {
            ui.set_host_fingerprint(SharedString::default());
        }
    }
    if let Some(selection) = update.selection {
        ui.set_selected_id(selection.into());
    }
    if let Some(snapshot) = update.snapshot {
        apply_snapshot(ui, &snapshot);
    }
    if update.close_dialog {
        ui.set_dialog(0);
    }
    if let Some(identities) = update.identities {
        ui.set_identity_details(identities.into());
        ui.set_identity_status("This PC is set up. Sharing is still off.".into());
    }
    if let Some(summary) = update.report_summary {
        ui.set_has_report(true);
        ui.set_report_summary(summary.into());
    }
    activity(ui, &update.message, false);
}
fn run() -> Result<()> {
    let (root, smoke) = options()?;
    std::fs::create_dir_all(&root)?;
    let ui = AppWindow::new()?;
    ui.set_data_path(root.display().to_string().into());
    ui.set_revision("0".into());
    activity(&ui, "Starting the Bricks agent...", false);
    let (sender, mut receiver) = channel(8);
    let cancellation = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::new(AtomicBool::new(false));
    let closing = Arc::new(AtomicBool::new(false));
    bind(
        &ui,
        sender.clone(),
        root.clone(),
        cancellation.clone(),
        closing.clone(),
        shutdown.clone(),
    );
    bind_live(&ui, &sender);
    let weak = ui.as_weak();
    let worker_root = root.clone();
    let stop = shutdown.clone();
    let worker=std::thread::Builder::new().name("bricks-control-sdk".into()).spawn(move || {
        let runtime=tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("SDK runtime");
        runtime.block_on(async move {
            let agent=std::env::current_exe().expect("desktop executable").with_file_name("bricks-agent.exe");
            let mut last_instance=String::new();let mut offline_reported=false;
            while !stop.load(Ordering::Acquire) {
                let mut client=match bricks_control::ipc::connect_or_launch(&worker_root,&agent).await {
                    Ok(client)=>client,
                    Err(error)=>{
                        if !offline_reported {let text=format!("Agent unavailable: {error}. Reconnecting; session state cannot be confirmed.");let _=weak.upgrade_in_event_loop(move|ui|{ui.set_agent_online(false);activity(&ui,&text,true);});offline_reported=true;}
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;continue;
                    }
                };
                let state=client.state.clone();let restarted=last_instance!=state.instance;last_instance=state.instance.clone();offline_reported=false;
                let _=weak.upgrade_in_event_loop(move|ui|{apply_snapshot(&ui,&state.snapshot);apply_agent(&ui,&state);ui.set_agent_online(true);if restarted {ui.set_has_report(false);}activity(&ui,&state.status,false);});
                if let Ok(update)=client.execute(Command::Peers,Arc::new(AtomicBool::new(false))).await {let _=weak.upgrade_in_event_loop(move|ui|apply_outcome(&ui,update));}
                loop {
                    if stop.load(Ordering::Acquire){break;}
                    tokio::select! {
                        command=receiver.recv()=>{
                            let Some(command)=command else {return;};
                            cancellation.store(closing.load(Ordering::Acquire),Ordering::Release);
                            let cancellable=matches!(command,Command::Check{..}|Command::StartLive{..});
                            let _=weak.upgrade_in_event_loop(move|ui|ui.set_cancellable(cancellable));
                            let event_ui=weak.clone();
                            let result=client.execute_with_events(command,cancellation.clone(),move|state|{let state=state.clone();let _=event_ui.upgrade_in_event_loop(move|ui|apply_agent(&ui,&state));}).await.map(|mut update|{if update.snapshot.is_some(){update.snapshot=Some(client.state.snapshot.clone());}update}).map_err(|e|e.to_string());
                            let confirmed_state=client.state.snapshot.clone();
                            let close=closing.clone();let stopped=stop.clone();
                            if weak.upgrade_in_event_loop(move|ui|{
                                ui.set_busy(false);ui.set_cancellable(false);
                                if ui.get_revision().as_str()!=confirmed_state.revision.to_string(){apply_snapshot(&ui,&confirmed_state);}
                                match result {Ok(update)=>apply_outcome(&ui,update),Err(error)=>{if ui.get_dialog()!=0{ui.set_dialog_error(error.clone().into());}activity(&ui,&error,true);}}
                                if close.load(Ordering::Acquire){stopped.store(true,Ordering::Release);let _=slint::quit_event_loop();}
                            }).is_err(){return;}
                        }
                        state=client.next_state()=>match state {
                            Ok(state)=>{let _=weak.upgrade_in_event_loop(move|ui|{
                                apply_agent(&ui,&state);
                                if ui.get_revision().as_str()!=state.snapshot.revision.to_string(){apply_snapshot(&ui,&state.snapshot);}
                            });}
                            Err(error)=>{
                                let text=format!("Agent connection lost: {error}. Reconnecting. Check saved state before retrying an unfinished operation.");
                                let _=weak.upgrade_in_event_loop(move|ui|{ui.set_agent_online(false);ui.set_busy(false);activity(&ui,&text,true);});break;
                            }
                        },
                        _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{}
                    }
                }
            }
        });
    })?;
    let _smoke_timer = smoke.then(|| smoke_test(&ui, root));
    let result = ui.run();
    shutdown.store(true, Ordering::Release);
    drop(sender);
    drop(ui);
    worker
        .join()
        .map_err(|_| "Control SDK worker stopped unexpectedly.")?;
    result?;
    Ok(())
}
/// Drives the actual Slint callbacks and asynchronous controller, with isolated
/// data only. No physical input, identities or trust are generated by this test.
fn smoke_test(ui: &AppWindow, root: PathBuf) -> slint::Timer {
    let weak = ui.as_weak();
    let step = std::cell::Cell::new(0_u8);
    let started = std::time::Instant::now();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(150), move || {
        let Some(ui) = weak.upgrade() else { return; };
        let result: Result<bool> = (|| {
            if started.elapsed().as_secs() > 45 { return Err("UI smoke test timed out.".into()); }
            let current = step.get();
            if !ui.get_agent_online() || (ui.get_busy() && current!=10) { return Ok(false); }
            match current {
                0 => { screenshot(&ui, &root, "home-empty.png")?; ui.invoke_add_computer(); ui.invoke_save_computer("Gaming PC".into(), "192.168.1.20".into(), "47990".into()); }
                1 => {
                    if ui.get_computers().row_count() != 1 || ui.get_dialog() != 0 { return Err("Add PC failed.".into()); }
                    screenshot(&ui, &root, "home-saved.png")?;
                    ui.invoke_edit_computer();
                    ui.invoke_save_computer("Renamed PC".into(), "100.101.102.103".into(), "48000".into());
                }
                2 => {
                    if ui.get_selected_name() != "Renamed PC" { return Err("Edit PC failed.".into()); }
                    ui.set_screen(2);
                    ui.invoke_save_settings("My laptop".into(), "48001".into());
                }
                3 => {
                    if ui.get_default_port() != "48001" { return Err("Save settings failed.".into()); }
                    ui.invoke_add_computer();
                    if ui.get_edit_port() != "48001" { return Err("Saved default port was not applied.".into()); }
                    ui.invoke_save_computer("Invalid".into(), "0.0.0.0".into(), "0".into());
                }
                4 => {
                    if !ui.get_error() || ui.get_dialog_error().is_empty() || ui.get_computers().row_count() != 1 { return Err("Invalid-address feedback failed.".into()); }
                    screenshot(&ui, &root, "validation-error.png")?;
                    ui.set_dialog(0);
                    ui.set_advanced(true);
                    ui.invoke_run_check(0);
                }
                5 => {
                    if !ui.get_has_report() || ui.get_error() { return Err("Recovery check did not complete.".into()); }
                    screenshot(&ui, &root, "settings-advanced.png")?;
                    ui.invoke_export_report();
                }
                6 => {
                    if ui.get_error() || fs_report_count(&root)? != 1 { return Err("Report export failed.".into()); }
                    ui.set_screen(0);
                    ui.set_dialog(3);
                    ui.invoke_forget_computer();
                }
                7 => {
                    if ui.get_computers().row_count() != 0 || ui.get_dialog() != 0 { return Err("Forget PC failed.".into()); }
                    let saved: Snapshot = serde_json::from_slice(&std::fs::read(root.join("desktop.json"))?)?;
                    if !saved.devices.is_empty() || saved.settings.computer_name != "My laptop" { return Err("Persistent final state is wrong.".into()); }
                    ui.set_screen(1);
                }
                8 => {
                    screenshot(&ui, &root, "share.png")?;
                    ui.set_screen(3);
                }
                9 => {
                    screenshot(&ui, &root, "about.png")?;
                    ui.set_screen(2);
                    ui.invoke_run_check(2);
                }
                10 => {
                    if !ui.get_cancellable(){return Ok(false);}
                    ui.invoke_cancel_operation();
                }
                11 => {
                    if !ui.get_error() || !ui.get_status().contains("Cancelled"){return Err("Confirmed cancellation did not reach the UI.".into());}
                    screenshot(&ui,&root,"cancelled.png")?;
                    std::fs::write(root.join("smoke-result.json"), serde_json::to_vec_pretty(&serde_json::json!({"status":"PASS", "checks":["add", "edit", "settings", "default port", "invalid input feedback", "real recovery adapter", "report export", "forget", "persistent state", "confirmed network check cancellation through agent"]}))?)?;
                    return Ok(true);
                }
                _ => return Err("Invalid smoke-test state.".into()),
            }
            step.set(current + 1);
            Ok(false)
        })();
        match result {
            Ok(true) => { let _ = slint::quit_event_loop(); }
            Err(error) => {
                let _ = std::fs::write(root.join("smoke-result.json"), serde_json::json!({"status":"FAIL", "error":error.to_string()}).to_string());
                let _ = slint::quit_event_loop();
            }
            Ok(false) => {}
        }
    });
    timer
}

fn fs_report_count(root: &std::path::Path) -> Result<usize> {
    Ok(std::fs::read_dir(root.join("reports"))?.count())
}

fn screenshot(ui: &AppWindow, root: &std::path::Path, name: &str) -> Result<()> {
    let pixels = ui.window().take_snapshot()?;
    let file = std::fs::File::create(root.join(name))?;
    let mut encoder = png::Encoder::new(file, pixels.width(), pixels.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(pixels.as_bytes())?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        fatal(error);
    }
}

fn fatal(error: Error) {
    #[cfg(windows)]
    {
        let message: Vec<u16> = format!("Bricks could not start.\n\n{error}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let title: Vec<u16> = "Bricks".encode_utf16().chain(Some(0)).collect();
        // SAFETY: Null-terminated strings live for the synchronous native dialog.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                0x10,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("Bricks could not start: {error}");
    std::process::exit(1);
}
