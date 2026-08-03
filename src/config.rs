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
    pub rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            warning_seconds: 60,
            poll_interval_ms: 1000,
            paused: false,
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
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            exe: String::new(),
            enabled: true,
            timeout_minutes: 30,
            force_kill: false,
            force_kill_after_secs: 30,
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
            rules: vec![Rule {
                exe: "notepad.exe".into(),
                enabled: true,
                timeout_minutes: 5,
                force_kill: true,
                force_kill_after_secs: 15,
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
