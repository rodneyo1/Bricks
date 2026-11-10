param([switch]$SkipRustBuild)
$ErrorActionPreference = 'Stop'
# Preserve the familiar command while making it build the product frontend.
& (Join-Path $PSScriptRoot 'build-desktop.ps1') -SkipBuild:$SkipRustBuild
