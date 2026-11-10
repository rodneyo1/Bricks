param([string]$ExecutablePath)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$workspacePath = Split-Path -Parent $PSScriptRoot
$testPath = Join-Path $workspacePath ('artifacts\desktop-native-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($testPath)
if (-not $ExecutablePath) { $ExecutablePath = Join-Path $workspacePath 'target\debug\bricks-ui.exe' }
$ExecutablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
$appProcess = Start-Process -FilePath $ExecutablePath -ArgumentList @('--data-dir', ('"' + $testPath + '"')) -WindowStyle Normal -PassThru

function Find-Element([System.Windows.Automation.AutomationElement]$parent, [string]$name, [System.Windows.Automation.ControlType]$kind = $null) {
    $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, $name)
    if ($kind) {
        $typeCondition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, $kind)
        $condition = New-Object System.Windows.Automation.AndCondition($condition, $typeCondition)
    }
    $until = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $element = $parent.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        if ($element) { return $element }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $until)
    throw ('Accessible control was not found: ' + $name)
}

function Invoke-Button([System.Windows.Automation.AutomationElement]$parent, [string]$name) {
    $element = Find-Element $parent $name ([System.Windows.Automation.ControlType]::Button)
    $until = [DateTime]::UtcNow.AddSeconds(10)
    while (-not $element.Current.IsEnabled -and [DateTime]::UtcNow -lt $until) { Start-Sleep -Milliseconds 100 }
    if (-not $element.Current.IsEnabled) { throw ('Control did not become enabled: ' + $name) }
    $pattern = $element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
}

function Set-Field([System.Windows.Automation.AutomationElement]$parent, [string]$name, [string]$value) {
    $element = Find-Element $parent $name ([System.Windows.Automation.ControlType]::Edit)
    $pattern = $element.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
    $pattern.SetValue($value)
}

try {
    $desktop = [System.Windows.Automation.AutomationElement]::RootElement
    $condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ProcessIdProperty, [int]$appProcess.Id)
    $window = $null
    $until = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $window = $desktop.FindFirst([System.Windows.Automation.TreeScope]::Children, $condition)
        if (-not $window) { Start-Sleep -Milliseconds 100 }
    } while (-not $window -and [DateTime]::UtcNow -lt $until)
    if (-not $window) { throw 'Bricks native window was not found.' }
    Invoke-Button $window 'Add PC'
    Set-Field $window 'Friendly name' 'Native UI PC'
    Set-Field $window 'LAN or tunnel IP address' '192.168.2.15'
    Set-Field $window 'Port' '47990'
    Invoke-Button $window 'Save'
    $statePath = Join-Path $testPath 'desktop.json'
    $until = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $statePath) -and [DateTime]::UtcNow -lt $until) { Start-Sleep -Milliseconds 100 }
    $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
    if ($state.devices.Count -ne 1 -or $state.devices[0].name -ne 'Native UI PC' -or $state.devices[0].address -ne '192.168.2.15:47990') { throw 'Native controls did not persist the entered computer.' }
    [void](Find-Element $window 'Select Native UI PC')
    Invoke-Button $window 'Edit'
    Set-Field $window 'Friendly name' 'Edited via native controls'
    Invoke-Button $window 'Save'
    $until = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        if ($state.devices[0].name -ne 'Edited via native controls') { Start-Sleep -Milliseconds 100 }
    } while ($state.devices[0].name -ne 'Edited via native controls' -and [DateTime]::UtcNow -lt $until)
    if ($state.devices[0].name -ne 'Edited via native controls') { throw 'Native edit did not persist.' }
    Write-Output ('PASS: native Windows accessibility actions added and edited a PC. Evidence: ' + $statePath)
} finally {
    $appProcess.Refresh()
    if (-not $appProcess.HasExited) {
        [void]$appProcess.CloseMainWindow()
        if (-not $appProcess.WaitForExit(15000)) {
            # Explicit isolated UI process; no OS input capability exists here.
            Stop-Process -Id $appProcess.Id
        }
    }
}
