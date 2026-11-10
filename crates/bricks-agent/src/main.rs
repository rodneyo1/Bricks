use bricks_control::{Controller, Result, service::Service};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
fn main() -> Result<()> {
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        let command = std::env::args().nth(1).unwrap_or_default();
        let host = matches!(
            command.as_str(),
            "live-host" | "record-host" | "--internal-input-watchdog"
        );
        if bricks_transport::live::run_cli_if_requested(host)? {
            return Ok(());
        }
    }
    let mut root = None;
    let mut idle = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--data-dir") => {
                root = Some(PathBuf::from(args.next().ok_or("missing data directory")?))
            }
            Some("--idle-exit-ms") => {
                idle = Some(Duration::from_millis(
                    args.next()
                        .ok_or("missing idle timeout")?
                        .to_str()
                        .ok_or("invalid timeout")?
                        .parse()?,
                ))
            }
            _ => return Err("usage: bricks-agent [--data-dir PATH] [--idle-exit-ms N]".into()),
        }
    }
    let root = root.map(Ok).unwrap_or_else(bricks_control::default_root)?;
    let control = Controller::open(root.clone())?;
    let service = Service::new(control)?;
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                signal.store(true, Ordering::Release);
            }
        });
        #[cfg(windows)]
        bricks_control::ipc::serve(&root, service.clone(), stop, idle).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !service.snapshot().active_operations.is_empty() {
            if tokio::time::Instant::now() >= deadline {
                return Err::<(), bricks_control::Error>(
                    "agent shutdown is incomplete; marker retained".into(),
                );
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        std::fs::remove_file(root.join("agent-running.json"))?;
        Ok(())
    })
}
