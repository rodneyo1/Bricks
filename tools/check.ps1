$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $workspacePath
try {
    $cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargoCommand) {
        $cargoPath = $cargoCommand.Source
    } else {
        $cargoPath = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
        if (-not (Test-Path -LiteralPath $cargoPath)) {
            throw 'Cargo was not found. Install the pinned Rust toolchain first.'
        }
    }
    function Invoke-CargoCheck {
        param([string[]]$CargoArguments)
        & $cargoPath @CargoArguments
        if ($LASTEXITCODE -ne 0) { throw "cargo failed with exit code $LASTEXITCODE" }
    }
    Invoke-CargoCheck -CargoArguments @('fmt', '--all', '--', '--check')
    Invoke-CargoCheck -CargoArguments @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
    Invoke-CargoCheck -CargoArguments @('test', '--workspace', '--locked')
    Invoke-CargoCheck -CargoArguments @('run', '--locked', '-p', 'bricks-client', '--', 'config', 'docs/examples/client.toml')
    Invoke-CargoCheck -CargoArguments @('run', '--locked', '-p', 'bricks-host', '--', 'config', 'docs/examples/host.toml')
    Invoke-CargoCheck -CargoArguments @('run', '--locked', '-p', 'bricks-host', '--', 'recovery-selftest')
} finally {
    Pop-Location
}
