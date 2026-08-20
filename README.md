# auto-idle-close

A tiny, portable Windows tray utility that gracefully closes applications you
left idle — pick the apps, set a timeout per app, and it takes care of the rest.

- **Portable**: one ~8 MB `auto-idle-close.exe`, no runtime, nothing else to
  ship. Settings live in `config.toml` next to the exe. Prefer an installer?
  Each release also has `auto-idle-close-setup.exe` (per-user, no admin
  rights, Start menu shortcut, optional desktop icon and autostart, clean
  uninstall).
- **Graceful by default**: apps are closed the same way as clicking their ✕
  button (`WM_CLOSE`), never killed — unless you opt in per app.
- **Warned before it acts**: an always-on-top countdown popup appears before
  closing, with *Keep open* / *Close now* buttons.
- **Light**: 0% CPU while idle (1 s polling), single background thread.
- **Self-updating (opt-out)**: once a day the app asks github.com for the
  latest release tag and shows a banner when a newer version exists. Nothing
  is downloaded or installed without a click; the update is fetched from the
  GitHub release, verified against its `SHA256SUMS.txt`, swapped in place,
  and the app restarts. Disable in Settings → Updates.
- **Remote desktop friendly**: inside an RDP session the app automatically
  switches from OpenGL to a DirectX 12 (wgpu) renderer, since RDP exposes
  only OpenGL 1.1. Force a renderer with `--renderer glow` or
  `--renderer wgpu` if needed.

## How it works

An app counts as **idle** when it hasn't been *used* for its configured number
of minutes — "used" means its window is in the foreground **and** you are
actively giving keyboard/mouse input. An app left focused on an unattended PC
still idles out.

Lifecycle per monitored process: `Active → Warning (countdown popup) → WM_CLOSE
→ gone`. If the app refuses to close (e.g. an unsaved-changes dialog) it is
left alone and its timer restarts — unless *force after* is enabled for that
rule, in which case it is terminated after the configured grace period.

## Usage

1. Run `auto-idle-close.exe`. The main window lists running applications by
   window title (with a 🔍 filter box) — tick the ones to manage, tune the
   timeout per rule.
2. Apps that aren't running yet can be added by exe name (*Add by name*) or
   picked from disk (*Browse…*). Rules with a known path get a `…` menu with
   **Start now** and **Create desktop shortcut**.
3. The **⚙ Settings** button opens a separate window with global options
   (warning length, pause, language, start-with-Windows, Explorer integration)
   and a collapsible *Recent activity* list. *Start with Windows* registers a
   per-user Run entry that launches the app at login hidden in the tray
   (equivalent to running it with `--tray`).
4. Closing the window hides it to the tray; left-click the tray clock icon to
   reopen, right-click for Pause / Exit.
5. Settings save automatically to `config.toml` beside the exe (keep the exe in
   a writable folder).

### Running apps *through* auto-idle-close

To guarantee an app is always monitored — even on a fresh boot when the tray
app isn't running yet — start it through the wrapper:

```
auto-idle-close.exe --launch "C:\path\to\app.exe"
```

This starts the app, starts auto-idle-close if needed (or hands the request to
the running instance), and adds/enables a rule for it automatically. Dropping
an .exe onto `auto-idle-close.exe` does the same. Three convenient frontends:

- **Explorer right-click**: enable *"Run with auto-idle-close"* in Settings to
  get a context-menu entry on every .exe (Windows 11: under *Show more
  options*). Re-toggle it if you move `auto-idle-close.exe`.
- **Companion shortcuts**: the `⋯` menu on a rule creates an
  *"App (auto-close)"* desktop shortcut that launches the app via the wrapper.
- **Autostart pairing**: point any existing shortcut/startup entry at the
  wrapper command above instead of the app itself.

```toml
# config.toml
warning_seconds = 60        # countdown length before closing

[[rules]]
exe = "chrome.exe"          # case-insensitive exe file name
enabled = true
timeout_minutes = 30        # idle time before closing
force_kill = false          # terminate if WM_CLOSE is ignored?
force_kill_after_secs = 30
path = 'C:\Program Files\Google\Chrome\Application\chrome.exe'  # optional; enables Start/shortcuts
```

Elevated (admin) processes cannot be managed from a non-elevated instance and
are shown grayed out.

## Admin-managed policy (optional)

For managed environments (e.g. RDP terminal servers), an administrator can
enforce rules that users see but cannot edit, remove, or pause away. Drop a
`policy.toml` into `%ProgramData%\auto-idle-close\` (a location standard
users cannot modify) and every instance on the machine picks it up within
seconds — no restart needed. Without the file the app behaves exactly as
before. See [docs/admin-policy.md](docs/admin-policy.md) for the file format,
deployment, and hardening notes.

## Downloads, SmartScreen, and verifying releases

Two flavours per release, both containing the same exe:

- `auto-idle-close.exe` — portable; drop it anywhere, config stays next to it.
- `auto-idle-close-setup.exe` — installer; installs per-user to
  `%LOCALAPPDATA%\Programs\auto-idle-close` (no admin prompt), adds a Start
  menu shortcut, offers optional desktop icon and start-with-Windows during
  setup, and its uninstaller removes the app plus any registry entries the app
  created (autostart, Explorer context-menu verb) and its config.

Releases are unsigned (no paid code-signing certificate), so on first run of a
downloaded exe Windows SmartScreen may show *"Windows protected your PC"* —
this is a reputation check, not a malware detection. Click **More info → Run
anyway**. The warning fades on its own as a release accumulates downloads.

To verify a download, every release ships a `SHA256SUMS.txt`. In PowerShell:

```powershell
(Get-FileHash .\auto-idle-close.exe -Algorithm SHA256).Hash
# compare with the value in SHA256SUMS.txt
```

Each release also attaches ready-made **winget manifests**
(`MateuszCiosek.AutoIdleClose.*.yaml`) with the correct hash filled in, for
submission to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs)
— installing via winget avoids the SmartScreen download prompt entirely.

Longer term, [SignPath Foundation](https://signpath.org/) offers free code
signing to qualifying open-source projects, which gives releases a persistent
publisher reputation.

## Building

Requires Rust (stable) and the MSVC build tools.

```
cargo build --release       # -> target/release/auto-idle-close.exe
cargo test                  # config + idle state machine unit tests
```

Debug builds keep a console window and log panics; release builds are windowed.

## Code map

| File | Responsibility |
|---|---|
| `src/main.rs` | wiring: config, channels, monitor thread, eframe/tray startup |
| `src/monitor.rs` | the idle state machine (`Engine`, pure & unit-tested) + polling thread |
| `src/win.rs` | all unsafe Win32: window enumeration, foreground, WM_CLOSE, terminate, input idle |
| `src/config.rs` | TOML config schema, load/save |
| `src/ui.rs` | settings window, warning popup (deferred viewport), hide-to-tray |
| `src/tray.rs` | tray icon, menu, procedurally drawn clock icon |
| `src/launcher.rs` | `--launch` wrapper mode, spool handoff, Explorer context menu, shortcuts |

Ideas for extending: scheduled close times, CPU-busy guard before closing,
start-with-Windows toggle, per-rule warning length, event-hook based foreground
tracking (`SetWinEventHook`) instead of polling.
