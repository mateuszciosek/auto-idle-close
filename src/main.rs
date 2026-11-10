#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod i18n;
mod launcher;
mod monitor;
mod policy;
mod tray;
mod ui;
mod update;
mod win;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use eframe::egui;

/// Glow (OpenGL) everywhere it works — smaller working set. RDP sessions
/// expose only OpenGL 1.1 (no GPU ICD), where context creation fails and,
/// with panic="abort", the app used to die silently; there we use wgpu,
/// whose DirectX 12 backend is properly virtualized over RDP (WARP software
/// fallback included). `--renderer glow|wgpu` overrides for troubleshooting.
fn pick_renderer() -> eframe::Renderer {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .position(|a| a == "--renderer")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
    {
        Some("wgpu") => eframe::Renderer::Wgpu,
        Some("glow") => eframe::Renderer::Glow,
        _ if win::is_remote_session() => eframe::Renderer::Wgpu,
        _ => eframe::Renderer::Glow,
    }
}

fn main() -> eframe::Result {
    // Wrapper mode: `--launch <exe>` (or a dropped .exe path) starts the
    // target and makes sure a rule binds it.
    let launch_target = launcher::launch_target_from_args();
    if let Some(target) = &launch_target {
        launcher::start_target(target);
    }

    // Tidy up the staging files a previous self-update left behind.
    update::cleanup_leftovers();

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
    // Optional admin policy (enforced rules); absent file = feature off.
    let policy = Arc::new(policy::Handle::new());
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (evt_tx, evt_rx) = mpsc::channel();
    let (tray_tx, tray_rx) = mpsc::channel();

    let (icon_rgba, icon_size) = tray::icon_rgba(64);
    let native_options = eframe::NativeOptions {
        renderer: pick_renderer(),
        viewport: egui::ViewportBuilder::default()
            .with_title("Auto Idle Close")
            .with_inner_size([560.0, 620.0])
            .with_min_inner_size([460.0, 400.0])
            // Autostart (--tray) begins hidden; the tray icon is the way in.
            .with_visible(!launcher::start_in_tray())
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

            // eframe force-shows the window after its first frame; keep a
            // --tray start off the screen through that (see win::suppress_show).
            if launcher::start_in_tray() {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(RawWindowHandle::Win32(h)) = cc.window_handle().map(|h| h.as_raw()) {
                    win::suppress_show(h.hwnd.get());
                }
            }

            let monitor_cfg = cfg_for_app.clone();
            let monitor_policy = policy.clone();
            let monitor_ctx = ctx.clone();
            let own_pid = win::my_pid();
            std::thread::spawn(move || {
                monitor::run(monitor_cfg, monitor_policy, cmd_rx, evt_tx, monitor_ctx, own_pid);
            });

            let (paused, lang) = {
                let cfg = cfg_for_app.lock().unwrap();
                (cfg.paused, cfg.language)
            };
            let tray = tray::init(ctx, tray_tx, paused, i18n::strings(lang))
                .expect("failed to create tray icon");

            Ok(Box::new(ui::App::new(
                cfg_for_app,
                policy,
                cmd_tx,
                evt_rx,
                tray_rx,
                tray,
            )))
        }),
    )
}
