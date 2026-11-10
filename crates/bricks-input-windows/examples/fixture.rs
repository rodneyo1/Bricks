fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let diagnose = match args.as_slice() {
        [] => false,
        [flag] if flag == "--diagnose-unmapped" => true,
        [flag] if flag == "--help" => {
            println!(
                "fixture [--diagnose-unmapped]: opt in to aggregate unsupported scan-code counts; no typed text or event sequence"
            );
            return Ok(());
        }
        _ => return Err("Usage: fixture [--diagnose-unmapped]".into()),
    };
    #[cfg(windows)]
    println!(
        "{}",
        serde_json::to_string_pretty(&bricks_input_windows::fixture::run_with_diagnostics(
            diagnose
        )?)?
    );
    #[cfg(not(windows))]
    return Err("The interactive fixture requires Windows".into());
    Ok(())
}
