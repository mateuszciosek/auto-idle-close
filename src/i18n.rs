//! UI translations. `Language::Auto` follows the Windows display language.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Auto,
    En,
    Pl,
}

/// Substitute the single `{}` placeholder in a translated template.
pub fn fmt(template: &str, arg: impl std::fmt::Display) -> String {
    template.replacen("{}", &arg.to_string(), 1)
}

pub struct Strings {
    // Main window
    pub btn_settings: &'static str,
    pub lbl_paused: &'static str,
    pub sec_running: &'static str,
    pub running_hint: &'static str,
    pub filter_hint: &'static str,
    pub scanning: &'static str,
    pub no_filter_match: &'static str,
    pub admin_row: &'static str,
    pub sec_rules: &'static str,
    pub no_rules: &'static str,
    pub remove_rule: &'static str,
    pub start_now: &'static str,
    pub create_shortcut: &'static str,
    pub shortcut_hint: &'static str,
    pub force_after: &'static str,
    pub force_hint: &'static str,
    pub close_after_idle: &'static str,
    pub add_by_name: &'static str,
    pub add_btn: &'static str,
    pub browse_btn: &'static str,
    pub browse_hint: &'static str,
    pub save_error_hint: &'static str,
    // Settings window
    pub settings_title: &'static str,
    pub sec_general: &'static str,
    pub warn_before: &'static str,
    pub pause_monitoring: &'static str,
    pub language_label: &'static str,
    pub lang_auto: &'static str,
    pub sec_integration: &'static str,
    pub autostart_checkbox: &'static str,
    pub autostart_hint: &'static str,
    pub log_autostart_on: &'static str,
    pub log_autostart_off: &'static str,
    pub log_autostart_failed: &'static str,
    pub explorer_checkbox: &'static str,
    pub explorer_hint: &'static str,
    pub sec_updates: &'static str,
    pub updates_checkbox: &'static str,
    pub updates_hint: &'static str,
    pub upd_check_now: &'static str,
    pub upd_checking: &'static str,
    pub upd_uptodate: &'static str,
    pub upd_available_fmt: &'static str,
    pub upd_install: &'static str,
    pub upd_notes: &'static str,
    pub upd_downloading: &'static str,
    pub upd_restarting: &'static str,
    pub upd_failed_fmt: &'static str,
    pub recent_activity: &'static str,
    pub nothing_yet: &'static str,
    pub config_prefix: &'static str,
    // Admin policy
    pub policy_managed_hint: &'static str,
    pub policy_banner: &'static str,
    pub policy_prefix: &'static str,
    pub policy_error_fmt: &'static str,
    pub log_rule_managed: &'static str,
    // Warning popup
    pub closing_in: &'static str,
    pub keep_open: &'static str,
    pub close_now: &'static str,
    // Log templates
    pub log_closed: &'static str,
    pub log_force_closed: &'static str,
    pub log_close_failed: &'static str,
    pub log_paused: &'static str,
    pub log_resumed: &'static str,
    pub log_bound: &'static str,
    pub log_started: &'static str,
    pub log_start_failed: &'static str,
    pub log_shortcut_ok: &'static str,
    pub log_shortcut_fail: &'static str,
    pub log_rule_added: &'static str,
    pub log_menu_installed: &'static str,
    pub log_menu_removed: &'static str,
    pub log_menu_failed: &'static str,
    pub save_error_fmt: &'static str,
    // Tray
    pub tray_open: &'static str,
    pub tray_pause: &'static str,
    pub tray_exit: &'static str,
    pub tray_tooltip: &'static str,
    pub tray_tooltip_paused: &'static str,
    /// Display name of the Explorer right-click verb.
    pub explorer_verb: &'static str,
    // Relative times
    pub ago_s: &'static str,
    pub ago_m: &'static str,
    pub ago_h: &'static str,
}

pub static EN: Strings = Strings {
    btn_settings: "⚙ Settings",
    lbl_paused: "⏸ paused",
    sec_running: "Running applications",
    running_hint: "Tick an application to close it automatically when idle.",
    filter_hint: "🔍 filter",
    scanning: "Scanning…",
    no_filter_match: "No window matches the filter.",
    admin_row: "(admin — cannot manage)",
    sec_rules: "Rules",
    no_rules: "No rules yet — tick an application above.",
    remove_rule: "Remove rule",
    start_now: "▶ Start now",
    create_shortcut: "Create desktop shortcut",
    shortcut_hint: "A \"(auto-close)\" shortcut that starts this app together with \
                    auto-idle-close, so it is always bound.",
    force_after: "force after",
    force_hint: "If the app ignores the polite close request (e.g. an \
                 unsaved-changes dialog), terminate it.",
    close_after_idle: "close after idle",
    add_by_name: "Add by name:",
    add_btn: "Add",
    browse_btn: "Browse…",
    browse_hint: "Pick an .exe from disk — you can then start it from here.",
    save_error_hint: "Move the exe to a writable folder to keep settings.",
    settings_title: "Settings — Auto Idle Close",
    sec_general: "General",
    warn_before: "Warn before closing:",
    pause_monitoring: "Pause monitoring",
    language_label: "Language:",
    lang_auto: "Auto (system)",
    sec_integration: "Integration",
    autostart_checkbox: "Start with Windows (hidden in the tray)",
    autostart_hint: "Runs auto-idle-close automatically at login (per-user registry \
                     Run entry, no admin rights). It points at this exe's current \
                     location — re-toggle after moving the file.",
    log_autostart_on: "Autostart at login enabled",
    log_autostart_off: "Autostart at login disabled",
    log_autostart_failed: "Could not change the autostart entry",
    explorer_checkbox: "Explorer right-click menu: \"Run with auto-idle-close\"",
    explorer_hint: "Adds an entry to the right-click menu of .exe files (on Windows 11 \
                    it's under \"Show more options\"). Starting an app that way launches \
                    it together with auto-idle-close and binds it automatically.\n\
                    Note: the entry points at this exe's current location — re-toggle \
                    after moving it.",
    sec_updates: "Updates",
    updates_checkbox: "Notify about new versions (daily check)",
    updates_hint: "Once a day the app asks github.com for the latest release tag. \
                   Nothing is downloaded or installed without your click.",
    upd_check_now: "Check now",
    upd_checking: "Checking…",
    upd_uptodate: "Up to date.",
    upd_available_fmt: "New version {} is available.",
    upd_install: "Download & install",
    upd_notes: "Release notes",
    upd_downloading: "Downloading update…",
    upd_restarting: "Restarting…",
    upd_failed_fmt: "Update failed: {}",
    recent_activity: "Recent activity",
    nothing_yet: "Nothing yet.",
    config_prefix: "Config: ",
    policy_managed_hint: "managed by your administrator",
    policy_banner: "Some settings are managed by your administrator.",
    policy_prefix: "Policy: ",
    policy_error_fmt: "Could not read the policy file: {}",
    log_rule_managed: "{} is managed by your administrator",
    closing_in: "idle — closing in {} s",
    keep_open: "Keep open",
    close_now: "Close now",
    log_closed: "Closed {}",
    log_force_closed: "Force-closed {}",
    log_close_failed: "{} did not close (unsaved changes?) — timer restarted",
    log_paused: "Monitoring paused",
    log_resumed: "Monitoring resumed",
    log_bound: "{} started and bound for auto-close",
    log_started: "Started {}",
    log_start_failed: "Could not start {}",
    log_shortcut_ok: "Desktop shortcut created for {}",
    log_shortcut_fail: "Could not create a shortcut for {}",
    log_rule_added: "Rule added for {}",
    log_menu_installed: "Explorer right-click entry installed",
    log_menu_removed: "Explorer right-click entry removed",
    log_menu_failed: "Could not update the Explorer right-click entry",
    save_error_fmt: "Could not save config.toml: {}",
    tray_open: "Open settings",
    tray_pause: "Pause monitoring",
    tray_exit: "Exit",
    tray_tooltip: "auto-idle-close",
    tray_tooltip_paused: "auto-idle-close (paused)",
    explorer_verb: "Run with auto-idle-close",
    ago_s: "{}s ago",
    ago_m: "{}m ago",
    ago_h: "{}h ago",
};

pub static PL: Strings = Strings {
    btn_settings: "⚙ Ustawienia",
    lbl_paused: "⏸ wstrzymano",
    sec_running: "Uruchomione aplikacje",
    running_hint: "Zaznacz aplikację, aby zamykać ją automatycznie, gdy jest bezczynna.",
    filter_hint: "🔍 filtruj",
    scanning: "Skanowanie…",
    no_filter_match: "Żadne okno nie pasuje do filtra.",
    admin_row: "(administrator — brak dostępu)",
    sec_rules: "Reguły",
    no_rules: "Brak reguł — zaznacz aplikację powyżej.",
    remove_rule: "Usuń regułę",
    start_now: "▶ Uruchom teraz",
    create_shortcut: "Utwórz skrót na pulpicie",
    shortcut_hint: "Skrót „(auto-close)”, który uruchamia tę aplikację razem z \
                    auto-idle-close, dzięki czemu jest zawsze monitorowana.",
    force_after: "wymuś po",
    force_hint: "Jeśli aplikacja zignoruje grzeczną prośbę o zamknięcie (np. okno \
                 niezapisanych zmian), zakończ ją.",
    close_after_idle: "zamknij po bezczynności",
    add_by_name: "Dodaj po nazwie:",
    add_btn: "Dodaj",
    browse_btn: "Przeglądaj…",
    browse_hint: "Wskaż plik .exe na dysku — będzie można uruchamiać go stąd.",
    save_error_hint: "Przenieś plik exe do folderu z prawem zapisu, aby zachować ustawienia.",
    settings_title: "Ustawienia — Auto Idle Close",
    sec_general: "Ogólne",
    warn_before: "Ostrzeż przed zamknięciem:",
    pause_monitoring: "Wstrzymaj monitorowanie",
    language_label: "Język:",
    lang_auto: "Automatycznie (system)",
    sec_integration: "Integracja",
    autostart_checkbox: "Uruchamiaj przy starcie systemu (ukryty w zasobniku)",
    autostart_hint: "Uruchamia auto-idle-close automatycznie po zalogowaniu (wpis Run \
                     w rejestrze użytkownika, bez uprawnień administratora). Wpis \
                     wskazuje obecną lokalizację exe — po przeniesieniu pliku przełącz \
                     opcję ponownie.",
    log_autostart_on: "Włączono autostart przy logowaniu",
    log_autostart_off: "Wyłączono autostart przy logowaniu",
    log_autostart_failed: "Nie udało się zmienić wpisu autostartu",
    explorer_checkbox: "Menu prawego przycisku Eksploratora: „Uruchom z auto-idle-close”",
    explorer_hint: "Dodaje pozycję do menu prawego przycisku plików .exe (w Windows 11 \
                    pod „Pokaż więcej opcji”). Uruchomienie aplikacji w ten sposób \
                    startuje ją razem z auto-idle-close i automatycznie dodaje regułę.\n\
                    Uwaga: wpis wskazuje obecną lokalizację tego exe — po przeniesieniu \
                    pliku przełącz opcję ponownie.",
    sec_updates: "Aktualizacje",
    updates_checkbox: "Powiadamiaj o nowych wersjach (codzienne sprawdzanie)",
    updates_hint: "Raz dziennie aplikacja pyta github.com o najnowsze wydanie. \
                   Nic nie jest pobierane ani instalowane bez Twojego kliknięcia.",
    upd_check_now: "Sprawdź teraz",
    upd_checking: "Sprawdzanie…",
    upd_uptodate: "Wersja aktualna.",
    upd_available_fmt: "Dostępna jest nowa wersja {}.",
    upd_install: "Pobierz i zainstaluj",
    upd_notes: "Informacje o wydaniu",
    upd_downloading: "Pobieranie aktualizacji…",
    upd_restarting: "Ponowne uruchamianie…",
    upd_failed_fmt: "Aktualizacja nie powiodła się: {}",
    recent_activity: "Ostatnia aktywność",
    nothing_yet: "Na razie nic.",
    config_prefix: "Konfiguracja: ",
    policy_managed_hint: "zarządzane przez administratora",
    policy_banner: "Część ustawień jest zarządzana przez administratora.",
    policy_prefix: "Polityka: ",
    policy_error_fmt: "Nie można odczytać pliku polityki: {}",
    log_rule_managed: "{} jest zarządzane przez administratora",
    closing_in: "bezczynność — zamknięcie za {} s",
    keep_open: "Pozostaw",
    close_now: "Zamknij teraz",
    log_closed: "Zamknięto {}",
    log_force_closed: "Wymuszono zamknięcie {}",
    log_close_failed: "{} nie zamknęło się (niezapisane zmiany?) — licznik wyzerowany",
    log_paused: "Monitorowanie wstrzymane",
    log_resumed: "Monitorowanie wznowione",
    log_bound: "Uruchomiono {} i objęto automatycznym zamykaniem",
    log_started: "Uruchomiono {}",
    log_start_failed: "Nie udało się uruchomić {}",
    log_shortcut_ok: "Utworzono skrót na pulpicie dla {}",
    log_shortcut_fail: "Nie udało się utworzyć skrótu dla {}",
    log_rule_added: "Dodano regułę dla {}",
    log_menu_installed: "Dodano wpis do menu Eksploratora",
    log_menu_removed: "Usunięto wpis z menu Eksploratora",
    log_menu_failed: "Nie udało się zmienić wpisu menu Eksploratora",
    save_error_fmt: "Nie można zapisać config.toml: {}",
    tray_open: "Otwórz ustawienia",
    tray_pause: "Wstrzymaj monitorowanie",
    tray_exit: "Zakończ",
    tray_tooltip: "auto-idle-close",
    tray_tooltip_paused: "auto-idle-close (wstrzymano)",
    explorer_verb: "Uruchom z auto-idle-close",
    ago_s: "{} s temu",
    ago_m: "{} min temu",
    ago_h: "{} godz. temu",
};

/// Resolve the effective strings for a configured language.
pub fn strings(lang: Language) -> &'static Strings {
    match lang {
        Language::En => &EN,
        Language::Pl => &PL,
        Language::Auto => {
            if crate::win::system_language_is_polish() {
                &PL
            } else {
                &EN
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_substitutes_once() {
        assert_eq!(fmt("Closed {}", "notepad.exe"), "Closed notepad.exe");
        assert_eq!(fmt("{} s temu", 42), "42 s temu");
        assert_eq!(fmt("no placeholder", "x"), "no placeholder");
    }

    #[test]
    fn explicit_languages_resolve() {
        assert!(std::ptr::eq(strings(Language::En), &EN));
        assert!(std::ptr::eq(strings(Language::Pl), &PL));
    }
}
