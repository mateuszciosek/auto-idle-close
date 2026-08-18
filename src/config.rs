use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Seconds of countdown warning shown before an app is closed.
    pub warning_seconds: u32,
    pub poll_interval_ms: u64,
    pub paused: bool,
    /// UI language; "auto" follows the Windows display language.
    pub language: crate::i18n::Language,
    /// Daily background check for a newer GitHub release (check only —
    /// installing always takes an explicit click).
    pub check_updates: bool,
    pub rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            warning_seconds: 60,
            poll_interval_ms: 1000,
            paused: false,
            language: crate::i18n::Language::Auto,
            check_updates: true,
            rules: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Rule {
    /// Executable file name, matched case-insensitively (e.g. "notepad.exe").
    pub exe: String,
    pub enabled: bool,
    pub timeout_minutes: u32,
    pub force_kill: bool,
    pub force_kill_after_secs: u32,
    /// Full path of the executable when known (picked from disk, launched via
    /// `--launch`, or seen running). Enables "Start" and companion shortcuts.
    pub path: Option<String>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            exe: String::new(),
            enabled: true,
            timeout_minutes: 30,
            force_kill: false,
            force_kill_after_secs: 30,
            path: None,
        }
    }
}

impl Config {
    pub fn rule_for(&self, exe: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.exe.eq_ignore_ascii_case(exe))
    }

    pub fn rule_for_mut(&mut self, exe: &str) -> Option<&mut Rule> {
        self.rules.iter_mut().find(|r| r.exe.eq_ignore_ascii_case(exe))
    }

    /// Upsert a rule for an executable path: creates it if missing, enables it,
    /// and remembers the full path. Returns the exe file name it bound.
    pub fn bind_path(&mut self, path: &std::path::Path) -> Option<String> {
        let exe = path.file_name()?.to_string_lossy().to_lowercase();
        match self.rule_for_mut(&exe) {
            Some(rule) => {
                rule.enabled = true;
                rule.path = Some(path.to_string_lossy().into_owned());
            }
            None => self.rules.push(Rule {
                exe: exe.clone(),
                path: Some(path.to_string_lossy().into_owned()),
                ..Default::default()
            }),
        }
        Some(exe)
    }
}

/// Config lives next to the exe so the whole app stays portable.
pub fn config_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.toml")))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

pub fn load_or_default() -> Config {
    match std::fs::read_to_string(config_path()) {
        Ok(text) => toml::from_str(&text).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save(cfg: &Config) -> io::Result<()> {
    let path = config_path();
    let tmp = path.with_extension("toml.tmp");
    let text = toml::to_string(cfg).map_err(io::Error::other)?;
    std::fs::write(&tmp, text)?;
    // On Windows std::fs::rename replaces an existing destination.
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cfg = Config {
            warning_seconds: 30,
            poll_interval_ms: 500,
            paused: true,
            language: crate::i18n::Language::Pl,
            check_updates: false,
            rules: vec![Rule {
                exe: "notepad.exe".into(),
                enabled: true,
                timeout_minutes: 5,
                force_kill: true,
                force_kill_after_secs: 15,
                path: Some(r"C:\Windows\notepad.exe".into()),
            }],
        };
        let text = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let cfg: Config = toml::from_str(
            r#"
            [[rules]]
            exe = "chrome.exe"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.warning_seconds, 60);
        assert_eq!(cfg.rules.len(), 1);
        assert_eq!(cfg.rules[0].exe, "chrome.exe");
        assert!(cfg.rules[0].enabled);
        assert_eq!(cfg.rules[0].timeout_minutes, 30);
        assert!(!cfg.rules[0].force_kill);
    }

    #[test]
    fn corrupt_file_falls_back() {
        let cfg: Config = toml::from_str("this is { not toml").unwrap_or_default();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn bind_path_upserts_and_enables() {
        let mut cfg = Config::default();
        // Creates a new rule with the path remembered.
        let exe = cfg.bind_path(std::path::Path::new(r"C:\Tools\MyApp.EXE"));
        assert_eq!(exe.as_deref(), Some("myapp.exe"));
        assert_eq!(cfg.rules.len(), 1);
        assert!(cfg.rules[0].enabled);
        assert_eq!(cfg.rules[0].path.as_deref(), Some(r"C:\Tools\MyApp.EXE"));

        // Re-binding an existing (disabled) rule enables it and updates the path.
        cfg.rules[0].enabled = false;
        let exe = cfg.bind_path(std::path::Path::new(r"D:\Other\myapp.exe"));
        assert_eq!(exe.as_deref(), Some("myapp.exe"));
        assert_eq!(cfg.rules.len(), 1);
        assert!(cfg.rules[0].enabled);
        assert_eq!(cfg.rules[0].path.as_deref(), Some(r"D:\Other\myapp.exe"));
    }

    #[test]
    fn rule_lookup_is_case_insensitive() {
        let mut cfg = Config::default();
        cfg.rules.push(Rule {
            exe: "Notepad.EXE".into(),
            ..Default::default()
        });
        assert!(cfg.rule_for("notepad.exe").is_some());
        assert!(cfg.rule_for("other.exe").is_none());
    }
}
