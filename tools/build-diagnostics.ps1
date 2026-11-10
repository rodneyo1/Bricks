param([switch]$SkipRustBuild)
$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent $PSScriptRoot
$outputPath = Join-Path $workspacePath 'dist\Bricks'
$sourcePath = Join-Path $workspacePath 'ui\windows'
$compilerPath = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (-not (Test-Path -LiteralPath $compilerPath)) { throw 'The Windows .NET Framework C# compiler was not found. This UI requires .NET Framework 4.8.' }
Push-Location -LiteralPath $workspacePath
try {
    if (-not $SkipRustBuild) {
        $cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
        $cargoPath = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
        & $cargoPath build --workspace --all-targets --locked --offline
        if ($LASTEXITCODE -ne 0) { throw 'Rust tools could not be built.' }
    }
    [void][System.IO.Directory]::CreateDirectory((Join-Path $outputPath 'bin'))
    $compilerArguments = @(
        '/nologo', '/target:winexe', '/platform:x64', '/optimize+', '/warn:4', '/warnaserror+',
        ('/out:' + (Join-Path $outputPath 'Bricks.exe')),
        ('/win32manifest:' + (Join-Path $sourcePath 'Bricks.manifest')),
        ('/win32icon:' + (Join-Path $sourcePath 'Bricks.ico')),
        '/r:System.dll', '/r:System.Core.dll', '/r:System.Drawing.dll', '/r:System.Windows.Forms.dll', '/r:System.Web.Extensions.dll',
        ('/resource:' + (Join-Path $workspacePath 'docs\examples\host.toml') + ',host.toml'),
        ('/resource:' + (Join-Path $workspacePath 'docs\examples\client.toml') + ',client.toml'),
        (Join-Path $sourcePath 'Bricks.Core.cs'), (Join-Path $sourcePath 'Bricks.Form.cs'), (Join-Path $sourcePath 'AssemblyInfo.cs')
    )
    & $compilerPath @compilerArguments
    if ($LASTEXITCODE -ne 0) { throw 'Windows UI compilation failed.' }
    Copy-Item -LiteralPath (Join-Path $sourcePath 'Bricks.exe.config') -Destination (Join-Path $outputPath 'Bricks.exe.config') -Force
    foreach ($binaryName in @('bricks-host.exe', 'bricks-client.exe')) {
        Copy-Item -LiteralPath (Join-Path $workspacePath ('target\debug\' + $binaryName)) -Destination (Join-Path $outputPath ('bin\' + $binaryName)) -Force
    }
    Copy-Item -LiteralPath (Join-Path $workspacePath 'target\debug\examples\fixture.exe') -Destination (Join-Path $outputPath 'bin\fixture.exe') -Force
    Copy-Item -LiteralPath (Join-Path $sourcePath 'START-HERE.txt') -Destination (Join-Path $outputPath 'START-HERE.txt') -Force
    $shortcutShell = New-Object -ComObject WScript.Shell
    $shortcut = $shortcutShell.CreateShortcut((Join-Path $workspacePath 'Bricks Diagnostics.lnk'))
    $shortcut.TargetPath = Join-Path $outputPath 'Bricks.exe'
    $shortcut.WorkingDirectory = $outputPath
    $shortcut.Description = 'Open Bricks checks and connection setup'
    $shortcut.IconLocation = (Join-Path $outputPath 'Bricks.exe') + ',0'
    $shortcut.Save()
    Write-Output ('Ready: ' + (Join-Path $outputPath 'Bricks.exe'))
    Write-Output 'Double-click Bricks Diagnostics in the workspace folder. Keep the complete dist\Bricks folder together when copying to another PC.'
} finally { Pop-Location }
