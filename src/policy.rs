//! Optional admin-managed policy layer.
//!
//! Admins can drop a `policy.toml` into `%ProgramData%\auto-idle-close\`
//! (a location standard users cannot modify) to enforce rules that the UI
//! shows but never edits. When the file is absent the app behaves exactly
//! as without this feature. The file is re-checked every few seconds so
//! policy changes apply without restarting every session.
//!
//! `AUTO_IDLE_CLOSE_POLICY` overrides the path (for testing).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::config::{Config, Rule};

/// How often the policy file is re-checked for changes (cheap stat call).
const RECHECK: Duration = Duration::from_secs(10);

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Policy {
    /// When false, users cannot pause monitoring (UI control disabled,
    /// the flag is forced off).
    pub allow_pause: bool,
    /// When false, the tray "Exit" item is disabled.
    pub allow_exit: bool,
    /// Enforced rules; they shadow user rules for the same exe and cannot
    /// be edited or removed in the UI.
    pub rules: Vec<Rule>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            allow_pause: true,
            allow_exit: true,
            rules: Vec::new(),
        }
    }
}

impl Policy {
    pub fn rule_for(&self, exe: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.exe.eq_ignore_ascii_case(exe))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Load {
    /// No policy file — the feature is off.
    Absent,
    Loaded(Policy),
    /// The file exists but could not be read or parsed. Fails open (no
    /// rules enforced) but the settings window surfaces the problem.
    Error(String),
}

pub fn policy_path() -> PathBuf {
    if let Ok(p) = std::env::var("AUTO_IDLE_CLOSE_POLICY") {
        return PathBuf::from(p);
    }
    let base = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".into());
    PathBuf::from(base).join("auto-idle-close").join("policy.toml")
}

/// (mtime, size) — enough to notice edits without reading the file.
fn stamp() -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(policy_path()).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn read_now() -> Load {
    match std::fs::read_to_string(policy_path()) {
        Ok(text) => match toml::from_str::<Policy>(&text) {
            Ok(policy) => Load::Loaded(policy),
            Err(e) => Load::Error(e.to_string()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Load::Absent,
        Err(e) => Load::Error(e.to_string()),
    }
}

struct Inner {
    load: Load,
    stamp: Option<(SystemTime, u64)>,
    next_check: Instant,
}

/// Shared, self-refreshing view of the policy file.
pub struct Handle {
    inner: Mutex<Inner>,
}

impl Handle {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                load: read_now(),
                stamp: stamp(),
                next_check: Instant::now() + RECHECK,
            }),
        }
    }

    /// Reload the file if the recheck interval passed and it changed on
    /// disk (or a previous read failed). Returns true when the effective
    /// policy changed.
    pub fn refresh_if_due(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();
        if now < inner.next_check {
            return false;
        }
        inner.next_check = now + RECHECK;
        let current = stamp();
        if current == inner.stamp && !matches!(inner.load, Load::Error(_)) {
            return false;
        }
        let load = read_now();
        let changed = load != inner.load;
        inner.load = load;
        inner.stamp = current;
        changed
    }

    pub fn snapshot(&self) -> Load {
        self.inner.lock().unwrap().load.clone()
    }

    pub fn policy(&self) -> Option<Policy> {
        match &self.inner.lock().unwrap().load {
            Load::Loaded(p) => Some(p.clone()),
            _ => None,
        }
    }

    pub fn is_managed(&self, exe: &str) -> bool {
        match &self.inner.lock().unwrap().load {
            Load::Loaded(p) => p.rule_for(exe).is_some(),
            _ => false,
        }
    }

    pub fn allow_pause(&self) -> bool {
        match &self.inner.lock().unwrap().load {
            Load::Loaded(p) => p.allow_pause,
            _ => true,
        }
    }

    pub fn allow_exit(&self) -> bool {
        match &self.inner.lock().unwrap().load {
            Load::Loaded(p) => p.allow_exit,
            _ => true,
        }
    }
}

/// The config the monitor actually runs on: policy rules first (they shadow
/// user rules for the same exe), user rules for everything else, and pause
/// stripped when the policy forbids it.
pub fn effective_config(user: &Config, policy: Option<&Policy>) -> Config {
    let Some(policy) = policy else {
        return user.clone();
    };
    let mut cfg = user.clone();
    if !policy.allow_pause {
        cfg.paused = false;
    }
    let mut rules = policy.rules.clone();
    rules.extend(
        user.rules
            .iter()
            .filter(|r| policy.rule_for(&r.exe).is_none())
            .cloned(),
    );
    cfg.rules = rules;
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_allow_everything() {
        let p: Policy = toml::from_str("").unwrap();
        assert!(p.allow_pause);
        assert!(p.allow_exit);
        assert!(p.rules.is_empty());
    }

    #[test]
    fn parses_minimal_policy() {
        let p: Policy = toml::from_str(
            r#"
            allow_pause = false

            [[rules]]
            exe = "comarch.exe"
            timeout_minutes = 15
            "#,
        )
        .unwrap();
        assert!(!p.allow_pause);
        assert!(p.allow_exit);
        assert_eq!(p.rules.len(), 1);
        assert_eq!(p.rules[0].exe, "comarch.exe");
        assert_eq!(p.rules[0].timeout_minutes, 15);
        assert!(p.rules[0].enabled);
    }

    #[test]
    fn no_policy_is_identity() {
        let user = Config {
            paused: true,
            rules: vec![Rule { exe: "a.exe".into(), ..Default::default() }],
            ..Default::default()
        };
        assert_eq!(effective_config(&user, None), user);
    }

    #[test]
    fn policy_rules_shadow_user_rules_case_insensitively() {
        let mut user = Config::default();
        user.rules.push(Rule {
            exe: "Comarch.EXE".into(),
            timeout_minutes: 1440,
            enabled: false,
            ..Default::default()
        });
        user.rules.push(Rule { exe: "other.exe".into(), ..Default::default() });

        let policy = Policy {
            rules: vec![Rule {
                exe: "comarch.exe".into(),
                timeout_minutes: 15,
                ..Default::default()
            }],
            ..Default::default()
        };

        let eff = effective_config(&user, Some(&policy));
        assert_eq!(eff.rules.len(), 2);
        assert_eq!(eff.rules[0].exe, "comarch.exe");
        assert_eq!(eff.rules[0].timeout_minutes, 15);
        assert!(eff.rules[0].enabled);
        assert_eq!(eff.rules[1].exe, "other.exe");
    }

    #[test]
    fn disallowed_pause_is_stripped() {
        let user = Config { paused: true, ..Default::default() };

        let allows = Policy::default();
        assert!(effective_config(&user, Some(&allows)).paused);

        let forbids = Policy { allow_pause: false, ..Default::default() };
        assert!(!effective_config(&user, Some(&forbids)).paused);
    }
}
