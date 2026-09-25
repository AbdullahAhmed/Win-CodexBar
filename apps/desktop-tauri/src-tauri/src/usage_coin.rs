//! Detached, movable Codex usage coin. Provider data stays in the shared cache.

use codexbar::settings::Settings;
use tauri::{LogicalPosition, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};

use crate::geometry_store::{self, StoredGeometry};

pub const LABEL: &str = "usage-coin";
const SIZE: f64 = 78.0;

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
        window.show().map_err(|e| e.to_string())?;
        ensure_visible(&window);
        return Ok(());
    }

    let builder = tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("index.html?window=usage-coin".into()),
    )
    .title("Codex Usage Coin")
    .inner_size(SIZE, SIZE)
    .min_inner_size(SIZE, SIZE)
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
    window.show().map_err(|e| e.to_string())?;
    ensure_visible(&window);
    Ok(())
}

fn fully_inside_bounds(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    bounds_position: PhysicalPosition<i32>,
    bounds_size: PhysicalSize<u32>,
) -> bool {
    let (x, y) = (i64::from(position.x), i64::from(position.y));
    let (left, top) = (i64::from(bounds_position.x), i64::from(bounds_position.y));
    x >= left
        && y >= top
        && x + i64::from(size.width) <= left + i64::from(bounds_size.width)
        && y + i64::from(size.height) <= top + i64::from(bounds_size.height)
}

/// A remembered position can partly leave the monitor after a DPI or display
/// change. Bring the entire coin back onto the primary display in that case.
fn ensure_visible(window: &tauri::WebviewWindow) {
    let (Ok(position), Ok(size), Ok(monitors)) = (
        window.outer_position(),
        window.outer_size(),
        window.available_monitors(),
    ) else {
        return;
    };
    if monitors.is_empty()
        || monitors.iter().any(|monitor| {
            fully_inside_bounds(position, size, *monitor.position(), *monitor.size())
        })
    {
        return;
    }
    let target_monitor = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next());
    let Some(target_monitor) = target_monitor else {
        return;
    };
    let work_area = target_monitor.work_area();
    let scale = target_monitor.scale_factor();
    let target = PhysicalPosition::new(
        work_area.position.x + (24.0 * scale).round() as i32,
        work_area.position.y + (80.0 * scale).round() as i32,
    );
    if let Err(error) = window.set_position(target) {
        tracing::warn!(%error, "could not recover usage coin onto primary monitor");
        return;
    }
    geometry_store::save_entry(
        LABEL,
        StoredGeometry {
            x: (f64::from(target.x) / scale).round() as i32,
            y: (f64::from(target.y) / scale).round() as i32,
            width: None,
            height: None,
        },
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_coin_must_fit_entirely_on_a_display() {
        let monitor_position = PhysicalPosition::new(370, -1080);
        let monitor_size = PhysicalSize::new(1920, 1080);
        let coin_size = PhysicalSize::new(132, 78);
        assert!(fully_inside_bounds(
            PhysicalPosition::new(2100, -597),
            coin_size,
            monitor_position,
            monitor_size,
        ));
        assert!(!fully_inside_bounds(
            PhysicalPosition::new(2174, -597),
            coin_size,
            monitor_position,
            monitor_size,
        ));
        assert!(!fully_inside_bounds(
            PhysicalPosition::new(500, -1100),
            coin_size,
            monitor_position,
            monitor_size,
        ));
    }
}
