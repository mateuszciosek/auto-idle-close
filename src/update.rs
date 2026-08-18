//! Self-update from GitHub releases — no server of our own.
//!
//! Check: one HTTPS HEAD following the `/releases/latest` redirect; the
//! final URL carries the tag, so no API quota or JSON parsing is involved.
//! Install (only on explicit user click): download the portable exe and
//! `SHA256SUMS.txt` from that release, verify the checksum, swap the running
//! exe via the rename dance (a running exe can be renamed but not
//! overwritten), then restart through a detached waiter process.
//! Everything shells out to tools present on every Windows 10+ install
//! (curl.exe, certutil.exe, powershell.exe) — no new dependencies.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;

const REPO: &str = "mateuszciosek/auto-idle-close";
const EXE_ASSET: &str = "auto-idle-close.exe";
const SUMS_ASSET: &str = "SHA256SUMS.txt";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// A failed background check (login before the network is up) retries sooner.
const RETRY_AFTER: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    /// Holds the release tag, e.g. "v0.7.0".
    Available(String),
    Downloading,
    Restarting,
    Failed(String),
}

#[derive(Default)]
pub struct UpdateState {
    status: Mutex<Option<Status>>, // None until first initialized below
    /// When the next background check is due; `None` means immediately.
    pub next_check: Mutex<Option<Instant>>,
    /// Set once the new exe is in place; the UI runs its exit flow, and the
    /// detached waiter starts the new version after this process dies.
    pub restart_ready: AtomicBool,
    busy: AtomicBool,
}

impl UpdateState {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone().unwrap_or(Status::Idle)
    }

    fn set_status(&self, s: Status) {
        *self.status.lock().unwrap() = Some(s);
    }
}

pub fn release_page_url() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

pub fn open_release_page() {
    let _ = Command::new("explorer.exe").arg(release_page_url()).spawn();
}

/// Query GitHub in the background and update `state.status`.
/// `background` checks fail silently (back to Idle) and retry sooner.
pub fn spawn_check(state: Arc<UpdateState>, ctx: egui::Context, background: bool) {
    if state.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    *state.next_check.lock().unwrap() = Some(Instant::now() + CHECK_EVERY);
    state.set_status(Status::Checking);
    ctx.request_repaint();
    std::thread::spawn(move || {
        let status = match latest_tag() {
            Ok(tag) if is_newer(&tag, current_version()) => Status::Available(tag),
            Ok(_) => Status::UpToDate,
            Err(_) if background => {
                *state.next_check.lock().unwrap() = Some(Instant::now() + RETRY_AFTER);
                Status::Idle
            }
            Err(e) => Status::Failed(e),
        };
        state.set_status(status);
        state.busy.store(false, Ordering::SeqCst);
        ctx.request_repaint();
    });
}

/// Download, verify, and stage the release `tag`, then flag for restart.
pub fn spawn_install(state: Arc<UpdateState>, ctx: egui::Context, tag: String) {
    if state.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    state.set_status(Status::Downloading);
    ctx.request_repaint();
    std::thread::spawn(move || {
        let result = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| install_at(&exe, &tag));
        match result {
            Ok(exe) => {
                spawn_relauncher(&exe);
                state.set_status(Status::Restarting);
                state.restart_ready.store(true, Ordering::SeqCst);
            }
            Err(e) => state.set_status(Status::Failed(e)),
        }
        state.busy.store(false, Ordering::SeqCst);
        ctx.request_repaint();
    });
}

/// Remove leftovers of a previous update (best effort; a still-exiting old
/// instance keeps `.old` locked — it goes away on the next start instead).
pub fn cleanup_leftovers() {
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        let _ = std::fs::remove_file(dir.join(format!("{EXE_ASSET}.old")));
        let _ = std::fs::remove_file(dir.join(format!("{EXE_ASSET}.new")));
        let _ = std::fs::remove_file(dir.join(format!("{SUMS_ASSET}.new")));
    }
}

// ---------------------------------------------------------------------------
// Pieces
// ---------------------------------------------------------------------------

fn current_version() -> &'static str {
    // Testing hook: lets a sandboxed copy pretend to be older than the
    // latest release without rebuilding.
    static OVERRIDE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    OVERRIDE
        .get_or_init(|| std::env::var("AUTO_IDLE_CLOSE_FAKE_VERSION").ok())
        .as_deref()
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn run_hidden(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    use std::os::windows::process::CommandExt;
    Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|_| format!("{program} not found"))
}

fn curl(args: &[&str]) -> Result<String, String> {
    let out = run_hidden("curl.exe", args)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("network error (curl exit {})", out.status.code().unwrap_or(-1)))
    }
}

/// Tag of the latest release, e.g. "v0.7.0", read from the redirect target
/// of the stable `/releases/latest` URL.
fn latest_tag() -> Result<String, String> {
    let effective = curl(&[
        "-fsIL", "-o", "NUL", "-w", "%{url_effective}", "-m", "15",
        &release_page_url(),
    ])?;
    effective
        .trim()
        .rsplit('/')
        .next()
        .filter(|t| parse_ver(t).is_some())
        .map(str::to_string)
        .ok_or_else(|| "unexpected release URL".to_string())
}

fn parse_ver(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.trim().trim_start_matches('v').split('.');
    let ver = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(ver)
}

fn is_newer(tag: &str, current: &str) -> bool {
    matches!((parse_ver(tag), parse_ver(current)), (Some(t), Some(c)) if t > c)
}

/// Download + verify + swap. Returns the exe path to relaunch.
/// Split out from `spawn_install` (and given the exe path explicitly) so the
/// whole flow short of relaunching is testable against a sandbox copy.
fn install_at(exe: &Path, tag: &str) -> Result<PathBuf, String> {
    let dir = exe.parent().ok_or("exe has no parent directory")?;
    let staged = dir.join(format!("{EXE_ASSET}.new"));
    let sums_file = dir.join(format!("{SUMS_ASSET}.new"));
    let base = format!("https://github.com/{REPO}/releases/download/{tag}");

    let fetch = |url: String, to: &Path, timeout: &str| -> Result<(), String> {
        let to = to.to_str().ok_or("non-unicode path")?;
        curl(&["-fsSL", "-m", timeout, "-o", to, &url]).map(|_| ())
    };
    let result = (|| {
        fetch(format!("{base}/{EXE_ASSET}"), &staged, "300")?;
        fetch(format!("{base}/{SUMS_ASSET}"), &sums_file, "60")?;
        let sums = std::fs::read_to_string(&sums_file).map_err(|e| e.to_string())?;
        let expected = expected_hash(&sums).ok_or("release has no checksum for the exe")?;
        let actual = file_sha256(&staged)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("checksum mismatch".to_string());
        }

        let old = dir.join(format!("{EXE_ASSET}.old"));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old).map_err(|e| format!("cannot move current exe: {e}"))?;
        if let Err(e) = std::fs::rename(&staged, exe) {
            let _ = std::fs::rename(&old, exe); // roll back
            return Err(format!("cannot place new exe: {e}"));
        }
        refresh_installed_version(dir, tag);
        Ok(())
    })();

    let _ = std::fs::remove_file(&sums_file);
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result.map(|()| exe.to_path_buf())
}

/// First token of the SHA256SUMS.txt line naming the portable exe.
fn expected_hash(sums: &str) -> Option<&str> {
    sums.lines().find_map(|line| {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        (it.next()?.trim_start_matches('*') == EXE_ASSET).then_some(hash)
    })
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let out = run_hidden(
        "certutil",
        &["-hashfile", path.to_str().ok_or("non-unicode path")?, "SHA256"],
    )?;
    if !out.status.success() {
        return Err("certutil failed".to_string());
    }
    // Line 1 is a header; line 2 is the hex digest (spaced on old builds).
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .nth(1)
        .map(|l| l.split_whitespace().collect::<String>())
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| "unexpected certutil output".to_string())
}

/// Keep the Add/Remove Programs version in sync for the installed flavour.
fn refresh_installed_version(dir: &Path, tag: &str) {
    if !dir.join("unins000.exe").exists() {
        return;
    }
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{B7A9E3D4-6C2F-4E8B-9A51-0D7C3F2A8E64}_is1";
    let _ = run_hidden(
        "reg",
        &["add", KEY, "/v", "DisplayVersion", "/d", tag.trim_start_matches('v'), "/f"],
    );
}

/// Detached helper that starts the new exe once this process has exited
/// (the single-instance mutex is only released then).
fn spawn_relauncher(exe: &Path) {
    use std::os::windows::process::CommandExt;
    let ps = format!(
        "Wait-Process -Id {} -ErrorAction SilentlyContinue; Start-Process -FilePath '{}'",
        std::process::id(),
        exe.to_string_lossy().replace('\'', "''"),
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing_and_ordering() {
        assert_eq!(parse_ver("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_ver("0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_ver("v1.2"), None);
        assert_eq!(parse_ver("v1.2.3.4"), None);
        assert_eq!(parse_ver("latest"), None);
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("v0.6.1", "0.6.1"));
        assert!(!is_newer("v0.6.0", "0.6.1"));
        assert!(!is_newer("garbage", "0.6.1"));
    }

    #[test]
    fn expected_hash_finds_the_exe_line() {
        let sums = "abc123  auto-idle-close-setup.exe\n\
                    deadbeef *auto-idle-close.exe\n";
        assert_eq!(expected_hash(sums), Some("deadbeef"));
        assert_eq!(expected_hash("nothing relevant"), None);
    }

    #[test]
    fn certutil_hashes_a_known_file() {
        let path = std::env::temp_dir().join("aic-update-hash-test.txt");
        std::fs::write(&path, "abc").unwrap();
        let hash = file_sha256(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            hash.to_lowercase(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Talks to github.com — run explicitly with: cargo test -- --ignored
    #[test]
    #[ignore]
    fn latest_tag_live() {
        let tag = latest_tag().expect("check failed");
        assert!(parse_ver(&tag).is_some(), "unexpected tag {tag}");
    }

    /// Full download/verify/swap against the real latest release, in a
    /// sandbox directory (does not touch the running test binary and does
    /// not relaunch anything). Run explicitly with: cargo test -- --ignored
    #[test]
    #[ignore]
    fn install_flow_in_sandbox() {
        let dir = std::env::temp_dir().join("aic-update-e2e");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(EXE_ASSET);
        std::fs::write(&exe, "pretend old exe").unwrap();

        let tag = latest_tag().expect("check failed");
        let swapped = install_at(&exe, &tag).expect("install failed");

        assert_eq!(swapped, exe);
        assert!(exe.metadata().unwrap().len() > 1_000_000, "new exe too small");
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("{EXE_ASSET}.old"))).unwrap(),
            "pretend old exe"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
