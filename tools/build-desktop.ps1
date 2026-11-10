param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent $PSScriptRoot
$outputPath = Join-Path $workspacePath 'dist\BricksDesktop'
$cargoPath = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargoCommand) { $cargoPath = $cargoCommand.Source }
Push-Location -LiteralPath $workspacePath
try {
    if (-not $SkipBuild) {
        & $cargoPath build --profile desktop --locked --offline -p bricks-ui
        if ($LASTEXITCODE -ne 0) { throw 'Bricks desktop compilation failed. On a new development PC, run cargo fetch --locked first.' }
        & $cargoPath rustc --release --locked --offline -p bricks-agent -- -C lto=fat
        if ($LASTEXITCODE -ne 0) { throw 'Optimized agent compilation failed.' }
    }
    [void][System.IO.Directory]::CreateDirectory($outputPath)
    Copy-Item -LiteralPath (Join-Path $workspacePath 'target\desktop\bricks-ui.exe') -Destination (Join-Path $outputPath 'Bricks.exe') -Force
    $agentBinary = Join-Path $workspacePath 'target\release\bricks-agent.exe'
    Copy-Item -LiteralPath $agentBinary -Destination (Join-Path $outputPath 'bricks-agent.exe') -Force
    Copy-Item -LiteralPath (Join-Path $workspacePath 'ui\desktop\START-HERE.txt') -Destination $outputPath -Force
    Copy-Item -LiteralPath (Join-Path $workspacePath 'ui\desktop\NOTICES.txt') -Destination $outputPath -Force
    Copy-Item -LiteralPath (Join-Path $workspacePath 'ui\desktop\Slint-Royalty-Free-2.0.md') -Destination $outputPath -Force
    $shortcutShell = New-Object -ComObject WScript.Shell
    $shortcut = $shortcutShell.CreateShortcut((Join-Path $workspacePath 'Start Bricks.lnk'))
    $shortcut.TargetPath = Join-Path $outputPath 'Bricks.exe'
    $shortcut.WorkingDirectory = $outputPath
    $shortcut.Description = 'Open the Bricks desktop application'
    $shortcut.IconLocation = (Join-Path $outputPath 'Bricks.exe') + ',0'
    $shortcut.Save()
    Write-Output ('Ready: ' + (Join-Path $outputPath 'Bricks.exe'))
    Write-Output 'Double-click Start Bricks. Copy the complete dist\BricksDesktop folder to another Windows PC; Rust is not needed to run it.'
} finally { Pop-Location }
