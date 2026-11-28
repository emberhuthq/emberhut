# Installs the Emberhut runner for this user on Windows, pairs it with a server and starts it at logon:
#
#   $env:EMBERHUT_SERVER="https://…"; $env:EMBERHUT_PAIR_CODE="…"; irm https://github.com/emberhuthq/emberhut/releases/latest/download/install-runner.ps1 | iex
#
# or, saved as a file: .\install-runner.ps1 -Server URL -Code CODE [-Name NAME] [-Advertise URL] [-Version V]
#
# It downloads the release's Windows build, checks its SHA-256 against the release's SHA256SUMS, unpacks it into
# %LOCALAPPDATA%\Programs\Emberhut Runner\versions\<version>, points "current" at it, and with a server and a code pairs
# and installs the scheduled task (at logon, as this user), then waits for the runner to answer. No administrator rights.
param(
  [string]$Server = $env:EMBERHUT_SERVER,
  [string]$Code = $env:EMBERHUT_PAIR_CODE,
  [string]$Name = $env:EMBERHUT_RUNNER_NAME,
  [string]$Advertise = $env:EMBERHUT_RUNNER_ADVERTISE,
  [string]$Version = $env:EMBERHUT_RUNNER_VERSION,
  [string]$Prefix = $env:EMBERHUT_RUNNER_PREFIX
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repo = "emberhuthq/emberhut"
# A release sets this to its own version; empty means the latest release.
$ScriptVersion = ""
if (-not $Version) { $Version = $ScriptVersion }
if (-not $Prefix) { $Prefix = Join-Path $env:LOCALAPPDATA "Programs\Emberhut Runner" }
if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") { throw "There is no runner build for $($env:PROCESSOR_ARCHITECTURE) on Windows yet (x64 only)." }
$Target = "windows-x64"

$Base = if ($Version) { "https://github.com/$Repo/releases/download/v$($Version.TrimStart('v'))" } else { "https://github.com/$Repo/releases/latest/download" }
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("emberhut-runner-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  Write-Host "Downloading the runner for $Target..."
  Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile "$Tmp\SHA256SUMS" -UseBasicParsing
  $Line = Get-Content "$Tmp\SHA256SUMS" | Where-Object { $_ -match "  emberhut-runner-.+-$Target\.zip$" } | Select-Object -First 1
  if (-not $Line) { throw "The release has no runner build for $Target." }
  $Want, $File = $Line -split "  ", 2
  Invoke-WebRequest -Uri "$Base/$File" -OutFile "$Tmp\$File" -UseBasicParsing
  $Got = (Get-FileHash -Algorithm SHA256 "$Tmp\$File").Hash.ToLowerInvariant()
  if ($Got -ne $Want) { throw "$File's SHA-256 is $Got, not the release's $Want." }

  $V = $File -replace "^emberhut-runner-(.+)-$Target\.zip$", '$1'
  $Dest = Join-Path $Prefix "versions\$V"
  if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
  Expand-Archive -Path "$Tmp\$File" -DestinationPath "$Tmp\x"
  New-Item -ItemType Directory -Force -Path (Split-Path $Dest) | Out-Null
  Move-Item (Join-Path "$Tmp\x" "emberhut-runner-$V-$Target") $Dest
  $Current = Join-Path $Prefix "current"
  if (Test-Path $Current) { (Get-Item $Current).Delete() }
  New-Item -ItemType Junction -Path $Current -Target $Dest | Out-Null
  $Runner = Join-Path $Current "bin\emberhut-runner.cmd"
  # The scheduled task runs the launcher through "current", so an upgrade needs only a restart.
  $env:EMBERHUT_RUNNER_LAUNCHER = $Runner
  Write-Host "Installed emberhut-runner $V in $Prefix."

  if ($Server -and $Code) {
    $PairArgs = @("pair", "--server", $Server, "--code", $Code)
    if ($Name) { $PairArgs += @("--name", $Name) }
    if ($Advertise) { $PairArgs += @("--advertise", $Advertise) }
    & $Runner @Args
    if ($LASTEXITCODE -ne 0) { throw "Pairing failed." }
  } else {
    & $Runner paired *> $null
    if ($LASTEXITCODE -eq 0) { & $Runner service install } else {
      Write-Host "Not paired yet. Get a code from the app's Machines page (Add a hut), then run:"
      Write-Host "  `"$Runner`" pair --server <your server> --code <code>"
      return
    }
  }
  for ($i = 0; $i -lt 30; $i++) {
    & $Runner status *> $null
    if ($LASTEXITCODE -eq 0) { & $Runner status; return }
    Start-Sleep -Seconds 1
  }
  throw "The runner didn't answer within 30 s; see its log (emberhut-runner service status)."
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
