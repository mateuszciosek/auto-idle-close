use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout, RichText, ViewportCommand};

use crate::config::{self, Config, Rule};
use crate::i18n::{self, Language, Strings, fmt};
use crate::launcher;
use crate::monitor::{MonitorCmd, MonitorEvent};
use crate::tray::{Tray, TrayMsg};
use crate::win;

const RUNNING_REFRESH: Duration = Duration::from_secs(2);
/// Config writes are debounced: saved this long after the last edit.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(700);

// NOTE on icon glyphs: egui's embedded fonts (Ubuntu-Light, NotoEmoji,
// emoji-icon-font) cover only a subset of Unicode. …/🗑/▶/⚙/🔍 are verified
// present; ⋯ (U+22EF) and ✕ (U+2715) are NOT — they render as boxes.

#[derive(Clone)]
struct ActiveWarning {
    pid: u32,
    exe: String,
    deadline: Instant,
}

struct AppRow {
    exe: String,
    path: Option<String>,
    title: String,
    windows: usize,
    admin: bool,
}

/// State shared with the deferred viewports (settings window, warning popup).
/// Their callbacks must be Send + Sync and outlive the frame, so everything
/// they touch lives behind this.
struct Shared {
    cfg: Arc<Mutex<Config>>,
    log: Mutex<VecDeque<(Instant, String)>>,
    /// Time of the last unsaved edit (drives the debounced save).
    dirty_since: Mutex<Option<Instant>>,
    ctx_menu_installed: AtomicBool,
    settings_open: AtomicBool,
}

impl Shared {
    fn push_log(&self, message: String) {
        let mut log = self.log.lock().unwrap();
        log.push_front((Instant::now(), message));
        log.truncate(20);
    }

    fn mark_dirty(&self) {
        *self.dirty_since.lock().unwrap() = Some(Instant::now());
    }
}

pub struct App {
    shared: Arc<Shared>,
    cmd_tx: Sender<MonitorCmd>,
    evt_rx: Receiver<MonitorEvent>,
    tray_rx: Receiver<TrayMsg>,
    tray: Tray,
    warnings: Arc<Mutex<Vec<ActiveWarning>>>,
    /// Sender clone for the warning viewport callback (Sender is !Sync).
    shared_cmd_tx: Arc<Mutex<Sender<MonitorCmd>>>,
    running: Vec<AppRow>,
    /// pid -> (lowercase exe name, full path); None = unqueryable (elevated).
    exe_cache: HashMap<u32, Option<(String, String)>>,
    last_refresh: Option<Instant>,
    app_filter: String,
    add_rule_text: String,
    /// Mirrors cfg.paused so the tray checkmark follows edits from anywhere.
    last_paused: bool,
    /// Mirrors the resolved language so tray labels follow changes.
    last_strings: &'static Strings,
    save_error: Option<String>,
    allow_exit: bool,
}

impl App {
    pub fn new(
        cfg: Arc<Mutex<Config>>,
        cmd_tx: Sender<MonitorCmd>,
        evt_rx: Receiver<MonitorEvent>,
        tray_rx: Receiver<TrayMsg>,
        tray: Tray,
    ) -> Self {
        let (last_paused, last_strings) = {
            let cfg = cfg.lock().unwrap();
            (cfg.paused, i18n::strings(cfg.language))
        };
        Self {
            shared: Arc::new(Shared {
                cfg,
                log: Mutex::new(VecDeque::new()),
                dirty_since: Mutex::new(None),
                ctx_menu_installed: AtomicBool::new(launcher::context_menu_installed()),
                settings_open: AtomicBool::new(false),
            }),
            shared_cmd_tx: Arc::new(Mutex::new(cmd_tx.clone())),
            cmd_tx,
            evt_rx,
            tray_rx,
            tray,
            warnings: Arc::new(Mutex::new(Vec::new())),
            running: Vec::new(),
            exe_cache: HashMap::new(),
            last_refresh: None,
            app_filter: String::new(),
            add_rule_text: String::new(),
            last_paused,
            last_strings,
            save_error: None,
            allow_exit: false,
        }
    }

    fn strings(&self) -> &'static Strings {
        i18n::strings(self.shared.cfg.lock().unwrap().language)
    }

    fn drain_channels(&mut self, ctx: &egui::Context) {
        let s = self.strings();
        while let Ok(msg) = self.tray_rx.try_recv() {
            match msg {
                TrayMsg::OpenSettings => {
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                TrayMsg::TogglePause => {
                    let paused = {
                        let mut cfg = self.shared.cfg.lock().unwrap();
                        cfg.paused = !cfg.paused;
                        cfg.paused
                    };
                    self.shared.mark_dirty();
                    self.shared.push_log(
                        if paused { s.log_paused } else { s.log_resumed }.to_string(),
                    );
                }
                TrayMsg::Exit => {
                    self.allow_exit = true;
                    let _ = self.cmd_tx.send(MonitorCmd::Shutdown);
                    self.save_now();
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        while let Ok(event) = self.evt_rx.try_recv() {
            let mut warnings = self.warnings.lock().unwrap();
            match event {
                MonitorEvent::WarningStarted { pid, exe, deadline } => {
                    warnings.retain(|w| w.pid != pid);
                    warnings.push(ActiveWarning { pid, exe, deadline });
                }
                MonitorEvent::WarningCancelled { pid } => {
                    warnings.retain(|w| w.pid != pid);
                }
                MonitorEvent::Closed { pid, exe, forced } => {
                    warnings.retain(|w| w.pid != pid);
                    drop(warnings);
                    self.shared.push_log(fmt(
                        if forced { s.log_force_closed } else { s.log_closed },
                        &exe,
                    ));
                }
                MonitorEvent::CloseFailed { pid, exe } => {
                    warnings.retain(|w| w.pid != pid);
                    drop(warnings);
                    self.shared.push_log(fmt(s.log_close_failed, &exe));
                }
                MonitorEvent::LaunchBound { exe } => {
                    drop(warnings);
                    self.shared.push_log(fmt(s.log_bound, &exe));
                }
            }
        }
    }

    fn refresh_running(&mut self) {
        let now = Instant::now();
        if self
            .last_refresh
            .is_some_and(|t| now.duration_since(t) < RUNNING_REFRESH)
        {
            return;
        }
        self.last_refresh = Some(now);

        let own = win::my_pid();
        let windows = win::enum_top_windows();
        let mut seen_pids: HashMap<u32, ()> = HashMap::new();
        let mut by_exe: HashMap<String, AppRow> = HashMap::new();
        let mut admin_rows: Vec<AppRow> = Vec::new();
        for w in &windows {
            if w.pid == own {
                continue;
            }
            let info = self
                .exe_cache
                .entry(w.pid)
                .or_insert_with(|| {
                    win::exe_for_pid(w.pid).map(|p| {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        (name, p.to_string_lossy().into_owned())
                    })
                })
                .clone();
            seen_pids.insert(w.pid, ());
            match info {
                Some((exe, path)) => {
                    let row = by_exe.entry(exe.clone()).or_insert_with(|| AppRow {
                        exe,
                        path: Some(path),
                        title: w.title.clone(),
                        windows: 0,
                        admin: false,
                    });
                    row.windows += 1;
                }
                None => admin_rows.push(AppRow {
                    exe: String::new(),
                    path: None,
                    title: w.title.clone(),
                    windows: 1,
                    admin: true,
                }),
            }
        }
        self.exe_cache.retain(|pid, _| seen_pids.contains_key(pid));

        let mut rows: Vec<AppRow> = by_exe.into_values().collect();
        // Window titles are what the user recognizes — sort by them.
        rows.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        admin_rows.sort_by(|a, b| a.title.cmp(&b.title));
        rows.extend(admin_rows);
        self.running = rows;
    }

    fn save_now(&mut self) {
        *self.shared.dirty_since.lock().unwrap() = None;
        let cfg = self.shared.cfg.lock().unwrap().clone();
        let s = i18n::strings(cfg.language);
        self.save_error = config::save(&cfg)
            .err()
            .map(|e| fmt(s.save_error_fmt, e));
    }

    fn save_if_settled(&mut self) {
        let due = self
            .shared
            .dirty_since
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() >= SAVE_DEBOUNCE);
        if due {
            self.save_now();
        }
    }

    fn section_running(&mut self, ui: &mut egui::Ui, s: &'static Strings) {
        ui.horizontal(|ui| {
            ui.heading(s.sec_running);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.app_filter)
                        .hint_text(s.filter_hint)
                        .desired_width(140.0),
                );
            });
        });
        ui.label(RichText::new(s.running_hint).weak().small());
        ui.add_space(4.0);

        let filter = self.app_filter.trim().to_lowercase();
        egui::ScrollArea::vertical()
            .id_salt("running")
            .max_height(240.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let mut changes: Vec<(String, Option<String>, bool)> = Vec::new();
                let mut shown = 0usize;
                {
                    let cfg = self.shared.cfg.lock().unwrap();
                    for row in &self.running {
                        if !filter.is_empty()
                            && !row.title.to_lowercase().contains(&filter)
                            && !row.exe.contains(&filter)
                        {
                            continue;
                        }
                        shown += 1;
                        ui.horizontal(|ui| {
                            if row.admin {
                                ui.add_enabled(false, egui::Checkbox::new(&mut false, ""));
                                ui.label(RichText::new(truncate(&row.title, 44)).weak());
                                ui.label(RichText::new(s.admin_row).weak().small());
                                return;
                            }
                            let mut checked =
                                cfg.rule_for(&row.exe).is_some_and(|r| r.enabled);
                            if ui.checkbox(&mut checked, "").changed() {
                                changes.push((row.exe.clone(), row.path.clone(), checked));
                            }
                            let mut title = truncate(&row.title, 44);
                            if row.windows > 1 {
                                title.push_str(&format!(" (+{})", row.windows - 1));
                            }
                            ui.label(RichText::new(title).strong())
                                .on_hover_text(&row.title);
                            ui.label(RichText::new(&row.exe).weak().small());
                        });
                    }
                    if self.running.is_empty() {
                        ui.label(RichText::new(s.scanning).weak());
                    } else if shown == 0 {
                        ui.label(RichText::new(s.no_filter_match).weak());
                    }
                }
                for (exe, path, enabled) in changes {
                    let mut cfg = self.shared.cfg.lock().unwrap();
                    match cfg.rule_for_mut(&exe) {
                        Some(rule) => {
                            rule.enabled = enabled;
                            if rule.path.is_none() {
                                rule.path = path;
                            }
                        }
                        None => {
                            if enabled {
                                cfg.rules.push(Rule { exe, path, ..Default::default() });
                            }
                        }
                    }
                    drop(cfg);
                    self.shared.mark_dirty();
                }
            });
    }

    fn section_rules(&mut self, ui: &mut egui::Ui, s: &'static Strings) {
        ui.heading(s.sec_rules);
        let mut start_actions: Vec<(String, String)> = Vec::new();
        let mut shortcut_actions: Vec<(String, String)> = Vec::new();
        let mut dirty = false;
        let mut cfg = self.shared.cfg.lock().unwrap();
        if cfg.rules.is_empty() {
            ui.label(RichText::new(s.no_rules).weak());
        }
        let mut remove: Option<usize> = None;
        for (i, rule) in cfg.rules.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                if ui.checkbox(&mut rule.enabled, "").changed() {
                    dirty = true;
                }
                ui.label(RichText::new(&rule.exe).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("🗑").on_hover_text(s.remove_rule).clicked() {
                        remove = Some(i);
                    }
                    if let Some(path) = rule.path.clone() {
                        ui.menu_button("…", |ui| {
                            if ui.button(s.start_now).clicked() {
                                start_actions.push((rule.exe.clone(), path.clone()));
                                ui.close();
                            }
                            if ui
                                .button(s.create_shortcut)
                                .on_hover_text(s.shortcut_hint)
                                .clicked()
                            {
                                shortcut_actions.push((rule.exe.clone(), path.clone()));
                                ui.close();
                            }
                            ui.label(RichText::new(&path).weak().small());
                        });
                    }
                    let kill_secs = ui.add_enabled(
                        rule.force_kill,
                        egui::DragValue::new(&mut rule.force_kill_after_secs)
                            .range(5..=600)
                            .suffix(" s"),
                    );
                    if kill_secs.changed() {
                        dirty = true;
                    }
                    if ui
                        .checkbox(&mut rule.force_kill, s.force_after)
                        .on_hover_text(s.force_hint)
                        .changed()
                    {
                        dirty = true;
                    }
                    if ui
                        .add(
                            egui::DragValue::new(&mut rule.timeout_minutes)
                                .range(1..=1440)
                                .suffix(" min"),
                        )
                        .changed()
                    {
                        dirty = true;
                    }
                    ui.label(s.close_after_idle);
                });
            });
        }
        if let Some(i) = remove {
            cfg.rules.remove(i);
            dirty = true;
        }
        drop(cfg);
        if dirty {
            self.shared.mark_dirty();
        }

        for (exe, path) in start_actions {
            if launcher::start_target(std::path::Path::new(&path)) {
                self.shared.push_log(fmt(s.log_started, &exe));
            } else {
                self.shared.push_log(fmt(s.log_start_failed, &exe));
            }
        }
        for (exe, path) in shortcut_actions {
            if launcher::create_companion_shortcut(std::path::Path::new(&path)) {
                self.shared.push_log(fmt(s.log_shortcut_ok, &exe));
            } else {
                self.shared.push_log(fmt(s.log_shortcut_fail, &exe));
            }
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(s.add_by_name);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.add_rule_text)
                    .hint_text("e.g. chrome.exe")
                    .desired_width(160.0),
            );
            let submitted =
                response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button(s.add_btn).clicked() || submitted)
                && !self.add_rule_text.trim().is_empty()
            {
                let mut exe = self.add_rule_text.trim().to_lowercase();
                if !exe.contains('.') {
                    exe.push_str(".exe");
                }
                let mut cfg = self.shared.cfg.lock().unwrap();
                if cfg.rule_for(&exe).is_none() {
                    cfg.rules.push(Rule { exe, ..Default::default() });
                    drop(cfg);
                    self.shared.mark_dirty();
                }
                self.add_rule_text.clear();
            }
            if ui.button(s.browse_btn).on_hover_text(s.browse_hint).clicked() {
                if let Some(path) = win::pick_exe_file() {
                    let mut cfg = self.shared.cfg.lock().unwrap();
                    if let Some(exe) = cfg.bind_path(&path) {
                        drop(cfg);
                        self.shared.mark_dirty();
                        self.shared.push_log(fmt(s.log_rule_added, &exe));
                    }
                }
            }
        });
    }

    /// The separate settings window, rendered as a deferred viewport so it
    /// keeps working regardless of the main window's visibility.
    fn settings_window(&self, ctx: &egui::Context) {
        if !self.shared.settings_open.load(Ordering::Relaxed) {
            return;
        }
        let shared = self.shared.clone();
        let s = self.last_strings;
        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of("app-settings"),
            egui::ViewportBuilder::default()
                .with_title(s.settings_title)
                .with_inner_size([440.0, 420.0])
                .with_min_inner_size([380.0, 300.0]),
            move |ui, _class| {
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    shared.settings_open.store(false, Ordering::Relaxed);
                }
                let s = i18n::strings(shared.cfg.lock().unwrap().language);
                egui::CentralPanel::default().show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.heading(s.sec_general);
                        {
                            let mut cfg = shared.cfg.lock().unwrap();
                            ui.horizontal(|ui| {
                                ui.label(s.warn_before);
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut cfg.warning_seconds)
                                            .range(5..=600)
                                            .suffix(" s"),
                                    )
                                    .changed()
                                {
                                    drop(cfg);
                                    shared.mark_dirty();
                                    return;
                                }
                            });
                        }
                        {
                            let mut cfg = shared.cfg.lock().unwrap();
                            let mut paused = cfg.paused;
                            if ui.checkbox(&mut paused, s.pause_monitoring).changed() {
                                cfg.paused = paused;
                                drop(cfg);
                                shared.mark_dirty();
                            }
                        }
                        {
                            let mut cfg = shared.cfg.lock().unwrap();
                            let mut lang = cfg.language;
                            ui.horizontal(|ui| {
                                ui.label(s.language_label);
                                egui::ComboBox::from_id_salt("language")
                                    .selected_text(match lang {
                                        Language::Auto => s.lang_auto,
                                        Language::En => "English",
                                        Language::Pl => "Polski",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut lang,
                                            Language::Auto,
                                            s.lang_auto,
                                        );
                                        ui.selectable_value(&mut lang, Language::En, "English");
                                        ui.selectable_value(&mut lang, Language::Pl, "Polski");
                                    });
                            });
                            if lang != cfg.language {
                                cfg.language = lang;
                                drop(cfg);
                                shared.mark_dirty();
                            }
                        }

                        ui.add_space(8.0);
                        ui.heading(s.sec_integration);
                        let mut installed =
                            shared.ctx_menu_installed.load(Ordering::Relaxed);
                        if ui
                            .checkbox(&mut installed, s.explorer_checkbox)
                            .on_hover_text(s.explorer_hint)
                            .changed()
                        {
                            let ok = if installed {
                                launcher::install_context_menu(s.explorer_verb)
                            } else {
                                launcher::uninstall_context_menu()
                            };
                            if ok {
                                shared.ctx_menu_installed.store(installed, Ordering::Relaxed);
                                shared.push_log(
                                    if installed {
                                        s.log_menu_installed
                                    } else {
                                        s.log_menu_removed
                                    }
                                    .to_string(),
                                );
                            } else {
                                shared.push_log(s.log_menu_failed.to_string());
                            }
                        }

                        ui.add_space(8.0);
                        let log_len = shared.log.lock().unwrap().len();
                        egui::CollapsingHeader::new(format!(
                            "{} ({log_len})",
                            s.recent_activity
                        ))
                        .default_open(false)
                        .show(ui, |ui| {
                            let log = shared.log.lock().unwrap();
                            if log.is_empty() {
                                ui.label(RichText::new(s.nothing_yet).weak());
                            }
                            for (when, message) in log.iter() {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(ago(*when, s)).weak().small());
                                    ui.label(RichText::new(message).small());
                                });
                            }
                        });

                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(format!(
                                "{}{}",
                                s.config_prefix,
                                config::config_path().display()
                            ))
                            .weak()
                            .small(),
                        );
                    });
                });
            },
        );
    }

    /// Countdown popup as a deferred viewport (an immediate viewport would
    /// panic while the main window is hidden to the tray).
    fn warning_popup(&mut self, ctx: &egui::Context) {
        let count = self.warnings.lock().unwrap().len();
        if count == 0 {
            return;
        }
        // Keep the root's pass loop alive so the viewport stays registered
        // (and gets pruned promptly once the last warning is gone).
        ctx.request_repaint_after(Duration::from_millis(300));

        let width = 340.0;
        let height = 14.0 + 52.0 * count as f32;
        let pos = ctx
            .input(|i| i.viewport().monitor_size)
            .map(|m| egui::pos2(m.x - width - 16.0, m.y - height - 80.0))
            .unwrap_or(egui::pos2(600.0, 400.0));

        let warnings = self.warnings.clone();
        let cmd_tx = self.shared_cmd_tx.clone();
        let s = self.last_strings;
        ctx.show_viewport_deferred(
            egui::ViewportId::from_hash_of("idle-warnings"),
            egui::ViewportBuilder::default()
                .with_title("Closing soon")
                .with_inner_size([width, height])
                .with_position(pos)
                .with_always_on_top()
                .with_decorations(false)
                .with_resizable(false)
                .with_taskbar(false),
            move |ui, _class| {
                // Tick the countdown text.
                ui.ctx().request_repaint_after(Duration::from_millis(250));
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut acted: Vec<(u32, MonitorCmd)> = Vec::new();
                    for w in warnings.lock().unwrap().iter() {
                        let left = w.deadline.saturating_duration_since(Instant::now());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&w.exe).strong());
                                ui.label(
                                    RichText::new(fmt(s.closing_in, left.as_secs()))
                                        .weak()
                                        .small(),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button(s.close_now).clicked() {
                                    acted.push((w.pid, MonitorCmd::CloseNow(w.pid)));
                                }
                                if ui.button(s.keep_open).clicked() {
                                    acted.push((w.pid, MonitorCmd::KeepOpen(w.pid)));
                                }
                            });
                        });
                        ui.separator();
                    }
                    if !acted.is_empty() {
                        let tx = cmd_tx.lock().unwrap();
                        let mut warnings = warnings.lock().unwrap();
                        for (pid, cmd) in acted {
                            let _ = tx.send(cmd);
                            warnings.retain(|w| w.pid != pid);
                        }
                        // Make sure the root runs a pass soon to prune this
                        // viewport if it just became empty.
                        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
                    }
                });
            },
        );
    }
}

impl eframe::App for App {
    /// Runs every frame — including while the main window is hidden in the
    /// tray (as long as something calls `request_repaint`, which the monitor
    /// thread and tray handlers do). Everything that must keep working while
    /// hidden lives here: channels, exit flow, and the extra viewports.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_channels(ctx);

        // Closing the main window hides to tray instead of exiting.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_exit {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        // Keep the tray in sync with pause/language edits from any window.
        let (paused, s) = {
            let cfg = self.shared.cfg.lock().unwrap();
            (cfg.paused, i18n::strings(cfg.language))
        };
        if !std::ptr::eq(s, self.last_strings) {
            self.last_strings = s;
            self.tray.apply_language(s, paused);
        }
        if paused != self.last_paused {
            self.last_paused = paused;
            self.tray.set_paused(paused, s);
        }

        self.warning_popup(ctx);
        self.settings_window(ctx);
        self.save_if_settled();
    }

    /// Only runs while the main window is visible.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.refresh_running();
        let s = self.last_strings;

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Auto Idle Close");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(s.btn_settings).clicked() {
                        let was_open =
                            self.shared.settings_open.swap(true, Ordering::Relaxed);
                        if was_open {
                            ui.ctx().send_viewport_cmd_to(
                                egui::ViewportId::from_hash_of("app-settings"),
                                ViewportCommand::Focus,
                            );
                        }
                    }
                    if self.last_paused {
                        ui.label(RichText::new(s.lbl_paused).weak());
                    }
                });
            });
            ui.add_space(4.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                self.section_running(ui, s);
                ui.add_space(10.0);
                ui.separator();
                self.section_rules(ui, s);
                if let Some(err) = &self.save_error {
                    ui.add_space(8.0);
                    ui.colored_label(Color32::from_rgb(220, 60, 60), err);
                    ui.label(RichText::new(s.save_error_hint).weak().small());
                }
            });
        });
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

fn ago(t: Instant, s: &Strings) -> String {
    let secs = t.elapsed().as_secs();
    if secs < 60 {
        fmt(s.ago_s, secs)
    } else if secs < 3600 {
        fmt(s.ago_m, secs / 60)
    } else {
        fmt(s.ago_h, secs / 3600)
    }
}
