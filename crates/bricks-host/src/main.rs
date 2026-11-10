fn main() -> std::process::ExitCode {
    #[cfg(all(windows, target_pointer_width = "64"))]
    if std::env::args().nth(1).as_deref() == Some("verify-live-cleanup") {
        let result = std::env::args_os()
            .nth(2)
            .ok_or_else(|| "verify-live-cleanup needs a fresh output directory".into())
            .and_then(|path| bricks_transport::live_verification::run(std::path::Path::new(&path)));
        return match result {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    #[cfg(all(windows, target_pointer_width = "64"))]
    match bricks_transport::live::run_cli_if_requested(true) {
        Ok(true) => return std::process::ExitCode::SUCCESS,
        Ok(false) => {}
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    }
    match bricks_transport::recovery_check::run_cli_if_requested() {
        Ok(true) => return std::process::ExitCode::SUCCESS,
        Ok(false) => {}
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    }
    match bricks_transport::identity::run_cli_if_requested("bricks-host") {
        Ok(true) => return std::process::ExitCode::SUCCESS,
        Ok(false) => {}
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    }
    match bricks_transport::probe::run_cli_if_requested(true) {
        Ok(true) => return std::process::ExitCode::SUCCESS,
        Ok(false) => {}
        Err(error) => {
            eprintln!("{error}");
            return std::process::ExitCode::FAILURE;
        }
    }
    match bricks_config::run_config_cli(bricks_config::Role::Host) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
