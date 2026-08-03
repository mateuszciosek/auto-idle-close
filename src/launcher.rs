//! "Run with auto-idle-close" wrapper support.
//!
//! `auto-idle-close.exe --launch <path-to-exe>` starts the target application
//! AND guarantees it is bound to an enabled rule:
//! - if no tray instance is running, this process becomes it (rule added
//!   before monitoring starts);
//! - if one is already running, the request is handed over through a small
//!   spool file next to the exe, which the monitor thread drains every tick.
//!
//! The same entry point powers the optional Explorer right-click menu entry
//! and per-rule companion shortcuts.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Prevents a console window from flashing for spawned helper processes.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn spool_path() -> PathBuf {
    exe_dir().join("launch-queue.txt")
}

/// Parse a `--launch <path>` request (also accepts a bare .exe path, so
/// drag-and-drop onto auto-idle-close.exe works).
pub fn launch_target_from_args() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let raw = match args.iter().position(|a| a == "--launch") {
        Some(i) => args.get(i + 1)?.clone(),
        None => args
            .iter()
            .find(|a| !a.starts_with('-') && a.to_lowercase().ends_with(".exe"))?
            .clone(),
    };
    let path = PathBuf::from(raw);
    let path = path.canonicalize().unwrap_or(path);
    path.is_file().then_some(strip_verbatim(path))
}

/// canonicalize() yields \\?\C:\... paths; plain form is friendlier in config.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

/// Start the target application (best effort).
pub fn start_target(path: &Path) -> bool {
    let mut cmd = Command::new(path);
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn().is_ok()
}

/// Hand a bind request to the already-running instance.
pub fn append_to_spool(path: &Path) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(spool_path())
    {
        let _ = writeln!(f, "{}", path.display());
    }
}

/// Drain pending bind requests (called by the monitor thread every tick).
pub fn drain_spool() -> Vec<PathBuf> {
    let path = spool_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let _ = std::fs::remove_file(&path);
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

// ---------------------------------------------------------------------------
// Explorer right-click integration ("Run with auto-idle-close" on .exe files)
// ---------------------------------------------------------------------------

const VERB_KEY: &str = r"HKCU\Software\Classes\exefile\shell\AutoIdleClose";

fn reg(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("reg")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn context_menu_installed() -> bool {
    reg(&["query", VERB_KEY, "/ve"])
}

pub fn install_context_menu() -> bool {
    let self_exe = std::env::current_exe().unwrap_or_default();
    let self_exe = self_exe.to_string_lossy();
    let command = format!("\"{self_exe}\" --launch \"%1\"");
    reg(&[
        "add", VERB_KEY, "/ve", "/d", "Run with auto-idle-close", "/f",
    ]) && reg(&["add", VERB_KEY, "/v", "Icon", "/d", &self_exe, "/f"])
        && reg(&[
            "add",
            &format!(r"{VERB_KEY}\command"),
            "/ve",
            "/d",
            &command,
            "/f",
        ])
}

pub fn uninstall_context_menu() -> bool {
    reg(&["delete", VERB_KEY, "/f"])
}

// ---------------------------------------------------------------------------
// Companion desktop shortcuts ("MyApp (auto-close).lnk")
// ---------------------------------------------------------------------------

/// Create a desktop shortcut that starts `target` through the wrapper, so the
/// pairing works even when auto-idle-close isn't running yet.
pub fn create_companion_shortcut(target: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    let self_exe = std::env::current_exe().unwrap_or_default();
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "app".into());
    // PowerShell resolves the real Desktop folder (handles OneDrive relocation).
    let ps = format!(
        "$desktop = [Environment]::GetFolderPath('Desktop'); \
         $ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut((Join-Path $desktop '{name} (auto-close).lnk')); \
         $s.TargetPath = '{wrapper}'; \
         $s.Arguments = '--launch \"{target}\"'; \
         $s.IconLocation = '{target},0'; \
         $s.WorkingDirectory = '{workdir}'; \
         $s.Save()",
        name = ps_quote(&stem),
        wrapper = ps_quote(&self_exe.to_string_lossy()),
        target = ps_quote(&target.to_string_lossy()),
        workdir = ps_quote(
            &target
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default()
        ),
    );
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Escape for inclusion inside a single-quoted PowerShell string.
fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_verbatim_prefix() {
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\Tools\app.exe")),
            PathBuf::from(r"C:\Tools\app.exe")
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(r"C:\Tools\app.exe")),
            PathBuf::from(r"C:\Tools\app.exe")
        );
    }

    #[test]
    fn ps_quote_escapes_single_quotes() {
        assert_eq!(ps_quote(r"C:\It's Here\app.exe"), r"C:\It''s Here\app.exe");
    }

    /// Creates and removes a desktop shortcut — run explicitly with --ignored.
    #[test]
    #[ignore]
    fn companion_shortcut_roundtrip() {
        use std::os::windows::process::CommandExt;
        let target = Path::new(r"C:\Windows\System32\charmap.exe");
        assert!(create_companion_shortcut(target), "shortcut creation failed");
        let check = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$p = Join-Path ([Environment]::GetFolderPath('Desktop')) 'charmap (auto-close).lnk'; \
                 if (Test-Path $p) { Remove-Item $p -Force; exit 0 } else { exit 1 }",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(check.status.success(), "shortcut not found on desktop");
    }

    /// Mutates HKCU — run explicitly with: cargo test -- --ignored
    #[test]
    #[ignore]
    fn context_menu_roundtrip() {
        assert!(install_context_menu(), "install failed");
        assert!(context_menu_installed(), "not detected after install");
        assert!(uninstall_context_menu(), "uninstall failed");
        assert!(!context_menu_installed(), "still detected after uninstall");
    }
}
