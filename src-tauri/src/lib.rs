mod codex;
mod models;

use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

#[cfg(debug_assertions)]
use models::UsageWindow;
use models::{ProviderSnapshot, WidgetPreferences};
use serde::Deserialize;
use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::Color,
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_window_state::Builder as WindowStateBuilder;

const COLLAPSED_LOGICAL_SIZE: f64 = 80.0;
const EXPANDED_LOGICAL_SIZE: f64 = 320.0;
const EXPANDED_MIN_LOGICAL_SIZE: f64 = 260.0;
const SETTINGS_MAX_LOGICAL_WIDTH: f64 = 500.0;
const SETTINGS_MAX_LOGICAL_HEIGHT: f64 = 560.0;
const EDGE_SAFE_INSET_LOGICAL: f64 = 0.0;
const SNAP_THRESHOLD_LOGICAL: f64 = 24.0;
const POSITION_EPSILON: u32 = 2;
const TRAY_PREVIEW_WIDTH: f64 = 210.0;
const TRAY_PREVIEW_HEIGHT: f64 = 118.0;
#[derive(Clone, Copy)]
enum HorizontalDock {
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum VerticalDock {
    Top,
    Bottom,
}

#[derive(Clone, Copy, Default)]
struct DockState {
    horizontal: Option<HorizontalDock>,
    vertical: Option<VerticalDock>,
}

impl DockState {
    fn is_docked(self) -> bool {
        self.horizontal.is_some() || self.vertical.is_some()
    }
}

#[derive(Clone, Copy)]
struct WidgetRect {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
}

#[derive(Clone, Copy, Deserialize)]
struct WorkAreaPoint {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, Deserialize)]
struct WorkAreaSize {
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Deserialize)]
struct WorkAreaPayload {
    position: WorkAreaPoint,
    size: WorkAreaSize,
}

#[derive(Clone, Copy, Deserialize)]
struct LogicalSizePayload {
    width: f64,
    height: f64,
}

#[derive(Clone, Copy)]
enum WidgetMode {
    Collapsed,
    Expanded,
}

#[derive(Clone, Copy)]
struct WidgetGeometryState {
    mode: WidgetMode,
    dock: DockState,
    collapsed_rect: WidgetRect,
    expanded_rect: Option<WidgetRect>,
    user_moved_expanded: bool,
}

struct AppState {
    client: reqwest::Client,
    preferences: Mutex<WidgetPreferences>,
    preferences_path: PathBuf,
    fetch_lock: tokio::sync::Mutex<()>,
    snapshot_cache: Mutex<Option<(Instant, Vec<ProviderSnapshot>)>>,
    #[cfg(debug_assertions)]
    simulate_short_window_for_testing: Mutex<bool>,
    geometry: Mutex<Option<WidgetGeometryState>>,
    drag_mode: Mutex<Option<WidgetMode>>,
}

fn apply_short_window_test_override(
    _state: &AppState,
    #[allow(unused_mut)] mut snapshots: Vec<ProviderSnapshot>,
) -> Vec<ProviderSnapshot> {
    #[cfg(debug_assertions)]
    if _state
        .simulate_short_window_for_testing
        .lock()
        .map(|value| *value)
        .unwrap_or(false)
    {
        for snapshot in &mut snapshots {
            if snapshot.status == "ok" {
                snapshot.short_window = Some(UsageWindow {
                    remaining_percent: 88.0,
                    resets_at: Some((chrono::Utc::now() + chrono::Duration::hours(3)).to_rfc3339()),
                    window_seconds: 18_000,
                });
            }
        }
    }
    snapshots
}

async fn fetch_snapshots_uncached(state: &State<'_, AppState>) -> Vec<ProviderSnapshot> {
    let _guard = state.fetch_lock.lock().await;
    let values = vec![codex::fetch_snapshot(&state.client).await];
    if let Ok(mut cache) = state.snapshot_cache.lock() {
        *cache = Some((Instant::now(), values.clone()));
    }
    apply_short_window_test_override(state.inner(), values)
}

fn load_preferences(path: &PathBuf) -> WidgetPreferences {
    let parse = |candidate: &PathBuf| {
        fs::read_to_string(candidate)
            .ok()
            .and_then(|raw| serde_json::from_str::<WidgetPreferences>(&raw).ok())
    };
    if let Some(value) = parse(path) {
        return value.normalized();
    }
    let backup = path.with_extension("json.bak");
    if let Some(value) = parse(&backup) {
        eprintln!("preferences recovered from backup");
        return value.normalized();
    }
    WidgetPreferences::default()
}

fn persist_preferences(path: &PathBuf, value: &WidgetPreferences) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| "failed to create settings directory".to_string())?;
    }
    let serialized =
        serde_json::to_vec_pretty(value).map_err(|_| "failed to serialize settings".to_string())?;
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    let mut file = fs::File::create(&temporary)
        .map_err(|_| "failed to create temporary settings file".to_string())?;
    file.write_all(&serialized)
        .and_then(|_| file.sync_all())
        .map_err(|_| "failed to write settings".to_string())?;
    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup).map_err(|_| "failed to back up settings".to_string())?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(&backup, path);
        return Err(format!("failed to commit settings: {error}"));
    }
    Ok(())
}

#[tauri::command]
async fn get_snapshots(state: State<'_, AppState>) -> Result<Vec<ProviderSnapshot>, String> {
    const CACHE_TTL: Duration = Duration::from_secs(30);
    if let Ok(cache) = state.snapshot_cache.lock() {
        if let Some((time, values)) = &*cache {
            if time.elapsed() < CACHE_TTL {
                return Ok(apply_short_window_test_override(&state, values.clone()));
            }
        }
    }
    let _guard = match state.fetch_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            if let Ok(cache) = state.snapshot_cache.lock() {
                if let Some((_, values)) = &*cache {
                    return Ok(apply_short_window_test_override(&state, values.clone()));
                }
            }
            return Ok(vec![ProviderSnapshot::failure(
                "unavailable",
                "Quota refresh is already running.",
            )]);
        }
    };
    if let Ok(cache) = state.snapshot_cache.lock() {
        if let Some((time, values)) = &*cache {
            if time.elapsed() < CACHE_TTL {
                return Ok(apply_short_window_test_override(&state, values.clone()));
            }
        }
    }
    let values = vec![codex::fetch_snapshot(&state.client).await];
    if let Ok(mut cache) = state.snapshot_cache.lock() {
        *cache = Some((Instant::now(), values.clone()));
    }
    Ok(apply_short_window_test_override(&state, values))
}

#[tauri::command]
async fn refresh_snapshots(state: State<'_, AppState>) -> Result<Vec<ProviderSnapshot>, String> {
    Ok(fetch_snapshots_uncached(&state).await)
}

fn clamp_position_to_monitor(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    monitor: &tauri::Monitor,
    safe_inset: i32,
) -> PhysicalPosition<i32> {
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let left = monitor_position.x;
    let top = monitor_position.y;
    let right = left + monitor_size.width as i32;
    let bottom = top + monitor_size.height as i32;
    PhysicalPosition::new(
        position
            .x
            .clamp(left - safe_inset, right - size.width as i32 + safe_inset),
        position
            .y
            .clamp(top - safe_inset, bottom - size.height as i32 + safe_inset),
    )
}

fn logical_to_physical(value: f64, scale_factor: f64) -> u32 {
    if value <= 0.0 {
        return 0;
    }
    (value * scale_factor).round().max(1.0) as u32
}

fn window_size_for_visual_size(visual_size: u32, safe_inset: u32) -> u32 {
    visual_size + safe_inset * 2
}

fn widget_window_size(logical_visual_size: f64, scale_factor: f64, safe_inset: u32) -> u32 {
    window_size_for_visual_size(
        logical_to_physical(logical_visual_size, scale_factor),
        safe_inset,
    )
}

fn expanded_window_size(
    logical_size: Option<LogicalSizePayload>,
    scale_factor: f64,
    safe_inset: u32,
) -> PhysicalSize<u32> {
    let width = logical_size
        .map(|value| {
            value
                .width
                .clamp(EXPANDED_MIN_LOGICAL_SIZE, SETTINGS_MAX_LOGICAL_WIDTH)
        })
        .unwrap_or(EXPANDED_LOGICAL_SIZE);
    let height = logical_size
        .map(|value| {
            value
                .height
                .clamp(EXPANDED_MIN_LOGICAL_SIZE, SETTINGS_MAX_LOGICAL_HEIGHT)
        })
        .unwrap_or(EXPANDED_LOGICAL_SIZE);
    PhysicalSize::new(
        widget_window_size(width, scale_factor, safe_inset),
        widget_window_size(height, scale_factor, safe_inset),
    )
}

fn detect_dock(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    monitor: &tauri::Monitor,
    threshold: i32,
    safe_inset: i32,
) -> DockState {
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let visible_left = position.x + safe_inset;
    let visible_top = position.y + safe_inset;
    let visible_right = position.x + size.width as i32 - safe_inset;
    let visible_bottom = position.y + size.height as i32 - safe_inset;
    let left_distance = (visible_left - monitor_position.x).abs();
    let top_distance = (visible_top - monitor_position.y).abs();
    let right_distance = (monitor_position.x + monitor_size.width as i32 - visible_right).abs();
    let bottom_distance = (monitor_position.y + monitor_size.height as i32 - visible_bottom).abs();
    let horizontal = if left_distance <= threshold || right_distance <= threshold {
        if left_distance <= right_distance {
            Some(HorizontalDock::Left)
        } else {
            Some(HorizontalDock::Right)
        }
    } else {
        None
    };
    let vertical = if top_distance <= threshold || bottom_distance <= threshold {
        if top_distance <= bottom_distance {
            Some(VerticalDock::Top)
        } else {
            Some(VerticalDock::Bottom)
        }
    } else {
        None
    };
    DockState {
        horizontal,
        vertical,
    }
}

fn snap_position(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    dock: DockState,
    monitor: &tauri::Monitor,
    safe_inset: i32,
) -> PhysicalPosition<i32> {
    let monitor_position = monitor.position();
    let monitor_size = monitor.size();
    let mut next = clamp_position_to_monitor(position, size, monitor, safe_inset);
    match dock.horizontal {
        Some(HorizontalDock::Left) => next.x = monitor_position.x - safe_inset,
        Some(HorizontalDock::Right) => {
            next.x = monitor_position.x + monitor_size.width as i32 - size.width as i32 + safe_inset
        }
        None => {}
    }
    match dock.vertical {
        Some(VerticalDock::Top) => next.y = monitor_position.y - safe_inset,
        Some(VerticalDock::Bottom) => {
            next.y =
                monitor_position.y + monitor_size.height as i32 - size.height as i32 + safe_inset
        }
        None => {}
    }
    next
}

fn expanded_position_in_bounds(
    collapsed: WidgetRect,
    expanded_size: PhysicalSize<u32>,
    dock: DockState,
    bounds_position: PhysicalPosition<i32>,
    bounds_size: PhysicalSize<u32>,
    safe_inset: i32,
) -> PhysicalPosition<i32> {
    let monitor_right = bounds_position.x + bounds_size.width as i32;
    let monitor_bottom = bounds_position.y + bounds_size.height as i32;
    let collapsed_left = collapsed.position.x + safe_inset;
    let collapsed_top = collapsed.position.y + safe_inset;
    let collapsed_right = collapsed.position.x + collapsed.size.width as i32 - safe_inset;
    let collapsed_bottom = collapsed.position.y + collapsed.size.height as i32 - safe_inset;
    let x = match dock.horizontal {
        Some(HorizontalDock::Left) => collapsed_left - safe_inset,
        Some(HorizontalDock::Right) => collapsed_right - expanded_size.width as i32 + safe_inset,
        None if collapsed_left + expanded_size.width as i32 - safe_inset > monitor_right => {
            collapsed_right - expanded_size.width as i32 + safe_inset
        }
        None => collapsed_left - safe_inset,
    };
    let y = match dock.vertical {
        Some(VerticalDock::Top) => collapsed_top - safe_inset,
        Some(VerticalDock::Bottom) => collapsed_bottom - expanded_size.height as i32 + safe_inset,
        None if collapsed_top + expanded_size.height as i32 - safe_inset > monitor_bottom => {
            collapsed_bottom - expanded_size.height as i32 + safe_inset
        }
        None => collapsed_top - safe_inset,
    };
    let min_x = bounds_position.x - safe_inset;
    let min_y = bounds_position.y - safe_inset;
    let max_x = (monitor_right - expanded_size.width as i32 + safe_inset).max(min_x);
    let max_y = (monitor_bottom - expanded_size.height as i32 + safe_inset).max(min_y);
    PhysicalPosition::new(x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

fn expanded_position(
    collapsed: WidgetRect,
    expanded_size: PhysicalSize<u32>,
    dock: DockState,
    monitor: &tauri::Monitor,
    work_area: Option<WorkAreaPayload>,
    safe_inset: i32,
) -> PhysicalPosition<i32> {
    let (bounds_position, bounds_size) = work_area
        .map(|area| {
            (
                PhysicalPosition::new(area.position.x, area.position.y),
                PhysicalSize::new(area.size.width, area.size.height),
            )
        })
        .unwrap_or_else(|| (*monitor.position(), *monitor.size()));
    expanded_position_in_bounds(
        collapsed,
        expanded_size,
        dock,
        bounds_position,
        bounds_size,
        safe_inset,
    )
}

fn collapsed_geometry_for_expand(
    current_position: PhysicalPosition<i32>,
    collapsed_size: PhysicalSize<u32>,
    monitor: &tauri::Monitor,
    threshold: i32,
    safe_inset: i32,
    previous: Option<WidgetGeometryState>,
) -> (WidgetRect, DockState) {
    if let Some(previous) = previous {
        let can_reuse_anchor = matches!(previous.mode, WidgetMode::Collapsed)
            || (matches!(previous.mode, WidgetMode::Expanded) && !previous.user_moved_expanded);
        if can_reuse_anchor {
            let position = if previous.dock.is_docked() {
                snap_position(
                    previous.collapsed_rect.position,
                    collapsed_size,
                    previous.dock,
                    monitor,
                    safe_inset,
                )
            } else {
                clamp_position_to_monitor(
                    previous.collapsed_rect.position,
                    collapsed_size,
                    monitor,
                    safe_inset,
                )
            };
            return (
                WidgetRect {
                    position,
                    size: collapsed_size,
                },
                previous.dock,
            );
        }
    }

    let current_collapsed = WidgetRect {
        position: clamp_position_to_monitor(current_position, collapsed_size, monitor, safe_inset),
        size: collapsed_size,
    };
    let dock = detect_dock(
        current_collapsed.position,
        collapsed_size,
        monitor,
        threshold,
        safe_inset,
    );
    let position = if dock.is_docked() {
        snap_position(
            current_collapsed.position,
            collapsed_size,
            dock,
            monitor,
            safe_inset,
        )
    } else {
        current_collapsed.position
    };
    (
        WidgetRect {
            position,
            size: collapsed_size,
        },
        dock,
    )
}

fn current_widget_rect(window: &tauri::WebviewWindow) -> Result<WidgetRect, String> {
    Ok(WidgetRect {
        position: window
            .outer_position()
            .map_err(|_| "failed to read widget position".to_string())?,
        size: window
            .outer_size()
            .map_err(|_| "failed to read widget size".to_string())?,
    })
}

fn monitor_and_scale(
    window: &tauri::WebviewWindow,
) -> Result<(Option<tauri::Monitor>, f64), String> {
    let monitor = window
        .current_monitor()
        .map_err(|_| "failed to read monitor".to_string())?;
    let scale_factor = monitor
        .as_ref()
        .map(|item| item.scale_factor())
        .unwrap_or(1.0);
    Ok((monitor, scale_factor))
}

fn infer_mode(rect: WidgetRect, collapsed_size: PhysicalSize<u32>) -> WidgetMode {
    if rect.size.width <= collapsed_size.width + POSITION_EPSILON
        && rect.size.height <= collapsed_size.height + POSITION_EPSILON
    {
        WidgetMode::Collapsed
    } else {
        WidgetMode::Expanded
    }
}

#[cfg(windows)]
fn apply_widget_region(
    hwnd: windows::Win32::Foundation::HWND,
    size: PhysicalSize<u32>,
    scale_factor: f64,
) {
    use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ};

    let width = size.width.min(i32::MAX as u32) as i32;
    let height = size.height.min(i32::MAX as u32) as i32;
    if width <= 0 || height <= 0 {
        return;
    }

    let collapsed_edge = logical_to_physical(COLLAPSED_LOGICAL_SIZE + 2.0, scale_factor);
    let corner_radius = if size.width.min(size.height) <= collapsed_edge {
        28.0
    } else {
        38.0
    };
    let corner_diameter = logical_to_physical(corner_radius * 2.0, scale_factor)
        .max(2)
        .min(i32::MAX as u32) as i32;

    unsafe {
        let region = CreateRoundRectRgn(
            0,
            0,
            width + 1,
            height + 1,
            corner_diameter,
            corner_diameter,
        );
        if region.is_invalid() {
            return;
        }
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(region.0));
        }
    }
}

#[cfg(windows)]
fn apply_widget_window_region(window: &tauri::WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else { return };
    let Ok(size) = window.outer_size() else {
        return;
    };
    apply_widget_region(hwnd, size, window.scale_factor().unwrap_or(1.0));
}

#[cfg(not(windows))]
fn apply_widget_window_region(_: &tauri::WebviewWindow) {}

#[cfg(windows)]
fn apply_widget_event_window_region(window: &tauri::Window) {
    let Ok(hwnd) = window.hwnd() else { return };
    let Ok(size) = window.outer_size() else {
        return;
    };
    apply_widget_region(hwnd, size, window.scale_factor().unwrap_or(1.0));
}

#[cfg(not(windows))]
fn apply_widget_event_window_region(_: &tauri::Window) {}

fn tray_progress_color(tier: &str) -> (u8, u8, u8) {
    match tier {
        "caution" => (255, 154, 231),
        "critical" => (255, 111, 181),
        "signed_out" | "unavailable" | "stale" => (148, 163, 184),
        _ => (255, 120, 242),
    }
}

fn set_pixel(rgba: &mut [u8], size: u32, x: u32, y: u32, color: (u8, u8, u8, u8)) {
    if x >= size || y >= size {
        return;
    }
    let index = ((y * size + x) * 4) as usize;
    rgba[index] = color.0;
    rgba[index + 1] = color.1;
    rgba[index + 2] = color.2;
    rgba[index + 3] = color.3;
}

fn draw_rounded_rect(
    rgba: &mut [u8],
    size: u32,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    radius: i32,
    color: (u8, u8, u8, u8),
) {
    let right = left + width;
    let bottom = top + height;
    for y in top..bottom {
        for x in left..right {
            let dx = if x < left + radius as u32 {
                left + radius as u32 - x
            } else if x >= right.saturating_sub(radius as u32) {
                x - (right - radius as u32 - 1)
            } else {
                0
            } as i32;
            let dy = if y < top + radius as u32 {
                top + radius as u32 - y
            } else if y >= bottom.saturating_sub(radius as u32) {
                y - (bottom - radius as u32 - 1)
            } else {
                0
            } as i32;
            if dx == 0 || dy == 0 || dx * dx + dy * dy <= radius * radius {
                set_pixel(rgba, size, x, y, color);
            }
        }
    }
}

fn tray_progress_icon(percent: f64, tier: &str) -> Image<'static> {
    const WIDTH: u32 = 42;
    const HEIGHT: u32 = 32;
    let mut rgba = vec![0; (WIDTH * HEIGHT * 4) as usize];
    let percent = percent.clamp(0.0, 100.0);
    let fill_width = if percent <= 0.0 {
        0
    } else {
        ((40.0 * percent / 100.0).round() as u32).clamp(20, 40)
    };
    let (r, g, b) = tray_progress_color(tier);

    draw_rounded_rect(&mut rgba, WIDTH, 0, 9, 42, 13, 6, (255, 255, 255, 135));
    draw_rounded_rect(&mut rgba, WIDTH, 1, 10, 40, 11, 5, (18, 24, 40, 238));
    draw_rounded_rect(&mut rgba, WIDTH, 2, 11, 38, 9, 4, (74, 82, 102, 172));
    draw_rounded_rect(&mut rgba, WIDTH, 2, 11, 38, 1, 1, (255, 255, 255, 90));
    if fill_width > 0 {
        draw_rounded_rect(&mut rgba, WIDTH, 1, 10, fill_width, 11, 5, (r, g, b, 255));
        draw_rounded_rect(&mut rgba, WIDTH, 2, 11, fill_width.saturating_sub(2), 3, 2, (255, 255, 255, 112));
        draw_rounded_rect(&mut rgba, WIDTH, 2, 18, fill_width.saturating_sub(2), 2, 1, (0, 0, 0, 38));
    }

    Image::new_owned(rgba, WIDTH, HEIGHT)
}

fn set_progress_tray_icon(
    app: &AppHandle,
    visible: bool,
    percent: f64,
    _tooltip: &str,
    tier: &str,
) -> Result<(), String> {
    let Some(tray) = app.tray_by_id("main") else {
        return Ok(());
    };
    if visible {
        tray.set_icon(Some(tray_progress_icon(percent, tier)))
            .map_err(|_| "failed to update tray progress icon".to_string())?;
        let _ = tray.set_tooltip(None::<String>);
        return Ok(());
    }
    if let Some(icon) = app.default_window_icon() {
        let _ = tray.set_icon(Some(icon.clone()));
    }
    let _ = tray.set_tooltip(Some("Quota Float Pro"));
    Ok(())
}

fn set_widget_visibility(app: &AppHandle, visible: bool) {
    if let Some(window) = app.get_webview_window("widget") {
        if visible {
            let _ = window.show();
        } else {
            let _ = window.hide();
        }
    }
}

#[tauri::command]
fn set_widget_visible(visible: bool, app: AppHandle) -> Result<(), String> {
    set_widget_visibility(&app, visible);
    Ok(())
}

fn hide_tray_preview(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("statusbar") {
        let _ = window.hide();
    }
}

fn show_tray_preview(app: &AppHandle, rect: tauri::Rect) {
    let Some(window) = app.get_webview_window("statusbar") else {
        return;
    };
    let visible = app
        .try_state::<AppState>()
        .and_then(|state| state.preferences.lock().ok().map(|prefs| prefs.show_status_bar_progress))
        .unwrap_or(false);
    if !visible {
        let _ = window.hide();
        return;
    }
    let scale_factor = window.scale_factor().unwrap_or(1.0);
    let width = logical_to_physical(TRAY_PREVIEW_WIDTH, scale_factor);
    let height = logical_to_physical(TRAY_PREVIEW_HEIGHT, scale_factor);
    let rect_position = rect.position.to_physical::<i32>(scale_factor);
    let rect_size = rect.size.to_physical::<u32>(scale_factor);
    let anchor_x = rect_position.x as f64 + rect_size.width as f64 / 2.0;
    let mut x = (anchor_x - width as f64 / 2.0).round() as i32;
    let mut y = (rect_position.y as f64 - height as f64 - 10.0).round() as i32;

    if let Ok(Some(monitor)) = app.monitor_from_point(rect_position.x as f64, rect_position.y as f64) {
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        x = x.clamp(
            monitor_position.x + 4,
            monitor_position.x + monitor_size.width as i32 - width as i32 - 4,
        );
        if y < monitor_position.y + 4 {
            y = (rect_position.y as f64 + rect_size.height as f64 + 10.0).round() as i32;
        }
    }

    let _ = window.set_size(PhysicalSize::new(width, height));
    let _ = window.set_position(PhysicalPosition::new(x, y));
    let _ = window.show();
    let _ = window.set_focus();
}

#[tauri::command]
fn expand_widget(
    work_area: Option<WorkAreaPayload>,
    logical_size: Option<LogicalSizePayload>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    let current = current_widget_rect(&window)?;
    let (monitor, scale_factor) = monitor_and_scale(&window)?;
    let safe_inset = logical_to_physical(EDGE_SAFE_INSET_LOGICAL, scale_factor);
    let collapsed_size = PhysicalSize::new(
        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
    );
    let expanded_size = expanded_window_size(logical_size, scale_factor, safe_inset);
    let Some(monitor) = monitor else {
        window
            .set_size(expanded_size)
            .map_err(|_| "failed to resize widget".to_string())?;
        apply_widget_window_region(&window);
        return Ok(());
    };
    let threshold = logical_to_physical(SNAP_THRESHOLD_LOGICAL, scale_factor) as i32;
    let previous = state.geometry.lock().ok().and_then(|value| *value);
    let (collapsed_rect, dock) = collapsed_geometry_for_expand(
        current.position,
        collapsed_size,
        &monitor,
        threshold,
        safe_inset as i32,
        previous,
    );
    let expanded_rect = WidgetRect {
        position: expanded_position(
            collapsed_rect,
            expanded_size,
            dock,
            &monitor,
            work_area,
            safe_inset as i32,
        ),
        size: expanded_size,
    };

    if let Ok(mut geometry) = state.geometry.lock() {
        *geometry = Some(WidgetGeometryState {
            mode: WidgetMode::Expanded,
            dock,
            collapsed_rect,
            expanded_rect: Some(expanded_rect),
            user_moved_expanded: false,
        });
    }

    window
        .set_position(expanded_rect.position)
        .map_err(|_| "failed to position widget".to_string())?;
    window
        .set_size(expanded_size)
        .map_err(|_| "failed to resize widget".to_string())?;
    apply_widget_window_region(&window);
    Ok(())
}

#[cfg(test)]
mod geometry_tests {
    use super::*;

    fn rect(x: i32, y: i32, size: u32) -> WidgetRect {
        WidgetRect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(size, size),
        }
    }

    #[test]
    fn window_size_includes_the_transparent_safe_inset() {
        assert_eq!(window_size_for_visual_size(80, 4), 88);
        assert_eq!(widget_window_size(320.0, 1.5, 6), 492);
        assert_eq!(logical_to_physical(0.0, 1.5), 0);
        assert_eq!(widget_window_size(80.0, 1.0, logical_to_physical(0.0, 1.0)), 80);
    }

    #[test]
    fn expansion_stays_above_a_bottom_taskbar() {
        let position = expanded_position_in_bounds(
            rect(1812, 952, 88),
            PhysicalSize::new(328, 328),
            DockState {
                horizontal: Some(HorizontalDock::Right),
                vertical: Some(VerticalDock::Bottom),
            },
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1040),
            4,
        );
        assert_eq!(position, PhysicalPosition::new(1572, 712));
    }

    #[test]
    fn expansion_handles_negative_origin_work_areas() {
        let position = expanded_position_in_bounds(
            rect(-1284, -4, 88),
            PhysicalSize::new(328, 328),
            DockState {
                horizontal: Some(HorizontalDock::Left),
                vertical: Some(VerticalDock::Top),
            },
            PhysicalPosition::new(-1280, 0),
            PhysicalSize::new(1280, 984),
            4,
        );
        assert_eq!(position, PhysicalPosition::new(-1284, -4));
    }

    #[test]
    fn undocked_expansion_flips_inward_near_work_area_edges() {
        let position = expanded_position_in_bounds(
            rect(1750, 900, 88),
            PhysicalSize::new(328, 328),
            DockState::default(),
            PhysicalPosition::new(0, 0),
            PhysicalSize::new(1920, 1040),
            4,
        );
        assert_eq!(position, PhysicalPosition::new(1510, 660));
    }

}

#[tauri::command]
fn collapse_widget(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    let current = current_widget_rect(&window)?;
    let (monitor, scale_factor) = monitor_and_scale(&window)?;
    let safe_inset = logical_to_physical(EDGE_SAFE_INSET_LOGICAL, scale_factor);
    let collapsed_size = PhysicalSize::new(
        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
    );
    let Some(monitor) = monitor else {
        window
            .set_size(collapsed_size)
            .map_err(|_| "failed to resize widget".to_string())?;
        apply_widget_window_region(&window);
        return Ok(());
    };
    let threshold = logical_to_physical(SNAP_THRESHOLD_LOGICAL, scale_factor) as i32;
    let previous = state.geometry.lock().ok().and_then(|value| *value);
    let user_moved_expanded = previous
        .map(|value| value.user_moved_expanded)
        .unwrap_or(false);
    let candidate = if user_moved_expanded {
        current.position
    } else {
        previous
            .map(|value| value.collapsed_rect.position)
            .unwrap_or(current.position)
    };
    let dock = detect_dock(
        candidate,
        collapsed_size,
        &monitor,
        threshold,
        safe_inset as i32,
    );
    let next_position = if dock.is_docked() {
        snap_position(candidate, collapsed_size, dock, &monitor, safe_inset as i32)
    } else {
        clamp_position_to_monitor(candidate, collapsed_size, &monitor, safe_inset as i32)
    };
    let collapsed_rect = WidgetRect {
        position: next_position,
        size: collapsed_size,
    };
    if let Ok(mut geometry) = state.geometry.lock() {
        *geometry = Some(WidgetGeometryState {
            mode: WidgetMode::Collapsed,
            dock,
            collapsed_rect,
            expanded_rect: None,
            user_moved_expanded: false,
        });
    }
    window
        .set_size(collapsed_size)
        .map_err(|_| "failed to resize widget".to_string())?;
    window
        .set_position(next_position)
        .map_err(|_| "failed to position widget".to_string())?;
    apply_widget_window_region(&window);
    Ok(())
}

#[tauri::command]
fn begin_widget_drag(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    let current = current_widget_rect(&window)?;
    let (_, scale_factor) = monitor_and_scale(&window)?;
    let safe_inset = logical_to_physical(EDGE_SAFE_INSET_LOGICAL, scale_factor);
    let collapsed_size = PhysicalSize::new(
        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
    );
    let mode = state
        .geometry
        .lock()
        .ok()
        .and_then(|value| *value)
        .map(|value| value.mode)
        .unwrap_or_else(|| infer_mode(current, collapsed_size));
    if let Ok(mut drag_mode) = state.drag_mode.lock() {
        *drag_mode = Some(mode);
    }
    Ok(())
}

#[tauri::command]
fn finish_widget_drag(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    let current = current_widget_rect(&window)?;
    let (monitor, scale_factor) = monitor_and_scale(&window)?;
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let threshold = logical_to_physical(SNAP_THRESHOLD_LOGICAL, scale_factor) as i32;
    let safe_inset = logical_to_physical(EDGE_SAFE_INSET_LOGICAL, scale_factor);
    let collapsed_size = PhysicalSize::new(
        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
    );
    let expanded_size = PhysicalSize::new(
        widget_window_size(EXPANDED_LOGICAL_SIZE, scale_factor, safe_inset),
        widget_window_size(EXPANDED_LOGICAL_SIZE, scale_factor, safe_inset),
    );
    let mode = state
        .drag_mode
        .lock()
        .ok()
        .and_then(|mut value| value.take())
        .or_else(|| {
            state
                .geometry
                .lock()
                .ok()
                .and_then(|value| *value)
                .map(|value| value.mode)
        })
        .unwrap_or_else(|| infer_mode(current, collapsed_size));

    match mode {
        WidgetMode::Collapsed => {
            let dock = detect_dock(
                current.position,
                collapsed_size,
                &monitor,
                threshold,
                safe_inset as i32,
            );
            let next_position = if dock.is_docked() {
                snap_position(
                    current.position,
                    collapsed_size,
                    dock,
                    &monitor,
                    safe_inset as i32,
                )
            } else {
                clamp_position_to_monitor(
                    current.position,
                    collapsed_size,
                    &monitor,
                    safe_inset as i32,
                )
            };
            let collapsed_rect = WidgetRect {
                position: next_position,
                size: collapsed_size,
            };
            window
                .set_position(next_position)
                .map_err(|_| "failed to position widget".to_string())?;
            if let Ok(mut geometry) = state.geometry.lock() {
                *geometry = Some(WidgetGeometryState {
                    mode: WidgetMode::Collapsed,
                    dock,
                    collapsed_rect,
                    expanded_rect: None,
                    user_moved_expanded: false,
                });
            }
        }
        WidgetMode::Expanded => {
            let current_position = clamp_position_to_monitor(
                current.position,
                expanded_size,
                &monitor,
                safe_inset as i32,
            );
            let updated_rect = WidgetRect {
                position: current_position,
                size: expanded_size,
            };
            window
                .set_position(current_position)
                .map_err(|_| "failed to position widget".to_string())?;
            if let Ok(mut geometry) = state.geometry.lock() {
                if let Some(mut value) = *geometry {
                    value.mode = WidgetMode::Expanded;
                    value.expanded_rect = Some(updated_rect);
                    value.user_moved_expanded = true;
                    *geometry = Some(value);
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
fn get_preferences(state: State<'_, AppState>) -> Result<WidgetPreferences, String> {
    state
        .preferences
        .lock()
        .map(|value| value.clone())
        .map_err(|_| "settings unavailable".into())
}

#[tauri::command]
fn set_preferences(
    preferences: WidgetPreferences,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let preferences = preferences.normalized();
    persist_preferences(&state.preferences_path, &preferences)?;
    *state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())? = preferences.clone();
    let _ = set_status_bar_progress_visible(
        preferences.show_status_bar_progress,
        None,
        app.clone(),
    );
    let _ = app.emit("preferences-changed", preferences);
    Ok(())
}

fn apply_lock(app: &AppHandle, locked: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    window
        .set_ignore_cursor_events(locked)
        .map_err(|_| "failed to toggle click-through".to_string())
}

#[tauri::command]
fn set_widget_locked(
    locked: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WidgetPreferences, String> {
    let previous = state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())?
        .clone();
    let mut next = previous.clone();
    next.locked = locked;
    persist_preferences(&state.preferences_path, &next)?;
    if let Err(error) = apply_lock(&app, locked) {
        let _ = persist_preferences(&state.preferences_path, &previous);
        return Err(error);
    }
    *state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())? = next.clone();
    let _ = app.emit("preferences-changed", next.clone());
    Ok(next)
}

#[tauri::command]
fn set_widget_always_on_top(
    always_on_top: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<WidgetPreferences, String> {
    let previous = state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())?
        .clone();
    let mut next = previous.clone();
    next.always_on_top = always_on_top;
    persist_preferences(&state.preferences_path, &next)?;
    let window = app
        .get_webview_window("widget")
        .ok_or_else(|| "widget window missing".to_string())?;
    if let Err(error) = window.set_always_on_top(always_on_top) {
        let _ = persist_preferences(&state.preferences_path, &previous);
        return Err(format!("failed to toggle always-on-top: {error}"));
    }
    *state
        .preferences
        .lock()
        .map_err(|_| "settings unavailable".to_string())? = next.clone();
    let _ = app.emit("preferences-changed", next.clone());
    Ok(next)
}

#[tauri::command]
fn set_status_bar_progress_visible(
    visible: bool,
    _work_area: Option<WorkAreaPayload>,
    app: AppHandle,
) -> Result<(), String> {
    hide_tray_preview(&app);
    set_widget_visibility(&app, !visible);
    set_progress_tray_icon(&app, visible, 100.0, "额度读取中", "healthy")
}

#[tauri::command]
fn set_tray_progress(
    visible: bool,
    percent: f64,
    tooltip: String,
    tier: String,
    app: AppHandle,
) -> Result<(), String> {
    set_progress_tray_icon(&app, visible, percent, &tooltip, &tier)
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show / Hide", true, None::<&str>)?;
    let refresh = MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
    let update = MenuItem::with_id(app, "update", "Check for updates", true, None::<&str>)?;
    let unlock = MenuItem::with_id(app, "unlock", "Unlock widget", true, None::<&str>)?;
    let pin = MenuItem::with_id(app, "pin", "Pin / Unpin Codex", true, None::<&str>)?;
    let theme = MenuItem::with_id(app, "theme", "Theme", true, None::<&str>)?;
    let language = MenuItem::with_id(
        app,
        "language",
        "Switch Language / 切换语言",
        true,
        None::<&str>,
    )?;
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start at login",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let initial_preferences = app
        .try_state::<AppState>()
        .and_then(|state| state.preferences.lock().ok().map(|prefs| prefs.clone()))
        .unwrap_or_default();
    if initial_preferences.language != "en" {
        let _ = show.set_text("显示 / 隐藏");
        let _ = refresh.set_text("立即刷新");
        let _ = update.set_text("检查更新");
        let _ = unlock.set_text("解锁悬浮窗");
        let _ = pin.set_text("固定 / 取消固定 Codex");
        let _ = theme.set_text("主题");
        let _ = language.set_text("Switch to English");
        let _ = autostart.set_text("开机启动");
        let _ = quit.set_text("退出");
    }
    let menu = Menu::with_items(
        app,
        &[
            &show, &refresh, &update, &unlock, &pin, &theme, &language, &autostart, &quit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .tooltip("Quota Float");
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let autostart_menu = autostart.clone();
    let show_menu = show.clone();
    let refresh_menu = refresh.clone();
    let update_menu = update.clone();
    let unlock_menu = unlock.clone();
    let pin_menu = pin.clone();
    let theme_menu = theme.clone();
    let language_menu = language.clone();
    let quit_menu = quit.clone();
    builder
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => {
                let status_visible = app
                    .try_state::<AppState>()
                    .and_then(|state| state.preferences.lock().ok().map(|prefs| prefs.show_status_bar_progress))
                    .unwrap_or(false);
                if status_visible {
                    return;
                }
                if let Some(window) = app.get_webview_window("widget") {
                    if window.is_visible().unwrap_or(false) {
                        let _ = window.hide();
                    } else {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
            }
            "refresh" => {
                let _ = app.emit_to("widget", "refresh-requested", ());
            }
            "update" => {
                let _ = app.emit_to("widget", "update-check-requested", ());
            }
            "unlock" => {
                let _ = apply_lock(app, false);
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut prefs) = state.preferences.lock() {
                        prefs.locked = false;
                        let _ = persist_preferences(&state.preferences_path, &prefs);
                        let _ = app.emit("preferences-changed", prefs.clone());
                    }
                }
            }
            "pin" => {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut prefs) = state.preferences.lock() {
                        prefs.pinned_provider = if prefs.pinned_provider.is_some() {
                            None
                        } else {
                            Some("codex".into())
                        };
                        let _ = persist_preferences(&state.preferences_path, &prefs);
                        let _ = app.emit("preferences-changed", prefs.clone());
                    }
                }
            }
            "theme" => {
                if let Some(window) = app.get_webview_window("settings") {
                    if !window.is_visible().unwrap_or(false) {
                        let _ = window.show();
                    }
                    let _ = window.set_focus();
                }
            }
            "language" => {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut prefs) = state.preferences.lock() {
                        prefs.language = if prefs.language == "en" {
                            "zh-CN".into()
                        } else {
                            "en".into()
                        };
                        let normalized = prefs.clone().normalized();
                        *prefs = normalized.clone();
                        let _ = persist_preferences(&state.preferences_path, &normalized);
                        let english = normalized.language == "en";
                        let _ = show_menu.set_text(if english {
                            "Show / Hide"
                        } else {
                            "显示 / 隐藏"
                        });
                        let _ = refresh_menu.set_text(if english {
                            "Refresh now"
                        } else {
                            "立即刷新"
                        });
                        let _ = update_menu.set_text(if english {
                            "Check for updates"
                        } else {
                            "检查更新"
                        });
                        let _ = unlock_menu.set_text(if english {
                            "Unlock widget"
                        } else {
                            "解锁悬浮窗"
                        });
                        let _ = pin_menu.set_text(if english {
                            "Pin / Unpin Codex"
                        } else {
                            "固定 / 取消固定 Codex"
                        });
                        let _ = theme_menu.set_text(if english { "Theme" } else { "主题" });
                        let _ = language_menu.set_text(if english {
                            "切换到中文"
                        } else {
                            "Switch to English"
                        });
                        let _ = autostart_menu.set_text(if english {
                            "Start at login"
                        } else {
                            "开机启动"
                        });
                        let _ = quit_menu.set_text(if english { "Quit" } else { "退出" });
                        let _ = app.emit("preferences-changed", normalized);
                    }
                }
            }
            "autostart" => {
                let manager = app.autolaunch();
                let enabled = manager.is_enabled().unwrap_or(false);
                let result = if enabled {
                    manager.disable()
                } else {
                    manager.enable()
                };
                match result {
                    Ok(()) => {
                        let _ = autostart_menu.set_checked(!enabled);
                    }
                    Err(_) => eprintln!("autostart update failed"),
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("widget") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            WindowStateBuilder::default()
                .skip_initial_state("settings")
                .skip_initial_state("statusbar")
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_config_dir()?;
            let preferences_path = data_dir.join("preferences.json");
            let preferences = load_preferences(&preferences_path);
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(12))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent("QuotaFloat/0.1")
                .build()
                .expect("static HTTP client configuration must be valid");
            app.manage(AppState {
                client,
                preferences: Mutex::new(preferences.clone()),
                preferences_path,
                fetch_lock: tokio::sync::Mutex::new(()),
                snapshot_cache: Mutex::new(None),
                #[cfg(debug_assertions)]
                simulate_short_window_for_testing: Mutex::new(false),
                geometry: Mutex::new(None),
                drag_mode: Mutex::new(None),
            });
            if setup_tray(app).is_err() {
                eprintln!("tray setup failed; enabling taskbar fallback");
                if let Some(window) = app.get_webview_window("widget") {
                    let _ = window.set_skip_taskbar(false);
                }
            }
            if preferences.show_status_bar_progress {
                let _ = set_progress_tray_icon(
                    app.handle(),
                    true,
                    100.0,
                    "额度读取中",
                    "healthy",
                );
                set_widget_visibility(app.handle(), false);
            }
            if preferences.locked {
                let _ = apply_lock(app.handle(), true);
            }
            if let Some(window) = app.get_webview_window("widget") {
                let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
                if !preferences.stay_expanded {
                    let scale_factor = window.scale_factor().unwrap_or(1.0);
                    let safe_inset = logical_to_physical(EDGE_SAFE_INSET_LOGICAL, scale_factor);
                    let collapsed_size = PhysicalSize::new(
                        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
                        widget_window_size(COLLAPSED_LOGICAL_SIZE, scale_factor, safe_inset),
                    );
                    let _ = window.set_size(collapsed_size);
                }
                apply_widget_window_region(&window);
                let _ = window.set_always_on_top(preferences.always_on_top);
            }
            if let Some(window) = app.get_webview_window("statusbar") {
                let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
                let _ = window.set_always_on_top(true);
                let _ = set_status_bar_progress_visible(
                    preferences.show_status_bar_progress,
                    None,
                    app.handle().clone(),
                );
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshots,
            refresh_snapshots,
            expand_widget,
            collapse_widget,
            begin_widget_drag,
            finish_widget_drag,
            get_preferences,
            set_preferences,
            set_widget_locked,
            set_widget_always_on_top,
            set_status_bar_progress_visible,
            set_tray_progress,
            set_widget_visible
        ])
        .on_tray_icon_event(|app, event| {
            match event {
                TrayIconEvent::Enter { rect, .. } | TrayIconEvent::Move { rect, .. } => {
                    show_tray_preview(app, rect);
                }
                TrayIconEvent::Leave { .. } => hide_tray_preview(app),
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    rect,
                    ..
                } => {
                    let status_visible = app
                        .try_state::<AppState>()
                        .and_then(|state| state.preferences.lock().ok().map(|prefs| prefs.show_status_bar_progress))
                        .unwrap_or(false);
                    if status_visible {
                        show_tray_preview(app, rect);
                    } else if let Some(window) = app.get_webview_window("widget") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                _ => {}
            }
        })
        .on_window_event(|window, event| {
            if window.label() == "widget" && matches!(event, WindowEvent::Resized(_)) {
                apply_widget_event_window_region(window);
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "statusbar" {
                    api.prevent_close();
                    let _ = window.hide();
                    return;
                }
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build Quota Float");
    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Resumed) {
            let _ = app_handle.emit_to("widget", "refresh-requested", ());
        }
    });
}
