# Installs the Emberhut runner on Windows, pairs it with a server and starts it as a Windows
# service. Pairing and the service need an administrator PowerShell:
#
#   $env:EMBERHUT_SERVER = "https://..."; $env:EMBERHUT_PAIR_CODE = "..."
#   irm https://github.com/emberhuthq/emberhut/releases/latest/download/install-runner.ps1 | iex
#
# or, saved as a file: .\install-runner.ps1 -Server URL -Pair CODE [-Version V] [-Prefix DIR]
#
# It downloads the release's Windows build, checks it against the release's SHA256SUMS, unpacks
# it into <prefix>\versions\<version> (default prefix: %LOCALAPPDATA%\Programs\Emberhut Runner),
# points <prefix>\current at it and puts that folder on this user's PATH. With a server and a
# code it then runs `emberhut-runner install --server URL --pair CODE`, which pairs the runner and
# starts its service through <prefix>\current, so a later upgrade only moves that link.
#
# The service runs as LocalSystem until another account is set in its properties (Log On); the
# agent has to be signed in as the account the service runs as.
#
# EMBERHUT_RUNNER_DOWNLOADS replaces https://github.com/emberhuthq/emberhut/releases/download, for
# a mirror holding the same v<version>/ folders.
param(
  [string]$Server = $env:EMBERHUT_SERVER,
  [string]$Pair = $env:EMBERHUT_PAIR_CODE,
  [string]$Version = "",
  [string]$Prefix = $env:EMBERHUT_RUNNER_PREFIX
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repo = "emberhuthq/emberhut"
# The release workflow writes its version here; empty means the latest release.
$ScriptVersion = ""

if (-not $Version) { $Version = $ScriptVersion }
if (-not $Prefix) { $Prefix = Join-Path $env:LOCALAPPDATA "Programs\Emberhut Runner" }
$Downloads = if ($env:EMBERHUT_RUNNER_DOWNLOADS) { $env:EMBERHUT_RUNNER_DOWNLOADS } else { "https://github.com/$Repo/releases/download" }
$Arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($Arch -ne "AMD64") { throw "There is no runner build for $Arch on Windows (x64 only)." }
$Target = "windows-x64"

if (-not $Version) {
  $Latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing
  $Version = $Latest.tag_name
  if (-not $Version) { throw "Can't find the latest release; name one with -Version." }
}
$Version = $Version.TrimStart("v")
if ($Version -notmatch '^[0-9A-Za-z.-]+$') { throw "Not a version: $Version" }

$Base = "$Downloads/v$Version"
$File = "emberhut-runner-$Version-$Target.zip"
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("emberhut-runner-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  Write-Host "Downloading emberhut-runner $Version for $Target..."
  Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile "$Tmp\SHA256SUMS" -UseBasicParsing
  $Want = $null
  foreach ($Line in Get-Content "$Tmp\SHA256SUMS") {
    $Hash, $Name = $Line -split '\s+', 2
    if ($Name -eq $File -or $Name -eq "*$File") { $Want = $Hash.ToLowerInvariant(); break }
  }
  if (-not $Want) { throw "Release v$Version has no runner build for $Target." }
  Invoke-WebRequest -Uri "$Base/$File" -OutFile "$Tmp\$File" -UseBasicParsing
  $Got = (Get-FileHash -Algorithm SHA256 "$Tmp\$File").Hash.ToLowerInvariant()
  if ($Got -ne $Want) { throw "$File's SHA-256 is $Got, not the $Want the release lists." }

  Expand-Archive -Path "$Tmp\$File" -DestinationPath "$Tmp\x"
  $Unpacked = Join-Path "$Tmp\x" "emberhut-runner-$Version-$Target"
  if (-not (Test-Path (Join-Path $Unpacked "emberhut-runner.exe"))) { throw "$File holds no emberhut-runner.exe." }
  $Dest = Join-Path $Prefix "versions\$Version"
  New-Item -ItemType Directory -Force -Path (Split-Path $Dest) | Out-Null
  if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
  Move-Item $Unpacked $Dest
  $Current = Join-Path $Prefix "current"
  # A junction: removing it leaves the folder it points at alone.
  if (Test-Path $Current) { (Get-Item $Current).Delete() }
  New-Item -ItemType Junction -Path $Current -Target $Dest | Out-Null
  $Runner = Join-Path $Current "emberhut-runner.exe"
  # The service runs the runner through "current", which stays the same across upgrades.
  $env:EMBERHUT_RUNNER_LAUNCHER = $Runner
  Write-Host "Installed emberhut-runner $Version in $Dest."

  $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
  if (-not (($UserPath -split ';') -contains $Current)) {
    $NewPath = if ($UserPath) { "$UserPath;$Current" } else { $Current }
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    Write-Host "Added $Current to your PATH (new terminals see it)."
  }

  if ($Server -and $Pair) {
    $Admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
      [Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $Admin) { throw "Installing the service needs an administrator: run this again from an administrator PowerShell." }
    & $Runner install --server $Server --pair $Pair
    if ($LASTEXITCODE -ne 0) { throw "emberhut-runner install failed." }
  } else {
    Write-Host "To pair it with your server and start its service, get a code from the app's Machines page, then run in an administrator PowerShell:"
    Write-Host "  emberhut-runner install --server <your server> --pair <code>"
    Write-Host "If its service was installed before, restart it to run this version: emberhut-runner service restart"
  }
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
