# auto-idle-close

A tiny, portable Windows tray utility that gracefully closes applications you
left idle — pick the apps, set a timeout per app, and it takes care of the rest.

- **Portable**: one ~5.5 MB `auto-idle-close.exe`, no installer, no runtime,
  nothing else to ship. Settings live in `config.toml` next to the exe.
- **Graceful by default**: apps are closed the same way as clicking their ✕
  button (`WM_CLOSE`), never killed — unless you opt in per app.
- **Warned before it acts**: an always-on-top countdown popup appears before
  closing, with *Keep open* / *Close now* buttons.
- **Light**: 0% CPU while idle (1 s polling), single background thread.

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
   (warning length, pause, Explorer integration) and a collapsible *Recent
   activity* list.
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
