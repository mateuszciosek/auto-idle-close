# Admin-managed policy

An optional layer for managed environments — typically RDP terminal servers
where auto-idle-close guards license seats of a shared application. An
administrator enforces rules centrally; users still run their own per-user
installs and can add their *own* rules, but cannot edit, remove, disable, or
pause away the enforced ones.

**Without a policy file, nothing changes** — the feature is completely
opt-in and invisible.

## How it works

At startup, and every ~10 seconds afterwards, the app looks for:

```
%ProgramData%\auto-idle-close\policy.toml
```

(normally `C:\ProgramData\auto-idle-close\policy.toml`). If present:

- Its rules are **enforced**: they appear in the Rules list grayed out with a
  *"managed by your administrator"* label — no delete button, no editable
  timeout. A user rule for the same exe (added before the policy existed) is
  shadowed and hidden.
- Users can still add and manage their own rules for *other* programs.
- The security boundary is the NTFS ACL on the file, not the app: standard
  users cannot modify admin-created files under `ProgramData`, so they cannot
  loosen the policy. There is no in-memory loophole either — the monitor
  merges the policy on every poll tick, and the UI has no code path that
  edits policy rules.

Changes to the file apply to running instances within seconds — users do not
need to log off.

## File format

Same rule schema as `config.toml`:

```toml
# C:\ProgramData\auto-idle-close\policy.toml

# Optional switches (both default to true):
allow_pause = false   # false: users cannot pause monitoring at all
allow_exit  = false   # false: the tray "Exit" item is grayed out

[[rules]]
exe = "comarch.exe"        # case-insensitive exe file name
timeout_minutes = 15       # idle minutes before closing
force_kill = true          # terminate if WM_CLOSE is ignored
force_kill_after_secs = 60
# path = 'C:\Comarch\comarch.exe'   # optional; enables "Start now"
```

- `allow_pause = false` also force-clears an already-set pause in every
  session.
- `allow_exit = false` only grays out the menu item. It raises the bar, but a
  determined user can still end the process via Task Manager — see
  *Keeping the app running* below.
- If the file exists but cannot be read or parsed, the policy is **not**
  enforced and the Settings window shows the error (in red, next to the
  policy path). Users see the same message, so a broken file is noticed.

## Deployment

1. As an administrator, create the folder and file:

   ```powershell
   New-Item -ItemType Directory -Force "$env:ProgramData\auto-idle-close"
   # then place policy.toml inside
   ```

   Files created by an administrator under `ProgramData` are not writable or
   deletable by standard users with default ACLs. To harden explicitly
   (read-only for users, full control for administrators):

   ```powershell
   icacls "$env:ProgramData\auto-idle-close" /inheritance:r `
     /grant "Administrators:(OI)(CI)F" "SYSTEM:(OI)(CI)F" "Users:(OI)(CI)RX"
   ```

2. Distribute the file however you already manage the machines: manually,
   a login script, Group Policy Preferences (Files), Intune, etc. It is one
   static file per machine — nothing else to install.

3. Users keep their normal per-user installs (`auto-idle-close-setup.exe`);
   self-update keeps working because the exe stays in a user-writable folder.

### Testing without admin rights

The `AUTO_IDLE_CLOSE_POLICY` environment variable overrides the path, so you
can point a test instance at any file:

```powershell
$env:AUTO_IDLE_CLOSE_POLICY = "C:\temp\policy.toml"
.\auto-idle-close.exe
```

## Keeping the app running

A user can always end the process (Task Manager, `taskkill`), which no
unprivileged tray app can prevent. The standard mitigation is an external
watchdog — a per-machine Scheduled Task that restarts the app if it is not
running. Example (run once as admin; adjust the exe path pattern to your
install location):

```powershell
$action  = New-ScheduledTaskAction -Execute "$env:LOCALAPPDATA\Programs\auto-idle-close\auto-idle-close.exe" -Argument "--tray"
$trigger = New-ScheduledTaskTrigger -AtLogOn
$trigger.Repetition = (New-ScheduledTaskTrigger -Once -At 0am -RepetitionInterval (New-TimeSpan -Minutes 5)).Repetition
Register-ScheduledTask -TaskName "auto-idle-close watchdog" -Action $action -Trigger $trigger
```

The app is single-instance per session, so re-launch attempts while it is
already running exit immediately and are harmless.
