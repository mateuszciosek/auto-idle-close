# Performance bench

Compares CPU usage of auto-idle-close builds under the terminal-server (RDP)
rendering path, without needing an actual RDP session: the app is forced onto
wgpu's WARP software rasterizer (`WGPU_ADAPTER_NAME=Microsoft Basic Render
Driver`), which is what real RDP sessions get (no GPU ICD), so every repaint
costs real CPU exactly like on the server.

Windows containers cannot host this (no interactive logon session, no tray,
no desktop), so this local harness plus a staging RDS host is the validation
story.

## Run

```powershell
powershell -File bench\stress.ps1            # full matrix, ~15 min
powershell -File bench\stress.ps1 -SkipBuild # reuse previously built exes
```

Builds `v0.8.0` from the git tag into `bench/bin/v080/` (one-time), the
working tree into `bench/bin/v081/`, and the victim helper. Results go to
`bench/results/*.csv` and the console.

## Scenarios

| Scenario | What it models |
|---|---|
| idle-4apps / idle-8apps | Users with 4 (or 8) rules, apps in use, nothing idling out — pure polling cost |
| warn-popup | Countdown popup on screen (4 apps warned at once) |
| after-close | The v0.8.0 killer: apps auto-closed successfully, popup gone — v0.8.0 then spins at 100% of a core forever (eframe ControlFlow::Poll bug) |
| close-cycle | Unattended/disconnected session: app ignores WM_CLOSE, warn -> close -> fail -> re-warn forever |
| stuck-closing | force_kill pending against an app that won't die (elevated app on v0.8.0 = popup stuck forever) |

The victim (`bench/victim`) is a plain Win32 window; `--ignore-close`
makes it swallow `WM_CLOSE` like an app sitting on an unsaved-changes dialog.
It is shown without activation so it never steals foreground (a foreground
window with recent input would never idle out).

Measurement is `TotalProcessorTime` delta over the window, reported as % of
one core, plus working set. Absolute numbers are machine-dependent; the
v080-vs-v081 ratio is the signal.
