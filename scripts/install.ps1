$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Root = Split-Path -Parent $PSScriptRoot
$Manifest = Join-Path $Root "herdr-plugin.toml"
$VersionMatch = Select-String -Path $Manifest -Pattern '^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $VersionMatch) { throw "Missing version in herdr-plugin.toml" }
$Version = $VersionMatch.Matches[0].Groups[1].Value

$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($Arch -ne "x64") { throw "Unsupported Windows architecture: $Arch" }

$Asset = "herdr-layout-windows-x64.exe"
$Url = "https://github.com/phenome/herdr-layout/releases/download/v$Version/$Asset"
$BinDir = Join-Path $Root "bin"
$Out = Join-Path $BinDir "herdr-layout.exe"
$Shim = Join-Path $BinDir "herdr-layout.cmd"
$CmdDispatcher = Join-Path $Root "bin.cmd"

New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
Write-Host "Downloading Herdr Layout binary: $Url"
Invoke-WebRequest -Uri $Url -OutFile $Out
Write-Host "Installed Herdr Layout binary: $Out"
Set-Content -Path $Shim -Encoding ASCII -Value '@echo off
"%~dp0herdr-layout.exe" %*'
Set-Content -Path $CmdDispatcher -Encoding ASCII -Value '@echo off
if /I "%~1"=="/herdr-layout.cmd" (
  "%~dp0bin\herdr-layout.exe" %2 %3 %4 %5 %6 %7 %8 %9
) else (
  "%~dp0bin\herdr-layout.exe" %*
)'
