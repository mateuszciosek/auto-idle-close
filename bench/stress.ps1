# Stress/perf comparison harness for auto-idle-close.
#
# Emulates the terminal-server (RDP) rendering path by forcing wgpu onto the
# WARP software rasterizer (WGPU_ADAPTER_NAME), then runs a matrix of
# scenarios against two builds:
#   v080 - baseline built from git tag v0.8.0
#   v081 - current working tree
#
# Scenarios (victims are aic-victim.exe copies; --ignore-close simulates an
# app stuck on an unsaved-changes dialog, i.e. WM_CLOSE has no effect):
#   idle-4apps     4 tracked apps, 30 min timeout: pure polling baseline
#   idle-8apps     8 tracked apps: does more apps cost more?
#   warn-popup     countdown popup active the whole window
#   close-cycle    unattended session: warn -> close ignored -> re-warn loop
#   stuck-closing  force-kill pending (kill_after=600): v0.8.0 popup bug
#
# Usage: powershell -File bench\stress.ps1 [-MeasureSec 30] [-SkipBuild]

param(
    [int]$MeasureSec = 30,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$bin = Join-Path $PSScriptRoot 'bin'
$victimSrc = Join-Path $PSScriptRoot 'victim\target\release\aic-victim.exe'
$victimDir = Join-Path $bin 'victims'
$resultsDir = Join-Path $PSScriptRoot 'results'

function Log($msg) {
    Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $msg)
}

if (Get-Process -Name 'auto-idle-close' -ErrorAction SilentlyContinue) {
    throw 'An auto-idle-close instance is already running (single-instance mutex would break the bench). Close it first.'
}

foreach ($d in @($bin, $victimDir, $resultsDir, (Join-Path $bin 'v080'), (Join-Path $bin 'v081'))) {
    New-Item -ItemType Directory -Force -Path $d | Out-Null
}

# --- builds -----------------------------------------------------------------

if (-not $SkipBuild) {
    Log 'Building victim helper...'
    cargo build --release --manifest-path (Join-Path $PSScriptRoot 'victim\Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'victim build failed' }

    Log 'Building v0.8.1 (working tree)...'
    cargo build --release --manifest-path (Join-Path $root 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'v0.8.1 build failed' }
    Copy-Item (Join-Path $root 'target\release\auto-idle-close.exe') (Join-Path $bin 'v081\auto-idle-close.exe') -Force
}

$v080exe = Join-Path $bin 'v080\auto-idle-close.exe'
if (-not (Test-Path $v080exe)) {
    Log 'Building v0.8.0 from git tag (one-time)...'
    $wt = Join-Path $env:TEMP 'aic-bench-v080'
    if (Test-Path $wt) { git -C $root worktree remove --force $wt 2>$null; Remove-Item -Recurse -Force $wt -ErrorAction SilentlyContinue }
    git -C $root worktree add $wt v0.8.0
    if ($LASTEXITCODE -ne 0) { throw 'git worktree add failed' }
    cargo build --release --manifest-path (Join-Path $wt 'Cargo.toml') --target-dir (Join-Path $wt 'target')
    if ($LASTEXITCODE -ne 0) { throw 'v0.8.0 build failed' }
    Copy-Item (Join-Path $wt 'target\release\auto-idle-close.exe') $v080exe -Force
    git -C $root worktree remove --force $wt
}

for ($i = 1; $i -le 8; $i++) {
    Copy-Item $victimSrc (Join-Path $victimDir "victim$i.exe") -Force
}

# --- scenario runner --------------------------------------------------------

function Write-BenchConfig {
    param($Dir, $Victims, $TimeoutMin, $WarnSecs, $ForceKill, $KillAfterSecs)
    $lines = @(
        "warning_seconds = $WarnSecs",
        'poll_interval_ms = 1000',
        'paused = false',
        'check_updates = false'
    )
    for ($i = 1; $i -le $Victims; $i++) {
        $fk = 'false'; if ($ForceKill) { $fk = 'true' }
        $lines += ''
        $lines += '[[rules]]'
        $lines += "exe = ""victim$i.exe"""
        $lines += 'enabled = true'
        $lines += "timeout_minutes = $TimeoutMin"
        $lines += "force_kill = $fk"
        $lines += "force_kill_after_secs = $KillAfterSecs"
    }
    Set-Content -Path (Join-Path $Dir 'config.toml') -Value ($lines -join "`n") -Encoding Ascii
}

function Run-Scenario {
    param($Version, $Scenario, $Victims, $IgnoreClose, $TimeoutMin, $WarnSecs,
          $ForceKill, $KillAfterSecs, $SettleSec, $Seconds)

    $dir = Join-Path $bin $Version
    Write-BenchConfig -Dir $dir -Victims $Victims -TimeoutMin $TimeoutMin `
        -WarnSecs $WarnSecs -ForceKill $ForceKill -KillAfterSecs $KillAfterSecs

    $victimProcs = @()
    for ($i = 1; $i -le $Victims; $i++) {
        $args = @('--title', "Victim $i (bench)")
        if ($IgnoreClose) { $args += '--ignore-close' }
        $victimProcs += Start-Process (Join-Path $victimDir "victim$i.exe") -ArgumentList $args -PassThru
    }
    Start-Sleep -Seconds 2

    $env:WGPU_ADAPTER_NAME = 'Microsoft Basic Render Driver'
    try {
        $app = Start-Process (Join-Path $dir 'auto-idle-close.exe') `
            -ArgumentList '--renderer', 'wgpu', '--tray' -PassThru
    } finally {
        Remove-Item Env:\WGPU_ADAPTER_NAME -ErrorAction SilentlyContinue
    }

    Log ("  {0}/{1}: settling {2}s, measuring {3}s..." -f $Version, $Scenario, $SettleSec, $Seconds)
    Start-Sleep -Seconds $SettleSec
    if ($app.HasExited) { throw "$Version/$Scenario : app exited during settle" }

    $c0 = (Get-Process -Id $app.Id).TotalProcessorTime.TotalSeconds
    $t0 = Get-Date
    Start-Sleep -Seconds $Seconds
    $proc = Get-Process -Id $app.Id
    $c1 = $proc.TotalProcessorTime.TotalSeconds
    $elapsed = ((Get-Date) - $t0).TotalSeconds
    $ws = [math]::Round($proc.WorkingSet64 / 1MB)

    Stop-Process -Id $app.Id -Force
    Wait-Process -Id $app.Id -Timeout 10 -ErrorAction SilentlyContinue
    foreach ($v in $victimProcs) { Stop-Process -Id $v.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Seconds 1

    [pscustomobject]@{
        Version    = $Version
        Scenario   = $Scenario
        Victims    = $Victims
        CpuPct     = [math]::Round(($c1 - $c0) / $elapsed * 100, 2)
        CpuSeconds = [math]::Round($c1 - $c0, 2)
        WindowSec  = [math]::Round($elapsed)
        WSMB       = $ws
    }
}

# --- matrix -----------------------------------------------------------------

$scenarios = @(
    @{ Scenario = 'idle-4apps';    Victims = 4; IgnoreClose = $false; TimeoutMin = 30; WarnSecs = 60;  ForceKill = $false; KillAfterSecs = 30;  SettleSec = 8;  Seconds = $MeasureSec },
    @{ Scenario = 'idle-8apps';    Victims = 8; IgnoreClose = $false; TimeoutMin = 30; WarnSecs = 60;  ForceKill = $false; KillAfterSecs = 30;  SettleSec = 8;  Seconds = $MeasureSec },
    @{ Scenario = 'warn-popup';    Victims = 4; IgnoreClose = $false; TimeoutMin = 1;  WarnSecs = 300; ForceKill = $false; KillAfterSecs = 30;  SettleSec = 8;  Seconds = 30 },
    @{ Scenario = 'after-close';   Victims = 4; IgnoreClose = $false; TimeoutMin = 1;  WarnSecs = 30;  ForceKill = $false; KillAfterSecs = 30;  SettleSec = 70; Seconds = 60 },
    @{ Scenario = 'close-cycle';   Victims = 4; IgnoreClose = $true;  TimeoutMin = 1;  WarnSecs = 300; ForceKill = $false; KillAfterSecs = 30;  SettleSec = 65; Seconds = 60 },
    @{ Scenario = 'stuck-closing'; Victims = 4; IgnoreClose = $true;  TimeoutMin = 1;  WarnSecs = 30;  ForceKill = $true;  KillAfterSecs = 600; SettleSec = 75; Seconds = 60 }
)

$results = @()
foreach ($version in @('v080', 'v081')) {
    Log "=== $version ==="
    foreach ($s in $scenarios) {
        $results += Run-Scenario -Version $version @s
    }
}

$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$csv = Join-Path $resultsDir "results-$stamp.csv"
$results | Export-Csv -Path $csv -NoTypeInformation -Encoding Ascii
Log "Results written to $csv"
$results | Format-Table -AutoSize | Out-String | Write-Host
