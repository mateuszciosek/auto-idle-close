//! Idle-monitoring engine. `Engine` is pure (no Win32, injectable time) so the
//! state machine is unit-testable; `run` is the thread wrapper doing real I/O.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::win;

/// How long we wait for an app to react to WM_CLOSE (when force-kill is off)
/// before giving up and resetting its idle timer.
const CLOSE_GRACE: Duration = Duration::from_secs(10);

pub enum MonitorCmd {
    KeepOpen(u32),
    CloseNow(u32),
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum MonitorEvent {
    WarningStarted { pid: u32, exe: String, deadline: Instant },
    WarningCancelled { pid: u32 },
    Closed { pid: u32, exe: String, forced: bool },
    CloseFailed { pid: u32, exe: String },
    /// An app was bound (rule added/enabled) via the `--launch` wrapper.
    LaunchBound { exe: String },
}

/// Everything the engine needs to know about the outside world for one tick.
pub struct WorldSnapshot {
    /// pid -> lowercase exe file name, for every visible top-level window's process.
    pub exe_by_pid: HashMap<u32, String>,
    /// pid -> its top-level window handles.
    pub hwnds_by_pid: HashMap<u32, Vec<isize>>,
    pub foreground_pid: Option<u32>,
    /// True when the user gave keyboard/mouse input recently. Being foreground
    /// only counts as "in use" while this is true — otherwise the last-focused
    /// app on an unattended PC would never idle out.
    pub input_recent: bool,
    /// Liveness of pids currently in the Closing state (they may have no windows).
    pub closing_alive: HashMap<u32, bool>,
}

pub enum Action {
    PostClose(Vec<isize>),
    Terminate(u32),
}

enum State {
    Active { last_fg: Instant },
    Warning { deadline: Instant },
    Closing { since: Instant, killed: bool },
}

struct Entry {
    exe: String,
    state: State,
    hwnds: Vec<isize>,
}

#[derive(Default)]
pub struct Engine {
    procs: HashMap<u32, Entry>,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn closing_pids(&self) -> Vec<u32> {
        self.procs
            .iter()
            .filter(|(_, e)| matches!(e.state, State::Closing { .. }))
            .map(|(pid, _)| *pid)
            .collect()
    }

    /// "Keep open" from the warning popup: reset the idle timer.
    pub fn keep_open(&mut self, pid: u32, now: Instant, events: &mut Vec<MonitorEvent>) {
        if let Some(entry) = self.procs.get_mut(&pid) {
            match entry.state {
                State::Warning { .. } => {
                    entry.state = State::Active { last_fg: now };
                    events.push(MonitorEvent::WarningCancelled { pid });
                }
                State::Active { .. } => entry.state = State::Active { last_fg: now },
                State::Closing { .. } => {} // too late
            }
        }
    }

    /// "Close now" from the warning popup.
    pub fn close_now(&mut self, pid: u32, now: Instant, actions: &mut Vec<Action>) {
        if let Some(entry) = self.procs.get_mut(&pid) {
            if !matches!(entry.state, State::Closing { .. }) {
                actions.push(Action::PostClose(entry.hwnds.clone()));
                entry.state = State::Closing { since: now, killed: false };
            }
        }
    }

    pub fn tick(
        &mut self,
        now: Instant,
        snap: &WorldSnapshot,
        cfg: &Config,
        events: &mut Vec<MonitorEvent>,
        actions: &mut Vec<Action>,
    ) {
        // 1. Drop entries that vanished, lost their rule, or got their pid reused.
        self.procs.retain(|pid, entry| {
            if let State::Closing { killed, .. } = entry.state {
                // A closing app may have no windows left; judge by process liveness.
                if snap.closing_alive.get(pid).copied().unwrap_or(false) {
                    return true;
                }
                events.push(MonitorEvent::Closed {
                    pid: *pid,
                    exe: entry.exe.clone(),
                    forced: killed,
                });
                return false;
            }
            let still_matches = snap.exe_by_pid.get(pid) == Some(&entry.exe)
                && cfg.rule_for(&entry.exe).is_some_and(|r| r.enabled);
            if !still_matches && matches!(entry.state, State::Warning { .. }) {
                events.push(MonitorEvent::WarningCancelled { pid: *pid });
            }
            still_matches
        });

        // 2. Track new processes matching an enabled rule.
        for (pid, exe) in &snap.exe_by_pid {
            if !self.procs.contains_key(pid)
                && cfg.rule_for(exe).is_some_and(|r| r.enabled)
            {
                self.procs.insert(
                    *pid,
                    Entry {
                        exe: exe.clone(),
                        state: State::Active { last_fg: now },
                        hwnds: Vec::new(),
                    },
                );
            }
        }

        // 3. Refresh window handles (keep the last known set while closing).
        for (pid, entry) in &mut self.procs {
            if let Some(hwnds) = snap.hwnds_by_pid.get(pid) {
                entry.hwnds = hwnds.clone();
            }
        }

        // 4. Foreground app with an active user is "in use": reset its timer,
        //    cancel a pending warning.
        if let Some(fg) = snap.foreground_pid.filter(|_| snap.input_recent) {
            if let Some(entry) = self.procs.get_mut(&fg) {
                match entry.state {
                    State::Active { .. } => entry.state = State::Active { last_fg: now },
                    State::Warning { .. } => {
                        entry.state = State::Active { last_fg: now };
                        events.push(MonitorEvent::WarningCancelled { pid: fg });
                    }
                    State::Closing { .. } => {}
                }
            }
        }

        // 5. While paused, freeze timers and cancel warnings (Closing continues).
        if cfg.paused {
            for (pid, entry) in &mut self.procs {
                match entry.state {
                    State::Active { .. } => entry.state = State::Active { last_fg: now },
                    State::Warning { .. } => {
                        entry.state = State::Active { last_fg: now };
                        events.push(MonitorEvent::WarningCancelled { pid: *pid });
                    }
                    State::Closing { .. } => {}
                }
            }
            return;
        }

        // 6. State transitions.
        for (pid, entry) in &mut self.procs {
            let Some(rule) = cfg.rule_for(&entry.exe) else { continue };
            let timeout = Duration::from_secs(u64::from(rule.timeout_minutes) * 60);
            let warn_lead = Duration::from_secs(u64::from(cfg.warning_seconds)).min(timeout);

            match entry.state {
                State::Active { last_fg } => {
                    if now.saturating_duration_since(last_fg) >= timeout - warn_lead {
                        let deadline = last_fg + timeout;
                        entry.state = State::Warning { deadline };
                        events.push(MonitorEvent::WarningStarted {
                            pid: *pid,
                            exe: entry.exe.clone(),
                            deadline,
                        });
                    }
                }
                State::Warning { deadline } => {
                    if now >= deadline {
                        actions.push(Action::PostClose(entry.hwnds.clone()));
                        entry.state = State::Closing { since: now, killed: false };
                    }
                }
                State::Closing { since, killed } => {
                    let elapsed = now.saturating_duration_since(since);
                    if rule.force_kill {
                        if !killed && elapsed >= Duration::from_secs(u64::from(rule.force_kill_after_secs)) {
                            actions.push(Action::Terminate(*pid));
                            entry.state = State::Closing { since, killed: true };
                        }
                    } else if elapsed >= CLOSE_GRACE {
                        // App ignored WM_CLOSE (unsaved-changes dialog?). Give up,
                        // reset the timer so we don't spam close requests.
                        entry.state = State::Active { last_fg: now };
                        events.push(MonitorEvent::CloseFailed {
                            pid: *pid,
                            exe: entry.exe.clone(),
                        });
                    }
                }
            }
        }
    }
}

fn build_snapshot(engine: &Engine, own_pid: u32) -> WorldSnapshot {
    let windows = win::enum_top_windows();
    let mut exe_by_pid = HashMap::new();
    let mut hwnds_by_pid: HashMap<u32, Vec<isize>> = HashMap::new();
    for w in &windows {
        if w.pid == own_pid {
            continue;
        }
        hwnds_by_pid.entry(w.pid).or_default().push(w.hwnd);
        if !exe_by_pid.contains_key(&w.pid) {
            if let Some(exe) = win::exe_name_for_pid(w.pid) {
                exe_by_pid.insert(w.pid, exe);
            }
        }
    }
    let closing_alive = engine
        .closing_pids()
        .into_iter()
        .map(|pid| (pid, win::is_alive(pid)))
        .collect();
    WorldSnapshot {
        exe_by_pid,
        hwnds_by_pid,
        foreground_pid: win::foreground_pid(),
        // "Recent" = within the last few seconds, comfortably above the poll
        // interval so normal interaction never reads as idle.
        input_recent: win::ms_since_input().is_some_and(|ms| ms < 5_000),
        closing_alive,
    }
}

fn execute(actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::PostClose(hwnds) => win::post_close(&hwnds),
            Action::Terminate(pid) => {
                win::terminate(pid);
            }
        }
    }
}

pub fn run(
    cfg: Arc<Mutex<Config>>,
    rx: Receiver<MonitorCmd>,
    tx: Sender<MonitorEvent>,
    ctx: eframe::egui::Context,
    own_pid: u32,
) {
    let mut engine = Engine::new();
    let mut next_tick = Instant::now();
    loop {
        let wait = next_tick.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(MonitorCmd::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(MonitorCmd::KeepOpen(pid)) => {
                let mut events = Vec::new();
                engine.keep_open(pid, Instant::now(), &mut events);
                forward(&tx, &ctx, events);
            }
            Ok(MonitorCmd::CloseNow(pid)) => {
                let mut actions = Vec::new();
                engine.close_now(pid, Instant::now(), &mut actions);
                execute(actions);
            }
            Err(RecvTimeoutError::Timeout) => {
                // Bind requests handed over by a `--launch` wrapper instance.
                let spooled = crate::launcher::drain_spool();
                if !spooled.is_empty() {
                    let mut config = cfg.lock().unwrap();
                    let mut bound = Vec::new();
                    for path in spooled {
                        if let Some(exe) = config.bind_path(&path) {
                            bound.push(exe);
                        }
                    }
                    let snapshot = config.clone();
                    drop(config);
                    let _ = crate::config::save(&snapshot);
                    forward(
                        &tx,
                        &ctx,
                        bound
                            .into_iter()
                            .map(|exe| MonitorEvent::LaunchBound { exe })
                            .collect(),
                    );
                }

                let config = cfg.lock().unwrap().clone();
                let snap = build_snapshot(&engine, own_pid);
                let mut events = Vec::new();
                let mut actions = Vec::new();
                engine.tick(Instant::now(), &snap, &config, &mut events, &mut actions);
                execute(actions);
                forward(&tx, &ctx, events);
                next_tick = Instant::now()
                    + Duration::from_millis(config.poll_interval_ms.clamp(250, 10_000));
            }
        }
    }
}

fn forward(tx: &Sender<MonitorEvent>, ctx: &eframe::egui::Context, events: Vec<MonitorEvent>) {
    if events.is_empty() {
        return;
    }
    for event in events {
        let _ = tx.send(event);
    }
    // Wake the UI thread so it drains the channel even while hidden.
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rule;

    fn cfg_with(timeout_minutes: u32, warning_seconds: u32, force_kill: bool) -> Config {
        Config {
            warning_seconds,
            rules: vec![Rule {
                exe: "notepad.exe".into(),
                enabled: true,
                timeout_minutes,
                force_kill,
                force_kill_after_secs: 30,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn snap(pids: &[(u32, &str)], fg: Option<u32>) -> WorldSnapshot {
        WorldSnapshot {
            exe_by_pid: pids.iter().map(|(p, e)| (*p, e.to_string())).collect(),
            hwnds_by_pid: pids.iter().map(|(p, _)| (*p, vec![*p as isize])).collect(),
            foreground_pid: fg,
            input_recent: true,
            closing_alive: HashMap::new(),
        }
    }

    fn snap_closing(pid: u32, alive: bool) -> WorldSnapshot {
        WorldSnapshot {
            exe_by_pid: HashMap::new(),
            hwnds_by_pid: HashMap::new(),
            foreground_pid: None,
            input_recent: true,
            closing_alive: [(pid, alive)].into(),
        }
    }

    fn run_tick(
        engine: &mut Engine,
        now: Instant,
        snap: &WorldSnapshot,
        cfg: &Config,
    ) -> (Vec<MonitorEvent>, Vec<Action>) {
        let mut events = Vec::new();
        let mut actions = Vec::new();
        engine.tick(now, snap, cfg, &mut events, &mut actions);
        (events, actions)
    }

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn warns_then_closes_idle_app() {
        let cfg = cfg_with(5, 60, false);
        let mut engine = Engine::new();
        let t0 = Instant::now();
        let world = snap(&[(100, "notepad.exe")], None);

        let (events, _) = run_tick(&mut engine, t0, &world, &cfg);
        assert!(events.is_empty());

        // Just before warning threshold (timeout 5 min - 60 s warning = 4 min).
        let (events, _) = run_tick(&mut engine, t0 + 3 * MIN + Duration::from_secs(59), &world, &cfg);
        assert!(events.is_empty());

        // At threshold: warning fires with deadline at t0 + 5 min.
        let (events, _) = run_tick(&mut engine, t0 + 4 * MIN, &world, &cfg);
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::WarningStarted { pid: 100, .. }]
        ));

        // At deadline: WM_CLOSE goes out.
        let (_, actions) = run_tick(&mut engine, t0 + 5 * MIN, &world, &cfg);
        assert!(matches!(actions.as_slice(), [Action::PostClose(_)]));

        // Process exits -> Closed event.
        let (events, _) = run_tick(
            &mut engine,
            t0 + 5 * MIN + Duration::from_secs(2),
            &snap_closing(100, false),
            &cfg,
        );
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::Closed { pid: 100, forced: false, .. }]
        ));
        assert!(engine.procs.is_empty());
    }

    #[test]
    fn foreground_resets_timer_and_cancels_warning() {
        let cfg = cfg_with(5, 60, false);
        let mut engine = Engine::new();
        let t0 = Instant::now();
        let world = snap(&[(100, "notepad.exe")], None);

        run_tick(&mut engine, t0, &world, &cfg);
        let (events, _) = run_tick(&mut engine, t0 + 4 * MIN, &world, &cfg);
        assert_eq!(events.len(), 1); // warning started

        // App comes to foreground during the warning.
        let fg_world = snap(&[(100, "notepad.exe")], Some(100));
        let (events, _) = run_tick(&mut engine, t0 + 4 * MIN + Duration::from_secs(10), &fg_world, &cfg);
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::WarningCancelled { pid: 100 }]
        ));

        // Timer restarted: no warning until 4 more minutes pass.
        let t1 = t0 + 4 * MIN + Duration::from_secs(10);
        let (events, _) = run_tick(&mut engine, t1 + 3 * MIN, &world, &cfg);
        assert!(events.is_empty());
        let (events, _) = run_tick(&mut engine, t1 + 4 * MIN, &world, &cfg);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn keep_open_resets_warning() {
        let cfg = cfg_with(5, 60, false);
        let mut engine = Engine::new();
        let t0 = Instant::now();
        let world = snap(&[(100, "notepad.exe")], None);

        run_tick(&mut engine, t0, &world, &cfg);
        run_tick(&mut engine, t0 + 4 * MIN, &world, &cfg);

        let mut events = Vec::new();
        engine.keep_open(100, t0 + 4 * MIN + Duration::from_secs(30), &mut events);
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::WarningCancelled { pid: 100 }]
        ));

        // Old deadline passes without a close.
        let (events, actions) = run_tick(&mut engine, t0 + 5 * MIN, &world, &cfg);
        assert!(events.is_empty());
        assert!(actions.is_empty());
    }

    #[test]
    fn force_kill_after_grace() {
        let cfg = cfg_with(5, 60, true);
        let mut engine = Engine::new();
        let t0 = Instant::now();
        let world = snap(&[(100, "notepad.exe")], None);

        run_tick(&mut engine, t0, &world, &cfg);
        run_tick(&mut engine, t0 + 4 * MIN, &world, &cfg);
        let (_, actions) = run_tick(&mut engine, t0 + 5 * MIN, &world, &cfg);
        assert!(matches!(actions.as_slice(), [Action::PostClose(_)]));

        // Still alive 29 s after WM_CLOSE: no kill yet.
        let (_, actions) = run_tick(
            &mut engine,
            t0 + 5 * MIN + Duration::from_secs(29),
            &snap_closing(100, true),
            &cfg,
        );
        assert!(actions.is_empty());

        // 30 s: terminate.
        let (_, actions) = run_tick(
            &mut engine,
            t0 + 5 * MIN + Duration::from_secs(30),
            &snap_closing(100, true),
            &cfg,
        );
        assert!(matches!(actions.as_slice(), [Action::Terminate(100)]));

        // Process gone next tick -> Closed{forced}.
        let (events, _) = run_tick(
            &mut engine,
            t0 + 5 * MIN + Duration::from_secs(31),
            &snap_closing(100, false),
            &cfg,
        );
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::Closed { pid: 100, forced: true, .. }]
        ));
    }

    #[test]
    fn close_ignored_reverts_with_fresh_timer() {
        let cfg = cfg_with(5, 60, false);
        let mut engine = Engine::new();
        let t0 = Instant::now();
        let world = snap(&[(100, "notepad.exe")], None);

        run_tick(&mut engine, t0, &world, &cfg);
        run_tick(&mut engine, t0 + 4 * MIN, &world, &cfg);
        run_tick(&mut engine, t0 + 5 * MIN, &world, &cfg);

        // App shows a "save changes?" dialog and stays alive past the grace period.
        let t_grace = t0 + 5 * MIN + CLOSE_GRACE;
        let mut alive = snap(&[(100, "notepad.exe")], None);
        alive.closing_alive = [(100, true)].into();
        let (events, actions) = run_tick(&mut engine, t_grace, &alive, &cfg);
        assert!(actions.is_empty());
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::CloseFailed { pid: 100, .. }]
        ));

        // Fresh timer: warning again only after another 4 minutes.
        let (events, _) = run_tick(&mut engine, t_grace + 3 * MIN, &world, &cfg);
        assert!(events.is_empty());
        let (events, _) = run_tick(&mut engine, t_grace + 4 * MIN, &world, &cfg);
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::WarningStarted { pid: 100, .. }]
        ));
    }

    #[test]
    fn paused_freezes_timers_and_cancels_warnings() {
        let mut cfg = cfg_with(5, 60, false);
        let mut engine = Engine::new();
        let t0 = Instant::now();
        let world = snap(&[(100, "notepad.exe")], None);

        run_tick(&mut engine, t0, &world, &cfg);
        run_tick(&mut engine, t0 + 4 * MIN, &world, &cfg); // warning active

        cfg.paused = true;
        let (events, actions) = run_tick(&mut engine, t0 + 4 * MIN + Duration::from_secs(1), &world, &cfg);
        assert!(actions.is_empty());
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::WarningCancelled { pid: 100 }]
        ));

        // Paused for an hour, then resumed: nothing closes immediately.
        let t_resume = t0 + 64 * MIN;
        let (events, actions) = run_tick(&mut engine, t_resume, &world, &cfg);
        assert!(events.len() == 0 && actions.is_empty());
        cfg.paused = false;
        let (events, actions) = run_tick(&mut engine, t_resume + Duration::from_secs(1), &world, &cfg);
        assert!(events.is_empty());
        assert!(actions.is_empty());
    }

    #[test]
    fn foreground_without_input_does_not_reset() {
        let cfg = cfg_with(5, 60, false);
        let mut engine = Engine::new();
        let t0 = Instant::now();
        // App stays foreground the whole time, but the user gives no input
        // (walked away): it must still idle out and get closed.
        let mut world = snap(&[(100, "notepad.exe")], Some(100));
        world.input_recent = false;

        run_tick(&mut engine, t0, &world, &cfg);
        let (events, _) = run_tick(&mut engine, t0 + 4 * MIN, &world, &cfg);
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::WarningStarted { pid: 100, .. }]
        ));
        let (_, actions) = run_tick(&mut engine, t0 + 5 * MIN, &world, &cfg);
        assert!(matches!(actions.as_slice(), [Action::PostClose(_)]));

        // The moment the user is back (input + foreground), the timer resets.
        let mut engine = Engine::new();
        let world_active = snap(&[(100, "notepad.exe")], Some(100));
        run_tick(&mut engine, t0, &world_active, &cfg);
        let (events, _) = run_tick(&mut engine, t0 + 4 * MIN, &world_active, &cfg);
        assert!(events.is_empty());
    }

    #[test]
    fn vanished_and_disabled_pids_are_dropped() {
        let cfg = cfg_with(5, 60, false);
        let mut engine = Engine::new();
        let t0 = Instant::now();

        run_tick(&mut engine, t0, &snap(&[(100, "notepad.exe")], None), &cfg);
        assert_eq!(engine.procs.len(), 1);

        // User closed it themselves: silently dropped.
        let (events, _) = run_tick(&mut engine, t0 + MIN, &snap(&[], None), &cfg);
        assert!(events.is_empty());
        assert!(engine.procs.is_empty());

        // Rule disabled mid-flight.
        run_tick(&mut engine, t0, &snap(&[(200, "notepad.exe")], None), &cfg);
        let mut disabled = cfg.clone();
        disabled.rules[0].enabled = false;
        run_tick(&mut engine, t0 + MIN, &snap(&[(200, "notepad.exe")], None), &disabled);
        assert!(engine.procs.is_empty());

        // Pid reuse: same pid, different exe -> old entry replaced.
        run_tick(&mut engine, t0, &snap(&[(300, "notepad.exe")], None), &cfg);
        run_tick(&mut engine, t0 + MIN, &snap(&[(300, "other.exe")], None), &cfg);
        assert!(engine.procs.is_empty());
    }

    #[test]
    fn multiple_processes_same_exe_are_independent() {
        let cfg = cfg_with(5, 60, false);
        let mut engine = Engine::new();
        let t0 = Instant::now();

        run_tick(&mut engine, t0, &snap(&[(100, "notepad.exe")], None), &cfg);
        // Second instance appears 2 minutes later.
        let both = snap(&[(100, "notepad.exe"), (200, "notepad.exe")], None);
        run_tick(&mut engine, t0 + 2 * MIN, &both, &cfg);
        assert_eq!(engine.procs.len(), 2);

        // First warns at t0+4min, second not until t0+6min.
        let (events, _) = run_tick(&mut engine, t0 + 4 * MIN, &both, &cfg);
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::WarningStarted { pid: 100, .. }]
        ));
        let (events, _) = run_tick(&mut engine, t0 + 6 * MIN, &both, &cfg);
        assert!(events.iter().any(
            |e| matches!(e, MonitorEvent::WarningStarted { pid: 200, .. })
        ));
    }

    #[test]
    fn warning_immediately_when_warning_longer_than_timeout() {
        let cfg = cfg_with(1, 120, false); // 1 min timeout, 2 min warning lead
        let mut engine = Engine::new();
        let t0 = Instant::now();
        let world = snap(&[(100, "notepad.exe")], None);

        // Warn lead clamps to the timeout: warning starts right away.
        let (events, _) = run_tick(&mut engine, t0, &world, &cfg);
        assert!(matches!(
            events.as_slice(),
            [MonitorEvent::WarningStarted { pid: 100, .. }]
        ));
        // Close at the full timeout.
        let (_, actions) = run_tick(&mut engine, t0 + MIN, &world, &cfg);
        assert!(matches!(actions.as_slice(), [Action::PostClose(_)]));
    }
}
