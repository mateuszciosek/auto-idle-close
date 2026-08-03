use std::sync::mpsc::Sender;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::Strings;

pub enum TrayMsg {
    OpenSettings,
    TogglePause,
    Exit,
}

pub struct Tray {
    // Must stay alive for the whole app lifetime or the icon disappears.
    _icon: TrayIcon,
    open_item: MenuItem,
    pause_item: CheckMenuItem,
    exit_item: MenuItem,
}

impl Tray {
    pub fn set_paused(&self, paused: bool, s: &Strings) {
        self.pause_item.set_checked(paused);
        self.set_tooltip(paused, s);
    }

    /// Re-label the menu after a language change.
    pub fn apply_language(&self, s: &Strings, paused: bool) {
        self.open_item.set_text(s.tray_open);
        self.pause_item.set_text(s.tray_pause);
        self.exit_item.set_text(s.tray_exit);
        self.set_tooltip(paused, s);
    }

    fn set_tooltip(&self, paused: bool, s: &Strings) {
        let _ = self._icon.set_tooltip(Some(if paused {
            s.tray_tooltip_paused
        } else {
            s.tray_tooltip
        }));
    }
}

/// Create the tray icon and menu. Must be called on the winit/UI thread so the
/// event loop pumps messages for tray-icon's hidden window. Events are pushed
/// from the registered handlers (polling alone would starve while the settings
/// window is hidden), and each one wakes egui via `request_repaint`.
pub fn init(
    ctx: eframe::egui::Context,
    tx: Sender<TrayMsg>,
    paused: bool,
    s: &Strings,
) -> tray_icon::Result<Tray> {
    let open_item = MenuItem::new(s.tray_open, true, None);
    let pause_item = CheckMenuItem::new(s.tray_pause, true, paused, None);
    let exit_item = MenuItem::new(s.tray_exit, true, None);
    let menu = Menu::new();
    menu.append_items(&[
        &open_item,
        &pause_item,
        &PredefinedMenuItem::separator(),
        &exit_item,
    ])
    .expect("append tray menu items");

    let (rgba, size) = icon_rgba(32);
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(if paused { s.tray_tooltip_paused } else { s.tray_tooltip })
        .with_icon(Icon::from_rgba(rgba, size, size).expect("valid icon rgba"))
        .build()?;

    let open_id: MenuId = open_item.id().clone();
    let pause_id: MenuId = pause_item.id().clone();
    let exit_id: MenuId = exit_item.id().clone();
    {
        let tx = tx.clone();
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let msg = if *event.id() == open_id {
                TrayMsg::OpenSettings
            } else if *event.id() == pause_id {
                TrayMsg::TogglePause
            } else if *event.id() == exit_id {
                TrayMsg::Exit
            } else {
                return;
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        }));
    }
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = tx.send(TrayMsg::OpenSettings);
            ctx.request_repaint();
        }
    }));

    Ok(Tray {
        _icon: icon,
        open_item,
        pause_item,
        exit_item,
    })
}

/// Procedurally drawn clock icon (blue face, white 10:10 hands) so no image
/// decoder needs to be linked. Returns (rgba, side length).
pub fn icon_rgba(size: u32) -> (Vec<u8>, u32) {
    let s = size as f32;
    let center = (s - 1.0) / 2.0;
    let radius = s * 0.47;
    let border = (s * 0.07).max(1.0);
    let hand_w = (s * 0.055).max(1.0);

    let face = [30u8, 111, 216];
    let rim = [16u8, 62, 133];
    let white = [255u8, 255, 255];

    // Hands as line segments from the center (minute toward 2, hour toward 10).
    let hands = [
        (center + s * 0.20, center - s * 0.20),
        (center - s * 0.17, center - s * 0.12),
    ];

    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32, y as f32);
            let dist = ((fx - center).powi(2) + (fy - center).powi(2)).sqrt();
            // Antialiased alpha at the outer edge.
            let alpha = (radius - dist + 0.5).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let mut color = if dist > radius - border { rim } else { face };
            for &(hx, hy) in &hands {
                if dist_to_segment(fx, fy, center, center, hx, hy) <= hand_w {
                    color = white;
                    break;
                }
            }
            let i = ((y * size + x) * 4) as usize;
            rgba[i..i + 3].copy_from_slice(&color);
            rgba[i + 3] = (alpha * 255.0) as u8;
        }
    }
    (rgba, size)
}

fn dist_to_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (abx, aby) = (bx - ax, by - ay);
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq == 0.0 {
        0.0
    } else {
        (((px - ax) * abx + (py - ay) * aby) / len_sq).clamp(0.0, 1.0)
    };
    let (cx, cy) = (ax + t * abx, ay + t * aby);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}
