param([switch]$SkipBuild, [switch]$Quick, [switch]$StrictRecovery, [switch]$GamesScheduling, [switch]$HighQoS)
$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent $PSScriptRoot
$cargoPath = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargoCommand) { $cargoPath = $cargoCommand.Source }
$outputPath = Join-Path $workspacePath ('artifacts\single-pc-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss'))
if (Test-Path -LiteralPath $outputPath) { throw 'Benchmark output already exists; preserve prior evidence.' }
Push-Location -LiteralPath $workspacePath
try {
    if (-not $SkipBuild) {
        & $cargoPath build --release --locked --offline -p bricks-lab
        if ($LASTEXITCODE -ne 0) { throw 'Release compilation failed.' }
        & $cargoPath rustc --release --locked --offline -p bricks-agent -- -C lto=fat
        if ($LASTEXITCODE -ne 0) { throw 'Optimized agent compilation failed.' }
    }
    [void][IO.Directory]::CreateDirectory((Join-Path $outputPath 'binaries'))
    foreach ($binary in @('bricks-agent.exe', 'bricks-lab.exe')) {
        Copy-Item -LiteralPath (Join-Path $workspacePath ('target\release\' + $binary)) -Destination (Join-Path $outputPath ('binaries\' + $binary))
    }
    $inputsPath = Join-Path $outputPath 'inputs'
    [void][IO.Directory]::CreateDirectory($inputsPath)
    foreach ($inputFile in @('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'docs\targets-and-benchmarks.md', 'docs\examples\host.toml', 'docs\examples\client.toml')) {
        Copy-Item -LiteralPath (Join-Path $workspacePath $inputFile) -Destination $inputsPath
    }
    $environment = [ordered]@{
        utc = [DateTime]::UtcNow.ToString('o')
        operating_system = [Environment]::OSVersion.VersionString
        logical_processors = [Environment]::ProcessorCount
        lockfile_sha256 = (Get-FileHash -LiteralPath Cargo.lock -Algorithm SHA256).Hash
        input_snapshots = @(Get-ChildItem -LiteralPath $inputsPath -File | ForEach-Object { [ordered]@{name=$_.Name;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash} })
        toolchain = (& (Join-Path (Split-Path -Parent $cargoPath) 'rustc.exe') -Vv | Out-String).Trim()
        binaries = @(Get-ChildItem (Join-Path $outputPath 'binaries') -File | ForEach-Object { [ordered]@{name=$_.Name;bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash} })
        background_load = 'Ordinary interactive development desktop; not a controlled hardware acceptance run'
        network = 'IPv4 loopback; no tunnel, physical NIC or OS input backend'
        recovery_profile = if ($StrictRecovery) { '30 ms history cutoff' } else { '150 ms history cutoff, within existing 30-150 ms bounds; immediate input application' }
        scheduling = if ($GamesScheduling) { 'Windows MMCSS Games task, scoped current thread' } else { 'Ordinary thread scheduling' }
        power_qos = if ($HighQoS) { 'Explicit process-local HighQoS' } else { 'OS heuristics' }
        agent_build = 'Release, full LTO; size-oriented entry/control parsers; speed-oriented core/transport'
        lab_build = 'Release, thin LTO; root input actor plus two QUIC I/O workers'
        lab_timer_policy = 'Scoped process policy honors active timer-resolution requests where supported; manifest/case records actual support'
        source_revision = 'No Git repository; lockfile and executable hashes identify this run'
    }
    try { $environment.cpu = @(Get-CimInstance Win32_Processor -ErrorAction Stop | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors) } catch { $environment.cpu = 'Unavailable to this account' }
    try { $environment.gpu = @(Get-CimInstance Win32_VideoController -ErrorAction Stop | Select-Object Name,DriverVersion) } catch { $environment.gpu = 'Unavailable to this account' }
    try { $environment.battery = @(Get-CimInstance Win32_Battery -ErrorAction Stop | Select-Object BatteryStatus,EstimatedChargeRemaining) } catch { $environment.battery = 'Unavailable to this account' }
    $environment.power_scheme = (& powercfg /getactivescheme | Out-String).Trim()
    $environment | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputPath 'environment.json') -Encoding UTF8
    $labPath = Join-Path $outputPath 'binaries\bricks-lab.exe'
    $runArguments = @('suite', (Join-Path $outputPath 'network'))
    if ($Quick) { $runArguments += '--quick' }
    if ($GamesScheduling) { $runArguments += '--games-scheduling' }
    if ($HighQoS) { $runArguments += '--high-qos' }
    if (-not $StrictRecovery) { $runArguments += '--conservative-recovery' }
    & $labPath @runArguments
    if ($LASTEXITCODE -ne 0) { throw 'Network suite failed to execute; inspect its artifacts.' }
    & $labPath sdk-crash-test (Join-Path $outputPath 'binaries\bricks-agent.exe') (Join-Path $outputPath 'sdk-crash')
    if ($LASTEXITCODE -ne 0) { throw 'Agent crash/restart check failed.' }
    $idleRoot = Join-Path $outputPath 'idle-agent'
    $agentProcess = Start-Process -FilePath (Join-Path $outputPath 'binaries\bricks-agent.exe') -ArgumentList @('--data-dir', ('"' + $idleRoot + '"'), '--idle-exit-ms', '65000') -WindowStyle Hidden -PassThru
    try {
        Start-Sleep -Milliseconds 500
        & $labPath resources $agentProcess.Id 60 | Set-Content -LiteralPath (Join-Path $outputPath 'agent-idle-resources.json') -Encoding UTF8
        if ($LASTEXITCODE -ne 0) { throw 'Agent resource sampling failed.' }
    } finally {
        if (-not $agentProcess.WaitForExit(10000)) { throw 'Idle agent did not shut down in its declared interval.' }
    }
    Write-Output ('Results: ' + $outputPath)
    Write-Output 'Reports label synthetic loopback results. They cannot pass two-PC capture-to-submit or physical gaming gates.'
} finally { Pop-Location }
