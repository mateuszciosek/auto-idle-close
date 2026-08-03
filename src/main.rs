#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod i18n;
mod launcher;
mod monitor;
mod tray;
mod ui;
mod win;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use eframe::egui;

fn main() -> eframe::Result {
    // Wrapper mode: `--launch <exe>` (or a dropped .exe path) starts the
    // target and makes sure a rule binds it.
    let launch_target = launcher::launch_target_from_args();
    if let Some(target) = &launch_target {
        launcher::start_target(target);
    }

    if !win::acquire_single_instance() {
        // Another instance owns the tray; hand the bind request over.
        if let Some(target) = &launch_target {
            launcher::append_to_spool(target);
        }
        return Ok(());
    }

    let mut initial_cfg = config::load_or_default();
    if let Some(target) = &launch_target {
        if initial_cfg.bind_path(target).is_some() {
            let _ = config::save(&initial_cfg);
        }
    }
    let cfg = Arc::new(Mutex::new(initial_cfg));
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (evt_tx, evt_rx) = mpsc::channel();
    let (tray_tx, tray_rx) = mpsc::channel();

    let (icon_rgba, icon_size) = tray::icon_rgba(64);
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Auto Idle Close")
            .with_inner_size([560.0, 620.0])
            .with_min_inner_size([460.0, 400.0])
            .with_icon(egui::IconData {
                rgba: icon_rgba,
                width: icon_size,
                height: icon_size,
            }),
        ..Default::default()
    };

    let cfg_for_app = cfg.clone();
    eframe::run_native(
        "auto-idle-close",
        native_options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();

            let monitor_cfg = cfg_for_app.clone();
            let monitor_ctx = ctx.clone();
            let own_pid = win::my_pid();
            std::thread::spawn(move || {
                monitor::run(monitor_cfg, cmd_rx, evt_tx, monitor_ctx, own_pid);
            });

            let (paused, lang) = {
                let cfg = cfg_for_app.lock().unwrap();
                (cfg.paused, cfg.language)
            };
            let tray = tray::init(ctx, tray_tx, paused, i18n::strings(lang))
                .expect("failed to create tray icon");

            Ok(Box::new(ui::App::new(
                cfg_for_app,
                cmd_tx,
                evt_rx,
                tray_rx,
                tray,
            )))
        }),
    )
}
