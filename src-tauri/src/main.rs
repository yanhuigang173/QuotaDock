#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod quota;
mod tray_icon;

use chrono::{DateTime, Duration as Cd, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::PathBuf, sync::Mutex, thread, time::Duration};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri::{Emitter, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    recent_fast_breath_ms: u64,
    critical_blink_ms: u64,
    quota_refresh_ms: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            recent_fast_breath_ms: 4000,
            critical_blink_ms: 3000,
            quota_refresh_ms: 180_000,
        }
    }
}
impl Settings {
    fn clamp(mut self) -> Self {
        self.recent_fast_breath_ms = self.recent_fast_breath_ms.clamp(1000, 20_000);
        self.critical_blink_ms = self.critical_blink_ms.clamp(1000, 20_000);
        self.quota_refresh_ms = self.quota_refresh_ms.clamp(60_000, 1_800_000);
        self
    }
}

struct Model {
    quota: Option<Value>,
    history: Vec<Value>,
    status: &'static str,
    error: Option<Value>,
    from_cache: bool,
    refreshing: bool,
    last_updated_at: Option<String>,
    next_refresh_at: Option<DateTime<Utc>>,
    failures: u32,
    visible: bool,
    settings: Settings,
    tray_icon_step: u8,
    dir: PathBuf,
    pinned: bool,
    compact: bool,
    scale: f64,
    expanded: bool,
    strip: bool,
    passthrough: bool,
}
impl Model {
    fn snapshot(&self) -> Value {
        json!({"quota":self.quota,"history":self.history,"status":self.status,"error":self.error,
        "fromCache":self.from_cache,"refreshing":self.refreshing,"lastUpdatedAt":self.last_updated_at,"nextRefreshAt":self.next_refresh_at.as_ref()})
    }
    fn schedule(&mut self, retry: bool) {
        let delay = if retry {
            (60_000_u64 << self.failures.saturating_sub(1).min(3)).min(300_000)
        } else if self.visible {
            self.settings.quota_refresh_ms
        } else {
            1_800_000
        };
        self.next_refresh_at = Some(Utc::now() + Cd::milliseconds(delay as i64));
    }
}
fn win(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or("Main window unavailable".into())
}
fn emit(app: &tauri::AppHandle) {
    let state = app.state::<Mutex<Model>>().lock().unwrap().snapshot();
    let _ = app.emit("quota:changed", state);
}
fn tray_size(app: &tauri::AppHandle) -> u32 {
    let scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map_or(1.0, |monitor| monitor.scale_factor());
    match (16.0 * scale).round() as u32 {
        0..=18 => 16,
        19..=22 => 20,
        23..=28 => 24,
        _ => 32,
    }
}
fn tray_state(app: &tauri::AppHandle) -> (Option<u8>, Option<u8>, bool, String) {
    let state = app.state::<Mutex<Model>>();
    let model = state.lock().unwrap();
    let quota = model.quota.as_ref();
    let short = tray_icon::quantize(quota.and_then(|q| q["primary"]["remainingPercent"].as_f64()), model.tray_icon_step);
    let long = tray_icon::quantize(quota.and_then(|q| q["secondary"]["remainingPercent"].as_f64()), model.tray_icon_step);
    let stale = model.status != "ready";
    let percent_text = |key: &str| {
        quota
            .and_then(|q| q[key]["remainingPercent"].as_f64())
            .map_or_else(|| "--".to_owned(), |p| format!("{:.0}", p.clamp(0.0, 100.0)))
    };
    let tooltip = if quota.is_some() {
        format!(
            "QuotaDock · 5h {}% · 7d {}%{}",
            percent_text("primary"),
            percent_text("secondary"),
            if stale { " · 旧数据" } else { "" }
        )
    } else if model.status == "error" {
        "QuotaDock · 额度读取失败".to_owned()
    } else {
        "QuotaDock · 读取中".to_owned()
    };
    (short, long, stale, tooltip)
}
fn update_tray(app: &tauri::AppHandle) {
    let (short, long, stale, tooltip) = tray_state(app);
    if let Some(tray) = app.tray_by_id("quota-tray") {
        let _ = tray.set_icon(Some(tray_icon::render(tray_size(app), short, long, stale)));
        let _ = tray.set_tooltip(Some(tooltip));
    }
}
fn read(path: PathBuf) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
fn save(path: PathBuf, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("tmp");
    fs::write(
        &temp,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    fs::rename(temp, path).map_err(|e| e.to_string())
}
fn sample(q: &Value) -> Value {
    fn part(w: &Value) -> Value {
        if w.is_null() {
            Value::Null
        } else {
            json!({"remainingPercent":w["remainingPercent"],
    "windowDurationMins":w["windowDurationMins"],"resetsAt":w["resetsAt"]})
        }
    }
    json!({"fetchedAt":q["fetchedAt"],"primary":part(&q["primary"]),"secondary":part(&q["secondary"])})
}
fn refresh_inner(app: &tauri::AppHandle) -> Value {
    {
        let state = app.state::<Mutex<Model>>();
        let mut m = state.lock().unwrap();
        if m.refreshing {
            return m.snapshot();
        }
        m.refreshing = true;
        m.error = None;
        m.next_refresh_at = None;
        if m.quota.is_none() {
            m.status = "loading";
        }
    }
    emit(app);
    let result = quota::read_quota();
    {
        let state = app.state::<Mutex<Model>>();
        let mut m = state.lock().unwrap();
        m.refreshing = false;
        match result {
            Ok(q) => {
                m.history.retain(|v| v["fetchedAt"] != q["fetchedAt"]);
                m.history.push(sample(&q));
                let cutoff = Utc::now() - Cd::days(14);
                m.history.retain(|v| {
                    v["fetchedAt"]
                        .as_str()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .is_some_and(|d| d.with_timezone(&Utc) >= cutoff)
                });
                let excess = m.history.len().saturating_sub(4096);
                m.history.drain(..excess);
                m.last_updated_at = q["fetchedAt"].as_str().map(str::to_owned);
                m.quota = Some(q.clone());
                m.status = "ready";
                m.error = None;
                m.from_cache = false;
                m.failures = 0;
                let _ = save(
                    m.dir.join("quota-cache.json"),
                    &json!({"version":1,"savedAt":Utc::now().to_rfc3339(),"quota":q}),
                );
                let _ = save(
                    m.dir.join("quota-history.json"),
                    &json!({"version":1,"samples":m.history}),
                );
                m.schedule(false);
            }
            Err(e) => {
                m.status = "error";
                m.error = Some(json!({"name":"Error","message":e}));
                m.failures += 1;
                m.schedule(true);
            }
        }
    }
    update_tray(app);
    emit(app);
    app.state::<Mutex<Model>>().lock().unwrap().snapshot()
}

#[tauri::command]
fn get_quota_state(m: State<'_, Mutex<Model>>) -> Value {
    m.lock().unwrap().snapshot()
}
#[tauri::command]
async fn refresh_quota(app: tauri::AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || refresh_inner(&app))
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn hide_window(app: tauri::AppHandle) -> Result<(), String> {
    win(&app)?.hide().map_err(|e| e.to_string())?;
    let state = app.state::<Mutex<Model>>();
    let mut m = state.lock().unwrap();
    m.visible = false;
    m.schedule(false);
    Ok(())
}
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0)
}
#[tauri::command]
fn get_always_on_top(m: State<'_, Mutex<Model>>) -> bool {
    m.lock().unwrap().pinned
}
#[tauri::command]
fn set_always_on_top(app: tauri::AppHandle, value: bool) -> Result<bool, String> {
    win(&app)?
        .set_always_on_top(value)
        .map_err(|e| e.to_string())?;
    app.state::<Mutex<Model>>().lock().unwrap().pinned = value;
    let _ = app.emit("window:alwaysOnTopChanged", value);
    Ok(value)
}
#[tauri::command]
fn get_compact_mode(m: State<'_, Mutex<Model>>) -> bool {
    m.lock().unwrap().compact
}
#[tauri::command]
fn set_compact_mode(app: tauri::AppHandle, value: bool) -> Result<bool, String> {
    {
        let state = app.state::<Mutex<Model>>();
        let mut m = state.lock().unwrap();
        m.compact = value;
        m.expanded = false;
        m.strip = false;
        m.passthrough = false;
    }
    let w = win(&app)?;
    w.set_ignore_cursor_events(false)
        .map_err(|e| e.to_string())?;
    w.set_skip_taskbar(value).map_err(|e| e.to_string())?;
    resize(&app)?;
    place(&app)?;
    let _ = app.emit("window:compactChanged", value);
    let _ = app.emit("window:compactDisplayModeChanged", "hud");
    Ok(value)
}
#[tauri::command]
fn get_compact_scale(m: State<'_, Mutex<Model>>) -> f64 {
    m.lock().unwrap().scale
}
#[tauri::command]
fn set_compact_scale(app: tauri::AppHandle, value: f64) -> Result<f64, String> {
    let scale = if value.is_finite() {
        value.clamp(0.46, 1.25)
    } else {
        1.0
    };
    app.state::<Mutex<Model>>().lock().unwrap().scale = scale;
    resize(&app)?;
    let _ = app.emit("window:compactScaleChanged", scale);
    Ok(scale)
}
#[tauri::command]
fn set_compact_expanded(app: tauri::AppHandle, value: bool) -> Result<bool, String> {
    let expanded = {
        let state = app.state::<Mutex<Model>>();
        let mut m = state.lock().unwrap();
        m.expanded = value && m.compact;
        m.expanded
    };
    resize(&app)?;
    Ok(expanded)
}
#[tauri::command]
fn get_compact_display_mode(m: State<'_, Mutex<Model>>) -> &'static str {
    if m.lock().unwrap().strip {
        "topStrip"
    } else {
        "hud"
    }
}
#[tauri::command]
fn set_compact_mouse_passthrough(app: tauri::AppHandle, value: bool) -> Result<bool, String> {
    let next = {
        let state = app.state::<Mutex<Model>>();
        let mut m = state.lock().unwrap();
        m.passthrough = value && m.compact && m.strip && !m.expanded;
        m.passthrough
    };
    win(&app)?
        .set_ignore_cursor_events(next)
        .map_err(|e| e.to_string())?;
    Ok(next)
}
#[tauri::command]
fn get_cursor_state(app: tauri::AppHandle) -> Result<Value, String> {
    let p = app.cursor_position().map_err(|e| e.to_string())?;
    let w = win(&app)?;
    let pos = w.outer_position().map_err(|e| e.to_string())?;
    let size = w.outer_size().map_err(|e| e.to_string())?;
    Ok(
        json!({"cursor":{"x":p.x,"y":p.y},"windowBounds":{"x":pos.x,"y":pos.y,"width":size.width,"height":size.height}}),
    )
}
#[tauri::command]
fn move_compact_window(app: tauri::AppHandle, delta_x: f64, delta_y: f64) -> Result<Value, String> {
    if !delta_x.is_finite() || !delta_y.is_finite() {
        return Err("Invalid movement".into());
    }
    let w = win(&app)?;
    let p = w.outer_position().map_err(|e| e.to_string())?;
    let x = p.x + delta_x.round() as i32;
    let y = p.y + delta_y.round() as i32;
    w.set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    Ok(json!({"x":x,"y":y,"snapped":false,"displayMode":"hud"}))
}
#[tauri::command]
fn snap_compact_window(app: tauri::AppHandle) -> Result<Value, String> {
    let w = win(&app)?;
    let p = w.outer_position().map_err(|e| e.to_string())?;
    let size = w.outer_size().map_err(|e| e.to_string())?;
    let monitor = app
        .monitor_from_point(
            (p.x + size.width as i32 / 2) as f64,
            (p.y + size.height as i32 / 2) as f64,
        )
        .map_err(|e| e.to_string())?
        .ok_or("No monitor")?;
    let area = monitor.work_area();
    let center = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let strip = p.y <= area.position.y + 4;
    let hud = (p.x - center).abs() <= 48 && (p.y - area.position.y - 6).abs() <= 28;
    let snapped = strip || hud;
    let x = if strip && (p.x - center).abs() > 56 {
        p.x.clamp(
            area.position.x,
            area.position.x + area.size.width as i32 - size.width as i32,
        )
    } else if snapped {
        center
    } else {
        p.x
    };
    let y = if strip {
        area.position.y
    } else if hud {
        area.position.y + 6
    } else {
        p.y
    };
    {
        let state = app.state::<Mutex<Model>>();
        let mut m = state.lock().unwrap();
        m.strip = strip;
        m.expanded = false;
    }
    resize(&app)?;
    if snapped {
        w.set_position(PhysicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
    }
    let mode = if strip { "topStrip" } else { "hud" };
    let _ = app.emit("window:compactDisplayModeChanged", mode);
    Ok(json!({"x":x,"y":y,"snapped":snapped,"displayMode":mode}))
}
#[tauri::command]
fn get_signal_settings(m: State<'_, Mutex<Model>>) -> Settings {
    m.lock().unwrap().settings.clone()
}
#[tauri::command]
fn set_signal_settings(app: tauri::AppHandle, settings: Settings) -> Result<Settings, String> {
    let settings = settings.clamp();
    let (dir, tray_icon_step) = {
        let state = app.state::<Mutex<Model>>();
        let mut m = state.lock().unwrap();
        m.settings = settings.clone();
        m.schedule(false);
        (m.dir.clone(), m.tray_icon_step)
    };
    save(
        dir.join("widget-settings.json"),
        &json!({"version":1,"signal":settings,"trayIconStep":tray_icon_step}),
    )?;
    let _ = app.emit("signal:settingsChanged", &settings);
    Ok(settings)
}
#[tauri::command]
fn reset_signal_settings(app: tauri::AppHandle) -> Result<Settings, String> {
    set_signal_settings(app, Settings::default())
}

fn set_tray_icon_step(app: &tauri::AppHandle, step: u8) {
    let (dir, settings) = {
        let state = app.state::<Mutex<Model>>();
        let mut model = state.lock().unwrap();
        model.tray_icon_step = step;
        (model.dir.clone(), model.settings.clone())
    };
    let _ = save(dir.join("widget-settings.json"), &json!({"version":1,"signal":settings,"trayIconStep":step}));
    update_tray(app);
}

fn resize(app: &tauri::AppHandle) -> Result<(), String> {
    let m = app.state::<Mutex<Model>>();
    let m = m.lock().unwrap();
    let (width, height) = if m.compact {
        (
            500.0 * m.scale,
            (if m.strip {
                if m.expanded {
                    124.0
                } else {
                    28.0
                }
            } else if m.expanded {
                168.0
            } else {
                86.0
            }) * m.scale,
        )
    } else {
        (390.0, 336.0)
    };
    win(app)?
        .set_size(tauri::LogicalSize::new(width.round(), height.round()))
        .map_err(|e| e.to_string())
}
fn place(app: &tauri::AppHandle) -> Result<(), String> {
    let w = win(app)?;
    let monitor = app
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("No monitor")?;
    let area = monitor.work_area();
    let size = w.outer_size().map_err(|e| e.to_string())?;
    let compact = app.state::<Mutex<Model>>().lock().unwrap().compact;
    let x = if compact {
        area.position.x + (area.size.width as i32 - size.width as i32) / 2
    } else {
        area.position.x + area.size.width as i32 - size.width as i32 - 24
    };
    w.set_position(PhysicalPosition::new(
        x,
        area.position.y + if compact { 6 } else { 24 },
    ))
    .map_err(|e| e.to_string())
}
fn show(app: &tauri::AppHandle) {
    if let Ok(w) = win(app) {
        let _ = w.show();
        let _ = w.set_focus();
    }
    let state = app.state::<Mutex<Model>>();
    let mut m = state.lock().unwrap();
    m.visible = true;
    m.schedule(false);
}
fn toggle(app: &tauri::AppHandle) {
    if win(app).is_ok_and(|w| w.is_visible().unwrap_or(false)) {
        let _ = hide_window(app.clone());
    } else {
        show(app);
    }
}
fn settings_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(
        app,
        "settings",
        WebviewUrl::App("settings/index.html".into()),
    )
    .title("灯效设置")
    .inner_size(360.0, 344.0)
    .resizable(false)
    .build();
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle();
            let dir = handle.path().app_data_dir()?;
            let legacy_paths: Vec<PathBuf> = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .into_iter()
                .flat_map(|base| {
                    [
                        base.join("codex-quota-widget"),
                        base.join("Codex Quota Widget"),
                    ]
                })
                .collect();
            let load = |name: &str| {
                read(dir.join(name))
                    .or_else(|| legacy_paths.iter().find_map(|base| read(base.join(name))))
            };
            let cache = load("quota-cache.json");
            let history = load("quota-history.json");
            let saved_settings = load("widget-settings.json");
            let tray_icon_step = saved_settings.as_ref()
                .and_then(|v| v["trayIconStep"].as_u64())
                .filter(|step| [1, 3, 5].contains(step))
                .unwrap_or(3) as u8;
            let settings = saved_settings
                .and_then(|v| serde_json::from_value::<Settings>(v["signal"].clone()).ok())
                .unwrap_or_default()
                .clamp();
            let quota = cache
                .as_ref()
                .filter(|c| c["version"] == 1)
                .map(|c| c["quota"].clone())
                .filter(|q| !q.is_null());
            let status = if quota.is_some() { "ready" } else { "loading" };
            let updated = cache.and_then(|c| c["savedAt"].as_str().map(str::to_owned));
            app.manage(Mutex::new(Model {
                from_cache: quota.is_some(),
                quota,
                history: history
                    .filter(|h| h["version"] == 1)
                    .and_then(|h| h["samples"].as_array().cloned())
                    .unwrap_or_default(),
                status,
                error: None,
                refreshing: false,
                last_updated_at: updated,
                next_refresh_at: None,
                failures: 0,
                visible: true,
                settings,
                tray_icon_step,
                dir,
                pinned: true,
                compact: true,
                scale: 0.46,
                expanded: false,
                strip: false,
                passthrough: false,
            }));
            let _ = place(handle);
            let show_item = MenuItem::with_id(app, "show", "显示/隐藏", true, None::<&str>)?;
            let refresh_item = MenuItem::with_id(app, "refresh", "刷新额度", true, None::<&str>)?;
            let settings_item =
                MenuItem::with_id(app, "settings", "灯效设置...", true, None::<&str>)?;
            let icon_step_one = MenuItem::with_id(app, "icon-step-1", "托盘图档位 1%", true, None::<&str>)?;
            let icon_step_three = MenuItem::with_id(app, "icon-step-3", "托盘图档位 3%（默认）", true, None::<&str>)?;
            let icon_step_five = MenuItem::with_id(app, "icon-step-5", "托盘图档位 5%", true, None::<&str>)?;
            let compact_item =
                MenuItem::with_id(app, "compact", "切换紧凑窗口", true, None::<&str>)?;
            let pin_item = MenuItem::with_id(app, "pin", "切换置顶", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show_item,
                    &refresh_item,
                    &settings_item,
                    &icon_step_one,
                    &icon_step_three,
                    &icon_step_five,
                    &compact_item,
                    &pin_item,
                    &quit_item,
                ],
            )?;
            let (short, long, stale, tooltip) = tray_state(handle);
            TrayIconBuilder::with_id("quota-tray")
                .icon(tray_icon::render(tray_size(handle), short, long, stale))
                .tooltip(tooltip)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => toggle(app),
                    "refresh" => {
                        let a = app.clone();
                        thread::spawn(move || {
                            refresh_inner(&a);
                        });
                    }
                    "settings" => settings_window(app),
                    "icon-step-1" => set_tray_icon_step(app, 1),
                    "icon-step-3" => set_tray_icon_step(app, 3),
                    "icon-step-5" => set_tray_icon_step(app, 5),
                    "compact" => {
                        let v = app.state::<Mutex<Model>>().lock().unwrap().compact;
                        let _ = set_compact_mode(app.clone(), !v);
                    }
                    "pin" => {
                        let v = app.state::<Mutex<Model>>().lock().unwrap().pinned;
                        let _ = set_always_on_top(app.clone(), !v);
                    }
                    "quit" => app.exit(0),
                    _ => (),
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        toggle(tray.app_handle());
                    }
                })
                .build(app)?;
            let a = handle.clone();
            thread::spawn(move || loop {
                thread::sleep(Duration::from_secs(1));
                let due = {
                    let state = a.state::<Mutex<Model>>();
                    let m = state.lock().unwrap();
                    !m.refreshing
                        && m.next_refresh_at
                            .as_ref()
                            .is_some_and(|at| at <= &Utc::now())
                };
                if due {
                    refresh_inner(&a);
                }
            });
            let a = handle.clone();
            thread::spawn(move || {
                refresh_inner(&a);
            });
            app.get_webview_window("main")
                .ok_or_else(|| std::io::Error::other("Main window unavailable"))?
                .show()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_quota_state,
            refresh_quota,
            hide_window,
            quit_app,
            get_always_on_top,
            set_always_on_top,
            get_compact_mode,
            set_compact_mode,
            get_compact_scale,
            set_compact_scale,
            set_compact_expanded,
            get_compact_display_mode,
            set_compact_mouse_passthrough,
            get_cursor_state,
            move_compact_window,
            snap_compact_window,
            get_signal_settings,
            set_signal_settings,
            reset_signal_settings
        ])
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("application error");
}
