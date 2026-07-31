//! The desktop shell: the window's lifecycle, the tray icon, and starting with the session.
//!
//! # Why the tray menu is built from labels the UI sends
//!
//! Every user-visible string in this product goes through an i18next key, and those live
//! in the frontend. A tray menu built in Rust would either duplicate them or bypass
//! translation entirely, so it does neither: the tray starts with an icon and no menu, and
//! the UI calls [`crate::commands::apply_tray_labels`] with the translated words on mount
//! and whenever the language changes. Adding Russian stays what CLAUDE.md promises — new
//! JSON, no code.
//!
//! Until that call arrives the tray has no menu, and closing the window therefore *quits*
//! rather than hiding: an application that vanished into a tray icon with no way back would
//! be worse than one that closed.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;

/// Identifier of the tray icon, so it can be looked up again to receive its menu.
pub const TRAY_ID: &str = "nm-tray";

/// Menu item that brings the window back.
const MENU_SHOW: &str = "nm-show";

/// Menu item that ends the application.
const MENU_QUIT: &str = "nm-quit";

/// Label of the main window in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

/// Translated words for the tray menu, supplied by the UI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TrayLabels {
    /// "Show the window".
    pub show: String,
    /// "Quit".
    pub quit: String,
}

/// Creates the tray icon, without a menu.
///
/// # Errors
///
/// Returns [`tauri::Error`] if the icon cannot be created — there is no useful degraded
/// mode, since the tray is how a minimized app is reached.
pub fn install_tray(app: &AppHandle<Wry>) -> Result<(), tauri::Error> {
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        // The product name is a proper noun and the same in every locale, which is why
        // this one string is not a translation key.
        .tooltip("Network Monitor")
        // A left click belongs to the window, not to a menu the user did not ask for.
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}

/// Gives the tray its menu, in the UI's language.
///
/// # Errors
///
/// Returns [`tauri::Error`] if the menu cannot be built or attached.
pub fn apply_tray_labels(app: &AppHandle<Wry>, labels: &TrayLabels) -> Result<(), tauri::Error> {
    let show = MenuItem::with_id(app, MENU_SHOW, &labels.show, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, &labels.quit, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &separator, &quit])?;

    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Err(tauri::Error::WebviewNotFound);
    };
    tray.set_menu(Some(menu))?;

    if let Some(state) = app.try_state::<AppState>() {
        state.mark_tray_ready();
    }
    Ok(())
}

/// Handles a click on a tray menu item.
///
/// The by-value event is Tauri's callback signature, not a choice.
#[allow(clippy::needless_pass_by_value)]
fn on_menu_event(app: &AppHandle<Wry>, event: MenuEvent) {
    match event.id().as_ref() {
        MENU_SHOW => show_window(app),
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

/// Shows the window and brings it forward.
pub fn show_window(app: &AppHandle<Wry>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    // Nothing useful follows a failure to show a window the user asked for; the tray icon
    // stays, so they can ask again.
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    set_visible(app, true);
}

/// Hides the window, leaving the core running.
pub fn hide_window(app: &AppHandle<Wry>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.hide();
    }
    set_visible(app, false);
}

/// Hides the window if it is showing, shows it otherwise.
fn toggle_window(app: &AppHandle<Wry>) {
    let showing = app
        .get_webview_window(MAIN_WINDOW)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);

    if showing {
        hide_window(app);
    } else {
        show_window(app);
    }
}

/// Tells the rest of the app whether the window can be drawn to.
///
/// This is what stops the UI rendering when it is out of sight: with the flag down the
/// monitor emits nothing, so the `WebView` is never woken to lay out a chart nobody can
/// see.
fn set_visible(app: &AppHandle<Wry>, visible: bool) {
    if let Some(state) = app.try_state::<AppState>() {
        state.set_visible(visible);
    }
}

/// Applies the autostart preference and reports what actually took effect.
///
/// The reported value is read back from the platform rather than echoed, so a request that
/// failed shows up as the toggle returning to where it was instead of a setting that claims
/// something untrue about the machine.
pub fn apply_autostart(app: &AppHandle<Wry>, wanted: bool) -> bool {
    use tauri_plugin_autostart::ManagerExt as _;

    let manager = app.autolaunch();
    let _ = if wanted {
        manager.enable()
    } else {
        manager.disable()
    };
    // Where the state cannot be read back at all, the request is taken at face value —
    // there is nothing better to report.
    manager.is_enabled().unwrap_or(wanted)
}
