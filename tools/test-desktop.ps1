param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent $PSScriptRoot
$cargoPath = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargoCommand) { $cargoPath = $cargoCommand.Source }
Push-Location -LiteralPath $workspacePath
try {
    & $cargoPath test --locked --offline -p bricks-control
    if ($LASTEXITCODE -ne 0) { throw 'Desktop controller tests failed.' }
    if (-not $SkipBuild) {
        & $cargoPath build --locked --offline -p bricks-ui -p bricks-agent
        if ($LASTEXITCODE -ne 0) { throw 'Desktop compilation failed.' }
    }
    $testPath = Join-Path $workspacePath ('artifacts\desktop-smoke-' + [Guid]::NewGuid().ToString('N'))
    [void][System.IO.Directory]::CreateDirectory($testPath)
    # An explicit fresh path is mandatory; the test never opens normal app data.
    $testProcess = Start-Process -FilePath (Join-Path $workspacePath 'target\debug\bricks-ui.exe') -ArgumentList @('--smoke-test', '--data-dir', ('"' + $testPath + '"')) -WindowStyle Normal -PassThru
    if (-not $testProcess.WaitForExit(60000)) {
        # Recording-only isolated test process; it cannot inject OS input.
        Stop-Process -Id $testProcess.Id
        throw 'Desktop smoke test did not exit in 60 seconds.'
    }
    $resultPath = Join-Path $testPath 'smoke-result.json'
    if (-not (Test-Path -LiteralPath $resultPath)) { throw ('Smoke test produced no result: ' + $testPath) }
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    if ($result.status -ne 'PASS' -or $testProcess.ExitCode -ne 0) { throw ('Desktop smoke test failed: ' + ($result | ConvertTo-Json -Compress)) }
    Write-Output ('PASS: desktop callback/worker/persistence smoke test (' + $result.checks.Count + ' checks).')
    Write-Output ('Evidence: ' + $resultPath)
} finally { Pop-Location }
