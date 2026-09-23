//! Detached, movable Codex usage coin. Provider data stays in the shared cache.

use codexbar::settings::Settings;
use tauri::{LogicalPosition, Manager, WebviewUrl};

use crate::geometry_store::{self, StoredGeometry};

pub const LABEL: &str = "usage-coin";
const SIZE: f64 = 156.0;

pub fn install(app: &tauri::AppHandle) {
    let settings = Settings::load();
    // Allows an isolated proof build to show the coin without editing the
    // user's persisted settings or the currently running installed app.
    let proof_requested = std::env::var_os("CODEXBAR_PROOF_COIN").is_some();
    if (settings.usage_coin_enabled || proof_requested)
        && let Err(error) = show(app, settings.usage_coin_always_on_top)
    {
        tracing::warn!(%error, "could not restore usage coin");
    }
}

fn show(app: &tauri::AppHandle, topmost: bool) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LABEL) {
        window
            .set_always_on_top(topmost)
            .map_err(|e| e.to_string())?;
        return window.show().map_err(|e| e.to_string());
    }

    let builder = tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("index.html?window=usage-coin".into()),
    )
    .title("Codex Usage Coin")
    .inner_size(SIZE, SIZE)
    .decorations(false)
    .shadow(false)
    .resizable(false)
    .always_on_top(topmost)
    .skip_taskbar(true)
    .theme(Some(tauri::Theme::Dark));
    #[cfg(windows)]
    let builder = builder.transparent(true);
    let window = builder
        .background_color(tauri::utils::config::Color(0, 0, 0, 0))
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;

    if let Some(pos) = geometry_store::load_entry(LABEL).filter(|p| p.x > -15_000 && p.y > -15_000)
    {
        let _ = window.set_position(LogicalPosition::new(pos.x as f64, pos.y as f64));
    } else if let Ok(Some(monitor)) = window.primary_monitor() {
        let scale = window.scale_factor().unwrap_or(1.0);
        let x = monitor.position().x as f64 / scale + 24.0;
        let y = monitor.position().y as f64 / scale + 80.0;
        let _ = window.set_position(LogicalPosition::new(x, y));
    }
    window.show().map_err(|e| e.to_string())
}

fn remember_position(window: &tauri::Window) {
    if window.is_minimized().unwrap_or(false) {
        return;
    }
    let (Ok(pos), Ok(size), Ok(monitors)) = (
        window.outer_position(),
        window.outer_size(),
        window.available_monitors(),
    ) else {
        return;
    };
    if pos.x <= -32_000 && pos.y <= -32_000 {
        return;
    }
    if !monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        pos.x < p.x + s.width as i32
            && pos.x + size.width as i32 > p.x
            && pos.y < p.y + s.height as i32
            && pos.y + size.height as i32 > p.y
    }) {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    geometry_store::save_entry(
        LABEL,
        StoredGeometry {
            x: (pos.x as f64 / scale).round() as i32,
            y: (pos.y as f64 / scale).round() as i32,
            width: None,
            height: None,
        },
    );
}

pub fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) -> bool {
    if window.label() != LABEL {
        return false;
    }
    match event {
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::CloseRequested { .. } => {
            remember_position(window);
        }
        _ => {}
    }
    true
}

pub fn toggle(app: &tauri::AppHandle) -> Result<(), String> {
    let mut settings = Settings::load();
    settings.usage_coin_enabled = !settings.usage_coin_enabled;
    settings.save().map_err(|e| e.to_string())?;
    if settings.usage_coin_enabled {
        show(app, settings.usage_coin_always_on_top)
    } else if let Some(window) = app.get_webview_window(LABEL) {
        window.close().map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub fn get_usage_coin_topmost() -> bool {
    Settings::load().usage_coin_always_on_top
}

#[tauri::command]
pub fn toggle_usage_coin_topmost(app: tauri::AppHandle) -> Result<bool, String> {
    let mut settings = Settings::load();
    let topmost = !settings.usage_coin_always_on_top;
    if let Some(window) = app.get_webview_window(LABEL) {
        window
            .set_always_on_top(topmost)
            .map_err(|e| e.to_string())?;
    }
    settings.usage_coin_always_on_top = topmost;
    settings.save().map_err(|e| e.to_string())?;
    Ok(topmost)
}
