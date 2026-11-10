param([switch]$Gui)
$ErrorActionPreference = 'Stop'
$workspacePath = Split-Path -Parent $PSScriptRoot
$sourcePath = Join-Path $workspacePath 'ui\windows'
$testOutputPath = Join-Path $workspacePath 'target\ui-tests'
[void][System.IO.Directory]::CreateDirectory($testOutputPath)
$compilerPath = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
$compilerArguments = @(
    '/nologo', '/target:exe', '/platform:x64', '/optimize+', '/warn:4', '/warnaserror+',
    '/main:Bricks.Desktop.FrontendTests', ('/out:' + (Join-Path $testOutputPath 'Bricks.Ui.Tests.exe')),
    ('/win32manifest:' + (Join-Path $sourcePath 'Bricks.manifest')),
    '/r:System.dll', '/r:System.Core.dll', '/r:System.Drawing.dll', '/r:System.Windows.Forms.dll', '/r:System.Web.Extensions.dll',
    ('/resource:' + (Join-Path $workspacePath 'docs\examples\host.toml') + ',host.toml'),
    ('/resource:' + (Join-Path $workspacePath 'docs\examples\client.toml') + ',client.toml'),
    (Join-Path $sourcePath 'Bricks.Core.cs'), (Join-Path $sourcePath 'Bricks.Form.cs'), (Join-Path $sourcePath 'Bricks.Tests.cs'), (Join-Path $sourcePath 'AssemblyInfo.cs')
)
& $compilerPath @compilerArguments
if ($LASTEXITCODE -ne 0) { throw 'UI tests did not compile.' }
Copy-Item -LiteralPath (Join-Path $sourcePath 'Bricks.exe.config') -Destination (Join-Path $testOutputPath 'Bricks.Ui.Tests.exe.config') -Force
$testArguments = @($workspacePath)
if ($Gui) { $testArguments += '--gui' }
& (Join-Path $testOutputPath 'Bricks.Ui.Tests.exe') @testArguments
if ($LASTEXITCODE -ne 0) { throw 'UI tests failed.' }
