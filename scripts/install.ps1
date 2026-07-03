$ErrorActionPreference = "Stop"

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

New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
Invoke-WebRequest -Uri $Url -OutFile $Out
