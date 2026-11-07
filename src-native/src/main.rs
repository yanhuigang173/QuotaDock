#![windows_subsystem = "windows"]

mod docking;
mod draw;
mod motion;
mod quota;
mod reset_alert;
mod session_watch;
mod task_notice;
mod theme;
mod tray_icon;

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ffi::c_void,
    fs,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    result::Result::Ok,
    sync::{
        atomic::{AtomicBool, AtomicIsize, Ordering},
        Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, HWND, POINT, RECT, SIZE},
    Graphics::{Dwm::DwmFlush, Gdi::*, GdiPlus::*},
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{ReleaseCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const TRAY_MESSAGE: u32 = WM_APP + 1;
const REFRESH_MESSAGE: u32 = WM_APP + 2;
const NOTICE_DONE: u32 = WM_APP + 3;
const ANIMATION_FRAME: u32 = WM_APP + 4;
const TASK_MESSAGE: u32 = WM_APP + 5;
const TASK_LIGHT_TIMER: usize = 4;
const CMD_TASK_ENABLED: usize = 320;
const CMD_TASK_PREVIEW: usize = 321;
const CMD_TASK_INSTALL: usize = 322;
const CMD_TASK_REVIEW: usize = 323;
const CMD_PERMISSION_ENABLED: usize = 324;
const CMD_TASK_STATUS: usize = 325;
const CMD_TASK_STATUS_BAR: usize = 326;
const CMD_TASK_STATUS_DOT: usize = 327;
const CMD_TASK_STATUS_RIBBON: usize = 328;
const CMD_TASK_SOURCE_LOCAL: usize = 330;
const CMD_TASK_SOURCE_HOOK: usize = 331;
const CMD_TASK_SOURCE_BOTH: usize = 332;
const CMD_TASK_CHARS_BASE: usize = 340;
const CMD_TASK_LINES_BASE: usize = 350;
const CMD_RIBBON_STRENGTH_BASE: usize = 360;
const CMD_COLOR_THEME_BASE: usize = 370;
const MOUSE_LEAVE: u32 = 675;
const CMD_TOGGLE: usize = 100;
const CMD_REFRESH: usize = 101;
const CMD_COMPACT: usize = 102;
const CMD_STRIP: usize = 103;
const CMD_FULL: usize = 106;
const CMD_FUSION: usize = 108;
const CMD_TOGGLE_FULL: usize = 110;
const CMD_PIN: usize = 104;
const CMD_THEME: usize = 105;
const CMD_AUTOSTART: usize = 107;
const CMD_ALERT_MASTER: usize = 301;
const CMD_ALERT_7D_TIME: usize = 302;
const CMD_ALERT_7D_BONUS: usize = 303;
const CMD_ALERT_5H_TIME: usize = 304;
const CMD_ALERT_PREVIEW: usize = 305;
const CMD_STYLE_POPUP: usize = 306;
const CMD_STYLE_ISLAND: usize = 307;
const CMD_NOTICE_DISMISS: usize = 308;
const CMD_STEP_1: usize = 111;
const CMD_STEP_3: usize = 113;
const CMD_STEP_5: usize = 115;
const CMD_QUIT: usize = 199;
const CMD_INTERVAL_1: usize = 201;
const CMD_INTERVAL_3: usize = 203;
const CMD_INTERVAL_5: usize = 205;
const CMD_INTERVAL_10: usize = 210;
const CMD_INTERVAL_30: usize = 230;

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static QUERY_RESULT: Mutex<Option<Result<Value, String>>> = Mutex::new(None);
static FRAME_QUEUED: AtomicBool = AtomicBool::new(false);
static ANIMATION_CLOCK: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

fn set_animation_running(running: bool) {
    *ANIMATION_CLOCK.0.lock().unwrap() = running;
    ANIMATION_CLOCK.1.notify_one();
}

fn start_animation_clock() {
    thread::spawn(|| loop {
        let mut active = ANIMATION_CLOCK.0.lock().unwrap();
        while !*active && MAIN_HWND.load(Ordering::Acquire) != 0 {
            active = ANIMATION_CLOCK.1.wait(active).unwrap();
        }
        if MAIN_HWND.load(Ordering::Acquire) == 0 {
            break;
        }
        drop(active);
        let start = Instant::now();
        unsafe {
            DwmFlush();
        }
        // Occluded windows can make DwmFlush return immediately; cap that path.
        if start.elapsed() < Duration::from_millis(8) {
            thread::sleep(Duration::from_millis(8) - start.elapsed().min(Duration::from_millis(8)));
        }
        if *ANIMATION_CLOCK.0.lock().unwrap() && !FRAME_QUEUED.swap(true, Ordering::AcqRel) {
            unsafe {
                PostMessageW(
                    MAIN_HWND.load(Ordering::Acquire) as HWND,
                    ANIMATION_FRAME,
                    0,
                    0,
                );
            }
        }
    });
}

struct LayerSurface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut c_void,
    width: i32,
    height: i32,
}

impl LayerSurface {
    unsafe fn new(width: i32, height: i32) -> Option<Self> {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, bitmap);
        Some(Self {
            dc,
            bitmap,
            old,
            bits,
            width,
            height,
        })
    }
}

impl Drop for LayerSurface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
fn window_class() -> String {
    // Integration fixtures must never attach to the user's running receiver.
    std::env::var("QUOTADOCK_TEST_INSTANCE")
        .ok()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .map(|id| format!("QuotaDockNativeWindow-{id}"))
        .unwrap_or_else(|| "QuotaDockNativeWindow".into())
}
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_NAME: &str = "QuotaDock";

fn autostart_command() -> Option<String> {
    Some(format!("\"{}\"", std::env::current_exe().ok()?.display()))
}

fn autostart_enabled() -> bool {
    let Some(expected) = autostart_command() else {
        return false;
    };
    unsafe {
        let mut key = null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        ) != 0
        {
            return false;
        }
        let name = wide(RUN_NAME);
        let mut value_type = 0;
        let mut bytes = 0;
        let status = RegQueryValueExW(
            key,
            name.as_ptr(),
            null(),
            &mut value_type,
            null_mut(),
            &mut bytes,
        );
        if status != 0 || value_type != REG_SZ || bytes == 0 || bytes % 2 != 0 {
            RegCloseKey(key);
            return false;
        }
        let mut data = vec![0u16; bytes as usize / 2];
        let status = RegQueryValueExW(
            key,
            name.as_ptr(),
            null(),
            &mut value_type,
            data.as_mut_ptr() as *mut u8,
            &mut bytes,
        );
        RegCloseKey(key);
        if status != 0 {
            return false;
        }
        let len = data.iter().position(|&c| c == 0).unwrap_or(data.len());
        String::from_utf16_lossy(&data[..len]).eq_ignore_ascii_case(&expected)
    }
}

fn set_autostart(enabled: bool) -> Result<(), u32> {
    let command = if enabled {
        Some(wide(&autostart_command().ok_or(ERROR_FILE_NOT_FOUND)?))
    } else {
        None
    };
    unsafe {
        let mut key = null_mut();
        let key_name = wide(RUN_KEY);
        let status = if enabled {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key_name.as_ptr(),
                0,
                null(),
                0,
                KEY_SET_VALUE,
                null(),
                &mut key,
                null_mut(),
            )
        } else {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                key_name.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if !enabled && status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if status != 0 {
            return Err(status);
        }
        let name = wide(RUN_NAME);
        let status = if enabled {
            let value = command.as_ref().unwrap();
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as *const u8,
                (value.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, name.as_ptr())
        };
        RegCloseKey(key);
        if status == 0 || (!enabled && status == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(status)
        }
    }
}
fn load_json(path: PathBuf) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
fn save_json(path: PathBuf, value: &Value) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(value) {
        let temp = path.with_extension("tmp");
        if fs::write(&temp, bytes).is_ok() {
            let _ = fs::rename(&temp, &path).or_else(|_| {
                fs::remove_file(&path)?;
                fs::rename(temp, path)
            });
        }
    }
}
fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("cn.codex.quota.widget")
}
fn load_compatible(dir: &std::path::Path, file: &str) -> Option<Value> {
    load_json(dir.join(file)).or_else(|| {
        let root = std::env::var_os("APPDATA").map(PathBuf::from)?;
        ["codex-quota-widget", "Codex Quota Widget"]
            .iter()
            .find_map(|name| load_json(root.join(name).join(file)))
    })
}

#[cfg(test)]
mod settings_regression_tests {
    use super::*;
    #[test]
    fn window_and_visual_preferences_survive_restart() {
        let suffix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(format!("settings-regression-{suffix}"));
        for file in ["widget-settings.json", "quota-cache.json", "quota-history.json"] {
            save_json(dir.join(file), &json!({"version":1}));
        }
        let mut app = App::load_from(dir.clone());
        assert!(app.pinned);
        assert!(!app.island);
        assert!(app.color_theme == theme::Theme::Default);
        for theme in theme::Theme::ALL {
            for strength in draw::RibbonStrength::ALL {
                app.color_theme = theme;
                app.task_ribbon_strength = strength;
                app.pinned = false;
                app.island = true;
                app.task_ribbon_enabled = true;
                app.task_status_enabled = false;
                app.save_settings();
                let loaded = App::load_from(dir.clone());
                assert!(!loaded.pinned);
                assert!(loaded.island);
                assert!(loaded.color_theme == theme);
                assert!(loaded.task_ribbon_strength == strength);
                assert!(loaded.task_ribbon_enabled);
                assert!(!loaded.task_status_enabled);
            }
        }
        for file in ["widget-settings.json", "quota-cache.json", "quota-history.json"] {
            fs::remove_file(dir.join(file)).unwrap();
        }
        fs::remove_dir(dir).unwrap();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignalSettings {
    recent_fast_breath_ms: u64,
    critical_blink_ms: u64,
    quota_refresh_ms: u64,
}
impl Default for SignalSettings {
    fn default() -> Self {
        Self {
            recent_fast_breath_ms: 4000,
            critical_blink_ms: 3000,
            quota_refresh_ms: 180_000,
        }
    }
}
#[derive(Clone, Copy)]
struct CompactView {
    fusion: bool,
    strip: bool,
    dock: docking::Edge,
    origin: POINT,
}

struct App {
    quota: Option<Value>,
    history: Vec<Value>,
    stale: bool,
    error: Option<String>,
    refreshing: bool,
    next_refresh: Instant,
    failures: u32,
    settings: SignalSettings,
    reset_alerts: reset_alert::Settings,
    task_notifications: bool,
    task_source: task_notice::Source,
    task_text: task_notice::TextSettings,
    task_layout: RefCell<Option<(String, u8, u32, u8)>>,
    task_status_enabled: bool,
    task_light_style: draw::StatusLightStyle,
    task_ribbon_enabled: bool,
    task_ribbon_strength: draw::RibbonStrength,
    task_state: session_watch::TaskState,
    task_pulse_started: Instant,
    task_pulse_timer: Cell<bool>,
    permission_prompts: bool,
    task_inbox: task_notice::Inbox,
    pending_notices: VecDeque<reset_alert::Notice>,
    notice_active: bool,
    island_notice: Option<reset_alert::Notice>,
    notice_until: Option<Instant>,
    notice_origin: Option<POINT>,
    notice_width: Option<i32>,
    restore_after_transition: bool,
    transition: Option<motion::Transition>,
    last_frame: RefCell<Option<motion::Frame>>,
    last_shell: Cell<motion::Shell>,
    surface: RefCell<Option<LayerSurface>>,
    icon_step: u8,
    icon: HICON,
    tray_added: bool,
    visible: bool,
    compact: bool,
    strip: bool,
    expanded: bool,
    hover_outside_since: Option<Instant>,
    menu_open: bool,
    fusion: bool,
    dock: docking::Edge,
    return_view: Option<CompactView>,
    dragging: bool,
    side_hover_anchor: Option<(f32, f32)>,
    island: bool,
    color_theme: theme::Theme,
    pinned: bool,
    scale: f32,
    dpi: f32,
    dir: PathBuf,
}
impl App {
    fn load() -> Self {
        Self::load_from(data_dir())
    }
    fn load_from(dir: PathBuf) -> Self {
        let cache = load_compatible(&dir, "quota-cache.json");
        let quota = cache
            .filter(|v| v["version"] == 1)
            .and_then(|v| v.get("quota").cloned())
            .filter(|v| !v.is_null());
        let history = load_compatible(&dir, "quota-history.json")
            .filter(|v| v["version"] == 1)
            .and_then(|v| v["samples"].as_array().cloned())
            .unwrap_or_default();
        let saved = load_compatible(&dir, "widget-settings.json");
        let settings = saved
            .as_ref()
            .and_then(|v| serde_json::from_value(v["signal"].clone()).ok())
            .unwrap_or_default();
        let reset_alerts = saved
            .as_ref()
            .and_then(|v| serde_json::from_value(v["resetAlerts"].clone()).ok())
            .unwrap_or_default();
        let icon_step = saved
            .as_ref()
            .and_then(|v| v["trayIconStep"].as_u64())
            .filter(|v| [1, 3, 5].contains(v))
            .unwrap_or(3) as u8;
        let scale = saved
            .as_ref()
            .and_then(|v| v["compactScale"].as_f64())
            .filter(|scale| {
                scale.is_finite()
                    && (saved
                        .as_ref()
                        .is_some_and(|v| v["compactScaleVersion"] == 2)
                        || (*scale - 0.72).abs() > 0.0001)
            })
            .unwrap_or(0.46)
            .clamp(0.46, 1.25) as f32;
        let fusion = saved.as_ref().is_some_and(|v| v["fusionMode"] == true);
        let dock = saved
            .as_ref()
            .and_then(|v| serde_json::from_value(v["dockEdge"].clone()).ok())
            .unwrap_or_default();
        let strip = if fusion {
            dock != docking::Edge::None
        } else {
            saved.as_ref().is_some_and(|v| v["compactStrip"] == true)
        };
        let task_notifications = !saved
            .as_ref()
            .is_some_and(|v| v["codexTaskNotifications"] == false);
        let permission_prompts = !saved
            .as_ref()
            .is_some_and(|v| v["codexPermissionPrompts"] == false);
        let task_source = saved.as_ref().and_then(|v| serde_json::from_value(v["codexTaskSource"].clone()).ok()).unwrap_or_default();
        let task_inbox = task_notice::Inbox::load(&dir);
        let task_text = saved.as_ref()
            .and_then(|v| serde_json::from_value::<task_notice::TextSettings>(v["codexTaskText"].clone()).ok())
            .unwrap_or_default().normalized();
        Self {
            stale: quota.is_some(),
            quota,
            history,
            error: None,
            refreshing: false,
            next_refresh: Instant::now(),
            failures: 0,
            settings,
            reset_alerts,
            task_notifications,
            task_source,
            task_text,
            task_layout: RefCell::new(None),
            task_status_enabled: !saved.as_ref().is_some_and(|v| v["codexTaskStatusLight"] == false),
            task_light_style: saved.as_ref().and_then(|v| serde_json::from_value(v["codexTaskStatusStyle"].clone()).ok()).unwrap_or_default(),
            task_ribbon_strength: saved.as_ref().and_then(|v| serde_json::from_value(v["codexTaskRibbonStrength"].clone()).ok()).unwrap_or_default(),
            task_ribbon_enabled: saved.as_ref().is_some_and(|v| v["codexTaskRibbon"].as_bool()
                .unwrap_or(v["codexTaskStatusStyle"] == "ribbon" && v["codexTaskStatusLight"] != false)),
            task_state: session_watch::TaskState::Idle,
            task_pulse_started: Instant::now(),
            task_pulse_timer: Cell::new(false),
            permission_prompts,
            task_inbox,
            pending_notices: VecDeque::new(),
            notice_active: false,
            island_notice: None,
            notice_until: None,
            notice_origin: None,
            notice_width: None,
            restore_after_transition: false,
            transition: None,
            last_frame: RefCell::new(None),
            last_shell: Cell::new(motion::Shell {
                color: 0xff080a0e,
                top_radius: 0.0,
                bottom_radius: 0.0,
                insets: [0.0; 4],
                background_inset: [0.0; 4],
            }),
            surface: RefCell::new(None),
            icon_step,
            icon: null_mut(),
            tray_added: false,
            visible: true,
            compact: true,
            strip,
            expanded: false,
            hover_outside_since: None,
            menu_open: false,
            fusion,
            dock,
            return_view: None,
            dragging: false,
            side_hover_anchor: None,
            island: saved.as_ref().is_some_and(|v| v["hudDarkBackground"] == true),
            color_theme: saved.as_ref().and_then(|v| serde_json::from_value(v["colorTheme"].clone()).ok()).unwrap_or_default(),
            pinned: !saved.as_ref().is_some_and(|v| v["alwaysOnTop"] == false),
            scale,
            dpi: 1.0,
            dir,
        }
    }
    fn save_settings(&self) {
        save_json(
            self.dir.join("widget-settings.json"),
            &json!({"version":1,"alwaysOnTop":self.pinned,"hudDarkBackground":self.island,"colorTheme":self.color_theme,"signal":self.settings,"resetAlerts":self.reset_alerts,"trayIconStep":self.icon_step,"compactScale":self.scale,"compactScaleVersion":2,"fusionMode":self.fusion,"compactStrip":self.strip,"dockEdge":self.dock,"codexTaskNotifications":self.task_notifications,"codexTaskSource":self.task_source,"codexTaskText":self.task_text,"codexTaskStatusLight":self.task_status_enabled,"codexTaskStatusStyle":self.task_light_style,"codexTaskRibbon":self.task_ribbon_enabled,"codexTaskRibbonStrength":self.task_ribbon_strength,"codexPermissionPrompts":self.permission_prompts}),
        );
    }
    fn value(&self, key: &str) -> Option<f64> {
        self.quota
            .as_ref()?
            .get(key)?
            .get("remainingPercent")?
            .as_f64()
    }
    fn ideal(&self, key: &str) -> Option<f64> {
        let w = self.quota.as_ref()?.get(key)?;
        let minutes = w.get("windowDurationMins")?.as_f64()?;
        let reset = DateTime::parse_from_rfc3339(w.get("resetsAt")?.as_str()?)
            .ok()?
            .with_timezone(&Utc);
        Some(
            ((reset - Utc::now()).num_seconds().max(0) as f64 / (minutes * 60.0) * 100.0)
                .clamp(0.0, 100.0),
        )
    }
    fn reset(&self, key: &str) -> String {
        self.quota
            .as_ref()
            .and_then(|q| q.get(key))
            .and_then(|w| w.get("resetsAt"))
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Local).format("%m/%d %H:%M").to_string())
            .unwrap_or_else(|| "--".into())
    }
    fn time_left(&self, key: &str) -> String {
        let Some(reset) = self
            .quota
            .as_ref()
            .and_then(|q| q.get(key))
            .and_then(|w| w.get("resetsAt"))
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        else {
            return "--".into();
        };
        let minutes = (reset.with_timezone(&Utc) - Utc::now())
            .num_minutes()
            .max(0);
        format!("{}h {}m", minutes / 60, minutes % 60)
    }
    fn plan(&self) -> String {
        self.quota
            .as_ref()
            .and_then(|q| q.get("planType"))
            .and_then(Value::as_str)
            .map(|s| s.to_uppercase())
            .unwrap_or_else(|| "--".into())
    }
    fn velocity_required(&self, key: &str) -> Option<f64> {
        let current = self.value(key)?;
        let ideal = self.ideal(key)?;
        let window = self.quota.as_ref()?.get(key)?;
        let reset = DateTime::parse_from_rfc3339(window.get("resetsAt")?.as_str()?)
            .ok()?
            .with_timezone(&Utc);
        let now = Utc::now();
        let lookback = if key == "primary" { 120.0 } else { 24.0 * 60.0 };
        let sample = self
            .history
            .iter()
            .rev()
            .filter_map(|entry| {
                let at = DateTime::parse_from_rfc3339(entry.get("fetchedAt")?.as_str()?)
                    .ok()?
                    .with_timezone(&Utc);
                let elapsed = (now - at).num_seconds() as f64 / 60.0;
                let previous = entry.get(key)?.get("remainingPercent")?.as_f64()?;
                (elapsed >= 10.0 && elapsed <= lookback && previous >= current)
                    .then_some((elapsed, previous))
            })
            .last();
        let Some((elapsed, previous)) = sample else {
            return Some(ideal);
        };
        let burn_rate = (previous - current) / elapsed;
        let remaining_minutes = (reset - now).num_seconds().max(0) as f64 / 60.0;
        let projected = (burn_rate * remaining_minutes).clamp(0.0, 100.0);
        let confidence = (elapsed / if key == "primary" { 30.0 } else { 720.0 }).clamp(0.0, 1.0);
        let weight = confidence * if key == "primary" { 0.7 } else { 0.45 };
        Some((ideal * (1.0 - weight) + projected * weight).clamp(0.0, 100.0))
    }
    fn status(&self) -> String {
        if let Some(e) = &self.error {
            return format!("读取失败 · {e}");
        }
        if self.quota.is_none() {
            return "正在读取 Codex 额度...".into();
        }
        if self.stale {
            return "显示上次读取的额度".into();
        }
        let long = self.value("secondary").unwrap_or(100.0);
        let ideal = self.ideal("secondary").unwrap_or(long);
        if long <= 5.0 {
            "接近耗尽".into()
        } else if long - ideal <= -15.0 {
            "建议减速".into()
        } else if long + 3.0 < self.velocity_required("secondary").unwrap_or(ideal) {
            "近期偏快".into()
        } else if long - ideal >= 15.0 {
            "余量充足".into()
        } else {
            "节奏正常".into()
        }
    }
    fn task_notice_lines(&self) -> u8 {
        let Some(notice) = self.island_notice.as_ref().filter(|notice| notice.task) else { return 1; };
        let body = self.task_text.summary(&notice.body);
        let scale = self.render_scale();
        let limit = self.task_text.max_lines;
        if let Some((text, max_lines, scale_bits, rows)) = self.task_layout.borrow().as_ref() {
            if text == &body && *max_lines == limit && *scale_bits == scale.to_bits() { return *rows; }
        }
        let rows = unsafe { draw::task_notice_lines(&body, limit, scale) };
        *self.task_layout.borrow_mut() = Some((body, limit, scale.to_bits(), rows));
        rows
    }
    fn display(&self) -> draw::Display {
        let notice = self.island_notice.as_ref().map(|notice| {
            let progress = self
                .notice_until
                .map(|until| {
                    until
                        .saturating_duration_since(Instant::now())
                        .as_secs_f32()
                        / 10.0
                })
                .unwrap_or(0.0);
            (
                notice.title.clone(),
                if notice.task { self.task_text.summary(&notice.body) } else { notice.body.clone() },
                notice.important,
                progress,
                notice.task,
            )
        });
        draw::Display {
            theme: self.color_theme,
            fusion: self.fusion,
            vertical: self.fusion && self.dock.vertical(),
            compact: self.compact,
            strip: self.strip,
            expanded: self.expanded,
            island: self.island,
            pinned: self.pinned,
            short: self.value("primary"),
            long: self.value("secondary"),
            short_ideal: self.ideal("primary"),
            long_ideal: self.ideal("secondary"),
            short_velocity: None,
            long_velocity: None,
            short_reset: self.reset("primary"),
            long_reset: self.reset("secondary"),
            short_left: self.time_left("primary"),
            long_left: self.time_left("secondary"),
            plan: self.plan(),
            status: self.status(),
            notice,
            notice_lines: self.task_notice_lines(),
            task_state_color: self.task_status_enabled.then(|| {
                let color = self.task_state.color();
                if self.task_state == session_watch::TaskState::Running && self.task_light_style == draw::StatusLightStyle::Bar {
                    let phase = self.task_pulse_started.elapsed().as_secs_f32() * std::f32::consts::TAU / 2.8;
                    let brightness = 0.45 + 0.55 * (0.5 + 0.5 * phase.cos());
                    (color & 0x00ffffff) | (((255.0 * brightness).round() as u32) << 24)
                } else { color }
            }),
            task_light_style: self.task_light_style,
            task_light_rotation: (self.task_state == session_watch::TaskState::Running && self.task_light_style == draw::StatusLightStyle::Dot)
                .then(|| (self.task_pulse_started.elapsed().as_secs_f32() * 360.0 / 1.8).rem_euclid(360.0)),
            task_ribbon_color: self.task_ribbon_enabled.then(|| self.task_state.color()),
            task_ribbon_strength: self.task_ribbon_strength,
            ribbon_attached: if self.fusion && self.compact { match self.dock {
                docking::Edge::Left => Some(0), docking::Edge::Top => Some(1),
                docking::Edge::Right => Some(2), docking::Edge::Bottom | docking::Edge::ScreenBottom => Some(3),
                _ => None,
            } } else { None },
            task_ribbon_rotation: (self.task_state == session_watch::TaskState::Running && self.task_ribbon_enabled)
                .then(|| (self.task_pulse_started.elapsed().as_secs_f32() * 360.0 / 4.4).rem_euclid(360.0)),
            task_light_active: self.task_state != session_watch::TaskState::Idle,
            content_only: true,
        }
    }
    fn target_shell(&self) -> motion::Shell {
        let scale = self.render_scale();
        let (color, top, bottom, insets) =
            if self.island_notice.as_ref().is_some_and(|notice| notice.task) {
                (0xff101d2e, 20.0, 20.0, [0.0; 4])
            } else if self.island_notice.is_some() || (self.compact && (self.strip || self.expanded)) {
                (
                    0xff080a0e,
                    if self.fusion && self.dock == docking::Edge::Top {
                        0.0
                    } else if self.fusion {
                        12.0
                    } else if self.strip || self.island_notice.is_some() {
                        0.0
                    } else {
                        20.0
                    },
                    if self.fusion && matches!(self.dock, docking::Edge::Bottom | docking::Edge::ScreenBottom) { 0.0 }
                    else if self.fusion { 12.0 } else { 20.0 },
                    [0.0; 4],
                )
            } else if self.compact {
                (
                    if self.island { 0xf5020203 } else { 0xdf242a35 },
                    35.0,
                    35.0,
                    [12.0, 8.0, 12.0, 8.0],
                )
            } else {
                (0xff25313a, 22.0, 22.0, [0.0; 4])
            };
        motion::Shell {
            color: self.color_theme.color(color),
            top_radius: top * scale,
            bottom_radius: bottom * scale,
            insets: insets.map(|value| value * scale),
            background_inset: if self.compact && !self.expanded && self.island_notice.is_none()
                && self.task_ribbon_enabled
                && self.task_state != session_watch::TaskState::Idle {
                let mut padding = [self.task_ribbon_strength.body_inset(scale); 4];
                if self.fusion { match self.dock {
                    docking::Edge::Left => padding[0] = 0.0,
                    docking::Edge::Top => padding[1] = 0.0,
                    docking::Edge::Right => padding[2] = 0.0,
                    docking::Edge::Bottom | docking::Edge::ScreenBottom => padding[3] = 0.0,
                    _ => {},
                } }
                padding
            } else { [0.0; 4] },
        }
    }
    fn width(&self) -> i32 {
        self.transition
            .as_ref()
            .map(|t| t.width())
            .unwrap_or_else(|| self.target_width())
    }
    fn target_width(&self) -> i32 {
        self.notice_width.unwrap_or_else(|| {
            ((if self.compact {
                if self.fusion && self.dock.vertical() && !self.expanded {
                    28.0
                } else {
                    500.0
                }
            } else {
                350.0
            }) * self.render_scale())
            .round() as i32
        })
    }
    fn height(&self) -> i32 {
        self.transition
            .as_ref()
            .map(|transition| transition.height())
            .unwrap_or_else(|| self.target_height())
    }
    fn target_height(&self) -> i32 {
        let height = if let Some(notice) = &self.island_notice {
            if notice.task { draw::task_notice_height(self.task_notice_lines()) } else { 146.0 }
        } else if self.compact {
            if self.expanded {
                176.0
            } else if self.fusion && self.dock.vertical() {
                500.0
            } else if self.strip {
                28.0
            } else {
                86.0
            }
        } else {
            218.0
        };
        (height * self.render_scale()).round() as i32
    }
    fn render_scale(&self) -> f32 {
        if let Some(width) = self.notice_width {
            width as f32 / 500.0
        } else {
            (if self.compact { self.scale } else { 1.0 }) * self.dpi
        }
    }
    fn schedule(&mut self) {
        let mut delay = if self.stale && self.error.is_some() {
            (60_000_u64 << self.failures.saturating_sub(1).min(3)).min(300_000)
        } else if self.visible {
            self.settings.quota_refresh_ms
        } else {
            1_800_000
        };
        if self.reset_alerts.enabled {
            delay = delay.min(self.settings.quota_refresh_ms);
            for (key, enabled) in [
                ("secondary", self.reset_alerts.weekly_timed),
                ("primary", self.reset_alerts.hourly_timed),
            ] {
                if !enabled {
                    continue;
                }
                let reset = self
                    .quota
                    .as_ref()
                    .and_then(|quota| quota.get(key))
                    .and_then(|window| window.get("resetsAt"))
                    .and_then(Value::as_str)
                    .and_then(|text| DateTime::parse_from_rfc3339(text).ok());
                if let Some(reset) = reset {
                    let until = (reset.with_timezone(&Utc) - Utc::now()).num_milliseconds();
                    let near_reset = if until > 0 {
                        (until as u64).saturating_add(5_000)
                    } else {
                        60_000
                    };
                    delay = delay.min(near_reset.max(15_000));
                }
            }
        }
        self.next_refresh = Instant::now() + Duration::from_millis(delay);
    }
}
thread_local! { static APP: RefCell<App> = RefCell::new(App::load()); }

fn request_refresh() {
    let start = APP.with(|cell| {
        let mut app = cell.borrow_mut();
        if app.refreshing {
            false
        } else {
            app.refreshing = true;
            true
        }
    });
    if !start {
        return;
    }
    thread::spawn(|| {
        let result = quota::read_quota();
        *QUERY_RESULT.lock().unwrap() = Some(result);
        let hwnd = MAIN_HWND.load(Ordering::SeqCst) as HWND;
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(hwnd, REFRESH_MESSAGE, 0, 0);
            }
        }
    });
}
unsafe fn tray_data(hwnd: HWND, icon: HICON, tooltip: &str) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = icon;
    let utf16 = wide(tooltip);
    let count = utf16.len().min(data.szTip.len());
    data.szTip[..count].copy_from_slice(&utf16[..count]);
    data
}
unsafe fn create_tray_icon(short: Option<u8>, long: Option<u8>, stale: bool) -> HICON {
    let size = 32;
    let rgba = tray_icon::render(size, short, long, stale);
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = size as i32;
    info.bmiHeader.biHeight = -(size as i32);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut pixels: *mut c_void = null_mut();
    let color = CreateDIBSection(
        null_mut(),
        &info,
        DIB_RGB_COLORS,
        &mut pixels,
        null_mut(),
        0,
    );
    if color.is_null() || pixels.is_null() {
        return null_mut();
    }
    let dst = std::slice::from_raw_parts_mut(pixels as *mut u8, rgba.len());
    for (source, target) in rgba.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let a = u16::from(source[3]);
        target[0] = (u16::from(source[2]) * a / 255) as u8;
        target[1] = (u16::from(source[1]) * a / 255) as u8;
        target[2] = (u16::from(source[0]) * a / 255) as u8;
        target[3] = source[3];
    }
    let mask_bits = [0u8; 128];
    let mask = CreateBitmap(
        size as i32,
        size as i32,
        1,
        1,
        mask_bits.as_ptr() as *const c_void,
    );
    let info = ICONINFO {
        fIcon: 1,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    let icon = CreateIconIndirect(&info);
    DeleteObject(mask);
    DeleteObject(color);
    icon
}
unsafe fn update_tray(hwnd: HWND, app: &mut App) {
    let short = tray_icon::quantize(app.value("primary"), app.icon_step);
    let long = tray_icon::quantize(app.value("secondary"), app.icon_step);
    let icon = create_tray_icon(short, long, app.stale || app.error.is_some());
    let actual = |key: &str| {
        app.value(key)
            .map_or("--".into(), |v| format!("{:.0}", v.clamp(0.0, 100.0)))
    };
    let tip = format!(
        "QuotaDock · 5h {}% · 7d {}%{}",
        actual("primary"),
        actual("secondary"),
        if app.error.is_some() {
            " · 刷新失败，显示上次数据"
        } else if app.stale {
            " · 正在刷新，显示上次数据"
        } else {
            ""
        }
    );
    let tip = if app.task_status_enabled || app.task_ribbon_enabled { format!("{tip} · {}", app.task_state.label()) } else { tip };
    let data = tray_data(hwnd, icon, &tip);
    Shell_NotifyIconW(if app.tray_added { NIM_MODIFY } else { NIM_ADD }, &data);
    app.tray_added = true;
    if !app.icon.is_null() {
        DestroyIcon(app.icon);
    }
    app.icon = icon;
}

unsafe fn receive_task_notices(hwnd: HWND) {
    APP.with(|cell| {
        let mut app = cell.borrow_mut();
        let dir = app.dir.clone();
        let state = session_watch::task_state();
        let state_changed = app.task_state != state;
        if state_changed && state == session_watch::TaskState::Running { app.task_pulse_started = Instant::now(); }
        app.task_state = state;
        let source = app.task_source;
        let events = app.task_inbox.drain(&dir, source);
        if app.task_notifications {
            for event in events {
                if app.pending_notices.len() < 32 {
                    app.pending_notices.push_back(task_notice::notice(event));
                }
            }
            show_next_notice(hwnd, &mut app);
        }
        if state_changed && (app.task_status_enabled || app.task_ribbon_enabled) {
            update_tray(hwnd, &mut app);
            if app.transition.is_none() { paint_layered(hwnd, &app); }
        }
        sync_task_light_timer(hwnd, &app);
    });
}

unsafe fn show_next_notice(hwnd: HWND, app: &mut App) {
    if app.notice_active {
        return;
    }
    let Some(notice) = app.pending_notices.pop_front() else {
        return;
    };
    app.notice_active = true;
    if notice.task || app.reset_alerts.style == reset_alert::Style::Island {
        let mut rect: RECT = zeroed();
        GetWindowRect(hwnd, &mut rect);
        if app.notice_origin.is_none() {
            let mut origin = POINT {
                x: rect.left,
                y: rect.top,
            };
            if app.compact && app.fusion && app.dock.vertical() {
                let work = work_area(hwnd);
                let height = (500.0 * app.scale * app.dpi).round() as i32;
                let width = (28.0 * app.scale * app.dpi).round() as i32;
                origin.x = if app.dock == docking::Edge::Right {
                    work.right - width
                } else {
                    work.left
                };
                if let Some((anchor, fraction)) = app.side_hover_anchor {
                    origin.y = (anchor - height as f32 * fraction).round() as i32;
                }
                origin.y = origin
                    .y
                    .clamp(work.top, (work.bottom - height).max(work.top));
            }
            app.notice_origin = Some(origin);
        }
        app.side_hover_anchor = None;
        app.notice_width = Some(if app.fusion && app.compact && app.dock.vertical() {
            (500.0 * app.scale * app.dpi).round() as i32
        } else {
            rect.right - rect.left
        });
        app.restore_after_transition = false;
        SetWindowTextW(hwnd, wide(&notice.title).as_ptr());
        app.island_notice = Some(notice);
        app.notice_until = Some(Instant::now() + Duration::from_secs(10));
        begin_transition(hwnd, app);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, 2, 100, None);
        return;
    }
    let owner = hwnd as isize;
    thread::spawn(move || unsafe {
        let title = wide(&notice.title);
        let body = wide(&notice.body);
        MessageBoxW(
            null_mut(),
            body.as_ptr(),
            title.as_ptr(),
            MB_OK
                | MB_TOPMOST
                | MB_SETFOREGROUND
                | if notice.important {
                    MB_ICONWARNING
                } else {
                    MB_ICONINFORMATION
                },
        );
        PostMessageW(owner as HWND, NOTICE_DONE, 0, 0);
    });
}

unsafe fn finish_island_notice(hwnd: HWND, app: &mut App) {
    if app.island_notice.take().is_none() {
        return;
    }
    KillTimer(hwnd, 2);
    app.notice_until = None;
    app.notice_active = false;
    app.notice_width = None;
    SetWindowTextW(hwnd, wide("QuotaDock").as_ptr());
    // Hover expansion belongs to the underlying HUD, not to the temporary notice.
    app.expanded = false;
    app.restore_after_transition = true;
    begin_transition(hwnd, app);
}

unsafe fn advance_island_notice(hwnd: HWND, app: &mut App) {
    if app
        .pending_notices
        .front()
        .is_some_and(|notice| notice.task || app.reset_alerts.style == reset_alert::Style::Island)
    {
        let notice = app.pending_notices.pop_front().unwrap();
        SetWindowTextW(hwnd, wide(&notice.title).as_ptr());
        app.island_notice = Some(notice);
        app.notice_until = Some(Instant::now() + Duration::from_secs(10));
        begin_transition(hwnd, app);
    } else {
        finish_island_notice(hwnd, app);
        show_next_notice(hwnd, app);
    }
}

fn enqueue_preview(app: &mut App) {
    app.pending_notices.push_back(reset_alert::Notice {
        title: "QuotaDock · 7 天额度已重置".into(),
        body: "这是一条一级提醒预览，当前样式结束后将显示二级提醒。".into(),
        important: true,
        task: false,
    });
    app.pending_notices.push_back(reset_alert::Notice {
        title: "QuotaDock · 5 小时额度已重置".into(),
        body: "这是一条二级提醒预览，真实提醒会显示剩余额度和下次重置时间。".into(),
        important: false,
        task: false,
    });
}

unsafe fn render_frame(app: &App) -> Option<motion::Frame> {
    let width = app.target_width();
    let height = app.target_height();
    if width <= 0 || height <= 0 {
        return None;
    }
    let memory = CreateCompatibleDC(null_mut());
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut bits: *mut c_void = null_mut();
    let bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        DeleteDC(memory);
        return None;
    }
    std::ptr::write_bytes(bits as *mut u8, 0, (width * height * 4) as usize);
    let old = SelectObject(memory, bitmap);
    let mut graphics = null_mut();
    if GdipCreateFromHDC(memory, &mut graphics) == 0 {
        GdipScaleWorldTransform(
            graphics,
            app.render_scale(),
            app.render_scale(),
            MatrixOrderAppend,
        );
        draw::paint(graphics, &app.display());
        GdipDeleteGraphics(graphics);
    }
    let pixels =
        std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize).to_vec();
    SelectObject(memory, old);
    DeleteObject(bitmap);
    DeleteDC(memory);
    Some(motion::Frame {
        width,
        height,
        pixels,
    })
}

unsafe fn begin_transition(hwnd: HWND, app: &mut App) {
    // A strip at the physical bottom is inside the taskbar area. Keep that
    // placement visible, then restore the user's pin setting when dragged away.
    SetWindowPos(hwnd, if app.pinned || app.island_notice.is_some() || (app.compact && app.fusion && app.dock == docking::Edge::ScreenBottom) { HWND_TOPMOST } else { HWND_NOTOPMOST }, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    if app.compact && app.expanded && app.island_notice.is_none() {
        SetTimer(hwnd, 3, 100, None);
    } else {
        KillTimer(hwnd, 3);
        app.hover_outside_since = None;
    }
    let previous = app.last_frame.borrow().clone();
    if let (Some(from), Some(to)) = (previous, render_frame(app)) {
        app.transition = Some(motion::Transition {
            from,
            to,
            started: Instant::now(),
            duration: Duration::from_millis(320),
            from_shell: app.last_shell.get(),
            to_shell: app.target_shell(),
        });
        set_animation_running(true);
    }
    size_window(hwnd, app, false);
}

fn task_light_animates(app: &App) -> bool {
    app.visible && app.compact && !app.expanded && !app.dragging
        && app.island_notice.is_none() && app.transition.is_none()
        && (app.task_status_enabled || app.task_ribbon_enabled) && app.task_state == session_watch::TaskState::Running
}

unsafe fn sync_task_light_timer(hwnd: HWND, app: &App) {
    let enabled = task_light_animates(app);
    if enabled != app.task_pulse_timer.get() {
        if enabled {
            let interval = if app.task_ribbon_enabled { 33 } else { 50 };
            app.task_pulse_timer.set(SetTimer(hwnd, TASK_LIGHT_TIMER, interval, None) != 0);
        } else {
            KillTimer(hwnd, TASK_LIGHT_TIMER);
            app.task_pulse_timer.set(false);
        }
    }
}

unsafe fn paint_layered(hwnd: HWND, app: &App) {
    sync_task_light_timer(hwnd, app);
    let frame = if let Some(transition) = &app.transition {
        let buffer = app
            .last_frame
            .borrow_mut()
            .take()
            .map(|frame| frame.pixels)
            .unwrap_or_default();
        Some(transition.frame_into(buffer))
    } else {
        render_frame(app)
    };
    let Some(frame) = frame else {
        return;
    };
    let width = frame.width;
    let height = frame.height;
    let screen = GetDC(null_mut());
    let required_width = app
        .transition
        .as_ref()
        .map(|transition| transition.from.width.max(transition.to.width))
        .unwrap_or(width);
    let required_height = app
        .transition
        .as_ref()
        .map(|transition| transition.from.height.max(transition.to.height))
        .unwrap_or(height);
    let mut surface = app.surface.borrow_mut();
    if surface
        .as_ref()
        .is_none_or(|surface| surface.width < required_width || surface.height < required_height)
    {
        *surface = LayerSurface::new(required_width, required_height);
    }
    let Some(surface) = surface.as_ref() else {
        ReleaseDC(null_mut(), screen);
        return;
    };
    let shell = app
        .transition
        .as_ref()
        .map(|transition| transition.shell())
        .unwrap_or_else(|| app.target_shell());
    motion::compose_strided(
        &frame,
        shell,
        std::slice::from_raw_parts_mut(
            surface.bits as *mut u8,
            (surface.width * surface.height * 4) as usize,
        ),
        surface.width as usize * 4,
    );
    let mut rect: RECT = zeroed();
    GetWindowRect(hwnd, &mut rect);
    let position = dock_position(hwnd, app, rect.left, rect.top, width, height);
    let size = SIZE {
        cx: width,
        cy: height,
    };
    let origin = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    UpdateLayeredWindow(
        hwnd, screen, &position, &size, surface.dc, &origin, 0, &blend, ULW_ALPHA,
    );
    ReleaseDC(null_mut(), screen);
    app.last_frame.replace(Some(frame));
    app.last_shell.set(shell);
}
unsafe fn work_area(hwnd: HWND) -> RECT {
    monitor_info(hwnd).rcWork
}
unsafe fn monitor_info(hwnd: HWND) -> MONITORINFO {
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(
        MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    );
    monitor
}
unsafe fn dock_position(hwnd: HWND, app: &App, x: i32, y: i32, width: i32, height: i32) -> POINT {
    if app.dragging {
        return POINT { x, y };
    }
    if !app.fusion && app.compact {
        return POINT { x, y };
    }
    let monitor = monitor_info(hwnd);
    let work = monitor.rcWork;
    let dock = if app.compact {
        app.dock
    } else {
        docking::Edge::None
    };
    let bottom = if dock == docking::Edge::ScreenBottom { monitor.rcMonitor.bottom } else { work.bottom };
    let x = match dock {
        docking::Edge::Left => work.left,
        docking::Edge::Right => work.right - width,
        _ => x,
    };
    // Keep the pointer inside the side panel as its long axis contracts.
    let y = if dock.vertical() && (app.expanded || app.transition.is_some()) {
        app.side_hover_anchor
            .map(|(anchor, fraction)| (anchor - height as f32 * fraction).round() as i32)
            .unwrap_or(y)
    } else {
        y
    };
    let y = match dock {
        docking::Edge::Top => work.top,
        docking::Edge::Bottom => work.bottom - height,
        docking::Edge::ScreenBottom => bottom - height,
        _ => y,
    };
    POINT {
        x: x.clamp(work.left, (work.right - width).max(work.left)),
        y: y.clamp(work.top, (bottom - height).max(work.top)),
    }
}
unsafe fn update_dock(hwnd: HWND, app: &mut App) {
    let mut rect: RECT = zeroed();
    GetWindowRect(hwnd, &mut rect);
    let monitor = monitor_info(hwnd);
    let work = monitor.rcWork;
    let bounds = |r: RECT| docking::Bounds {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    };
    app.dock = docking::detect_with_screen(bounds(rect), bounds(work), bounds(monitor.rcMonitor), app.dock, app.dpi);
    app.strip = app.dock != docking::Edge::None;
    app.expanded = false;
    app.side_hover_anchor = None;
}
unsafe fn size_window(hwnd: HWND, app: &App, center: bool) {
    if app.transition.is_some() && !center {
        paint_layered(hwnd, app);
        return;
    }
    let mut rect: RECT = zeroed();
    GetWindowRect(hwnd, &mut rect);
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(
        MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    );
    let work = monitor.rcWork;
    let notice = app.island_notice.is_some();
    let x = if center {
        work.left + (work.right - work.left - app.width()) / 2
    } else if notice {
        rect.left
    } else {
        (rect.left + (rect.right - rect.left - app.width()) / 2)
            .clamp(work.left, (work.right - app.width()).max(work.left))
    };
    let y = if center { work.top + 6 } else { rect.top };
    let point = dock_position(hwnd, app, x, y, app.width(), app.height());
    SetWindowPos(
        hwnd,
        if app.pinned || notice || (app.compact && app.fusion && app.dock == docking::Edge::ScreenBottom) {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        },
        point.x,
        point.y,
        app.width(),
        app.height(),
        SWP_NOACTIVATE,
    );
    paint_layered(hwnd, app);
}
unsafe fn popup(hwnd: HWND) {
    let menu = CreatePopupMenu();
    let layout = CreatePopupMenu();
    let appearance = CreatePopupMenu();
    let general = CreatePopupMenu();
    let integration = CreatePopupMenu();
    let hooks = CreatePopupMenu();
    let icon_step = CreatePopupMenu();
    let refresh = CreatePopupMenu();
    let alerts = CreatePopupMenu();
    let weekly_alerts = CreatePopupMenu();
    let hourly_alerts = CreatePopupMenu();
    let notification_style = CreatePopupMenu();
    let tasks = CreatePopupMenu();
    let task_sources = CreatePopupMenu();
    let task_chars = CreatePopupMenu();
    let task_lines = CreatePopupMenu();
    let task_status = CreatePopupMenu();
    let add = |target: HMENU, id: usize, title: &str, checked: bool| {
        let title = wide(title);
        AppendMenuW(
            target,
            MF_STRING | if checked { MF_CHECKED } else { 0 },
            id,
            title.as_ptr(),
        );
    };
    let sub = |title: &str, target: HMENU| {
        let title = wide(title);
        AppendMenuW(menu, MF_POPUP, target as usize, title.as_ptr());
    };
    APP.with(|cell| {
        let app = cell.borrow();
        add(
            menu,
            CMD_TOGGLE,
            if app.visible {
                "隐藏窗口"
            } else {
                "显示窗口"
            },
            false,
        );
        add(menu, CMD_REFRESH, "刷新额度", false);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(
            layout,
            CMD_COMPACT,
            "紧凑 HUD",
            app.compact && !app.fusion && !app.strip,
        );
        add(
            layout,
            CMD_STRIP,
            "顶部细条",
            app.compact && !app.fusion && app.strip,
        );
        add(
            layout,
            CMD_FUSION,
            "融合模式（靠边自动切换）",
            app.compact && app.fusion,
        );
        add(layout, CMD_FULL, "完整窗口", !app.compact);
        AppendMenuW(layout, MF_SEPARATOR, 0, null());
        add(layout, CMD_PIN, "始终置顶", app.pinned);
        sub("窗口布局", layout);
        let color_themes = CreatePopupMenu();
        for (index, theme) in theme::Theme::ALL.iter().enumerate() {
            add(color_themes, CMD_COLOR_THEME_BASE + index, theme.label(), app.color_theme == *theme);
        }
        AppendMenuW(appearance, MF_POPUP, color_themes as usize, wide(&format!("颜色主题（{}）", app.color_theme.label())).as_ptr());
        add(appearance, CMD_THEME, "紧凑 HUD · 深色底板", app.island);
        sub("外观与托盘", appearance);
        add(icon_step, CMD_STEP_1, "1% 档位", app.icon_step == 1);
        add(icon_step, CMD_STEP_3, "3% 档位", app.icon_step == 3);
        add(icon_step, CMD_STEP_5, "5% 档位", app.icon_step == 5);
        AppendMenuW(appearance, MF_POPUP, icon_step as usize, wide(&format!("托盘图标精度（{}%）", app.icon_step)).as_ptr());
        add(general, CMD_AUTOSTART, "开机启动 QuotaDock", autostart_enabled());
        AppendMenuW(general, MF_SEPARATOR, 0, null());
        for (id, mins) in [
            (CMD_INTERVAL_1, 1),
            (CMD_INTERVAL_3, 3),
            (CMD_INTERVAL_5, 5),
            (CMD_INTERVAL_10, 10),
            (CMD_INTERVAL_30, 30),
        ] {
            add(
                refresh,
                id,
                &format!("{mins} 分钟"),
                app.settings.quota_refresh_ms == mins * 60_000,
            );
        }
        AppendMenuW(general, MF_POPUP, refresh as usize, wide(&format!("额度刷新间隔（{} 分钟）", app.settings.quota_refresh_ms / 60_000)).as_ptr());
        add(
            alerts,
            CMD_ALERT_MASTER,
            "启用重置通知",
            app.reset_alerts.enabled,
        );
        add(
            notification_style,
            CMD_STYLE_POPUP,
            "弹窗",
            app.reset_alerts.style == reset_alert::Style::Popup,
        );
        add(
            notification_style,
            CMD_STYLE_ISLAND,
            "灵动岛（默认）",
            app.reset_alerts.style == reset_alert::Style::Island,
        );
        AppendMenuW(
            alerts,
            MF_POPUP,
            notification_style as usize,
            wide("通知样式").as_ptr(),
        );
        AppendMenuW(alerts, MF_SEPARATOR, 0, null());
        add(
            weekly_alerts,
            CMD_ALERT_7D_TIME,
            "到时间重置",
            app.reset_alerts.weekly_timed,
        );
        add(
            weekly_alerts,
            CMD_ALERT_7D_BONUS,
            "官方奖励重置",
            app.reset_alerts.weekly_bonus,
        );
        AppendMenuW(
            alerts,
            MF_POPUP,
            weekly_alerts as usize,
            wide("一级 · 7 天额度").as_ptr(),
        );
        add(
            hourly_alerts,
            CMD_ALERT_5H_TIME,
            "到时间重置",
            app.reset_alerts.hourly_timed,
        );
        AppendMenuW(
            alerts,
            MF_POPUP,
            hourly_alerts as usize,
            wide("二级 · 5 小时额度").as_ptr(),
        );
        AppendMenuW(alerts, MF_SEPARATOR, 0, null());
        add(alerts, CMD_ALERT_PREVIEW, "预览两级通知", false);
        add(
            tasks,
            CMD_TASK_ENABLED,
            "任务结束通知",
            app.task_notifications,
        );
        add(
            integration,
            CMD_PERMISSION_ENABLED,
            "显示允许 / 拒绝确认框",
            app.permission_prompts,
        );
        add(task_sources, CMD_TASK_SOURCE_LOCAL, "本地记录（默认）", app.task_source == task_notice::Source::Local);
        add(task_sources, CMD_TASK_SOURCE_HOOK, "仅 Hook", app.task_source == task_notice::Source::Hook);
        add(task_sources, CMD_TASK_SOURCE_BOTH, "本地记录 + Hook（自动去重）", app.task_source == task_notice::Source::Both);
        AppendMenuW(integration, MF_SEPARATOR, 0, null());
        AppendMenuW(integration, MF_POPUP, task_sources as usize, wide(&format!("任务接收方式（{}）", app.task_source.label())).as_ptr());
        add(task_status, CMD_TASK_STATUS, "显示状态灯", app.task_status_enabled);
        AppendMenuW(task_status, MF_SEPARATOR, 0, null());
        add(task_status, CMD_TASK_STATUS_BAR, "条形 · 进行中呼吸", app.task_light_style == draw::StatusLightStyle::Bar);
        add(task_status, CMD_TASK_STATUS_DOT, "点状 · 进行中旋转", app.task_light_style == draw::StatusLightStyle::Dot);
        AppendMenuW(task_status, MF_SEPARATOR, 0, null());
        add(task_status, CMD_TASK_STATUS_RIBBON, "显示边缘丝带", app.task_ribbon_enabled);
        let ribbon_strength = CreatePopupMenu();
        for (index, strength) in draw::RibbonStrength::ALL.iter().enumerate() {
            add(ribbon_strength, CMD_RIBBON_STRENGTH_BASE + index, strength.label(), app.task_ribbon_strength == *strength);
        }
        AppendMenuW(task_status, MF_POPUP, ribbon_strength as usize, wide(&format!("丝带强度（{}）", app.task_ribbon_strength.label())).as_ptr());
        sub("任务状态与丝带", task_status);
        for (index, count) in task_notice::TEXT_LENGTHS.iter().enumerate() {
            add(task_chars, CMD_TASK_CHARS_BASE + index, &format!("{count} 字"), app.task_text.max_chars == *count);
        }
        for (index, count) in task_notice::TEXT_LINES.iter().enumerate() {
            add(task_lines, CMD_TASK_LINES_BASE + index, &format!("{count} 行"), app.task_text.max_lines == *count);
        }
        AppendMenuW(tasks, MF_POPUP, task_chars as usize, wide(&format!("摘要上限（{} 字）", app.task_text.max_chars)).as_ptr());
        AppendMenuW(tasks, MF_POPUP, task_lines as usize, wide(&format!("行数上限（{} 行）", app.task_text.max_lines)).as_ptr());
        add(tasks, CMD_TASK_PREVIEW, "预览任务通知", false);
        AppendMenuW(tasks, MF_SEPARATOR, 0, null());
        AppendMenuW(tasks, MF_POPUP, alerts as usize, wide(&format!("额度重置通知（{}）", if app.reset_alerts.enabled { "开" } else { "关" })).as_ptr());
        add(hooks, CMD_TASK_INSTALL, "安装 Hook 并打开审阅", false);
        add(hooks, CMD_TASK_REVIEW, "打开 Hook 审阅终端", false);
        AppendMenuW(integration, MF_POPUP, hooks as usize, wide("Hook 设置").as_ptr());
        sub("通知设置", tasks);
        sub("Codex 集成", integration);
        sub("常规设置", general);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(menu, CMD_QUIT, "退出 QuotaDock", false);
    });
    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    APP.with(|cell| cell.borrow_mut().menu_open = true);
    TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, null());
    APP.with(|cell| {
        let mut app = cell.borrow_mut();
        app.menu_open = false;
        app.hover_outside_since = None;
    });
    DestroyMenu(menu);
}

// Screenshot overlays can swallow WM_MOUSELEAVE. While expanded, verify the
// actual cursor position as well; no polling is needed for a collapsed HUD.
unsafe fn check_hover_exit(hwnd: HWND) {
    APP.with(|cell| {
        let mut app = cell.borrow_mut();
        if !app.compact || !app.expanded || app.island_notice.is_some() || !app.visible {
            app.hover_outside_since = None;
            KillTimer(hwnd, 3);
            return;
        }
        if app.dragging || app.menu_open {
            app.hover_outside_since = None;
            return;
        }
        let mut cursor: POINT = zeroed();
        let mut bounds: RECT = zeroed();
        if GetCursorPos(&mut cursor) == 0 || GetWindowRect(hwnd, &mut bounds) == 0 { return; }
        let inside = cursor.x >= bounds.left && cursor.x < bounds.right
            && cursor.y >= bounds.top && cursor.y < bounds.bottom;
        if inside {
            app.hover_outside_since = None;
        } else if app.hover_outside_since.get_or_insert_with(Instant::now)
            .elapsed() >= Duration::from_millis(150) {
            app.expanded = false;
            app.hover_outside_since = None;
            begin_transition(hwnd, &mut app);
        }
    });
}
unsafe fn command(hwnd: HWND, id: usize) {
    if (CMD_COLOR_THEME_BASE..CMD_COLOR_THEME_BASE + theme::Theme::ALL.len()).contains(&id) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            app.color_theme = theme::Theme::ALL[id - CMD_COLOR_THEME_BASE];
            app.save_settings();
            begin_transition(hwnd, &mut app);
        });
        return;
    }
    if (CMD_RIBBON_STRENGTH_BASE..CMD_RIBBON_STRENGTH_BASE + draw::RibbonStrength::ALL.len()).contains(&id) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            app.task_ribbon_strength = draw::RibbonStrength::ALL[id - CMD_RIBBON_STRENGTH_BASE];
            app.save_settings();
            begin_transition(hwnd, &mut app);
        });
        return;
    }
    if matches!(id, CMD_TASK_STATUS_BAR | CMD_TASK_STATUS_DOT) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            app.task_light_style = match id {
                CMD_TASK_STATUS_DOT => draw::StatusLightStyle::Dot,
                _ => draw::StatusLightStyle::Bar,
            };
            app.save_settings();
            begin_transition(hwnd, &mut app);
        });
        return;
    }
    if matches!(id, CMD_TASK_STATUS | CMD_TASK_STATUS_RIBBON) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            if id == CMD_TASK_STATUS { app.task_status_enabled = !app.task_status_enabled; }
            else { app.task_ribbon_enabled = !app.task_ribbon_enabled; }
            session_watch::set_status_enabled(app.task_status_enabled || app.task_ribbon_enabled);
            app.task_state = session_watch::task_state();
            app.save_settings();
            update_tray(hwnd, &mut app);
            begin_transition(hwnd, &mut app);
        });
        return;
    }
    if (CMD_TASK_CHARS_BASE..CMD_TASK_CHARS_BASE + task_notice::TEXT_LENGTHS.len()).contains(&id)
        || (CMD_TASK_LINES_BASE..CMD_TASK_LINES_BASE + task_notice::TEXT_LINES.len()).contains(&id) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            if let Some(index) = id.checked_sub(CMD_TASK_CHARS_BASE).filter(|i| *i < task_notice::TEXT_LENGTHS.len()) {
                app.task_text.max_chars = task_notice::TEXT_LENGTHS[index];
            } else {
                app.task_text.max_lines = task_notice::TEXT_LINES[id - CMD_TASK_LINES_BASE];
            }
            app.save_settings();
            if app.island_notice.as_ref().is_some_and(|notice| notice.task) {
                begin_transition(hwnd, &mut app);
            }
        });
        return;
    }
    if id == CMD_REFRESH {
        request_refresh();
        return;
    }
    if id == CMD_QUIT {
        DestroyWindow(hwnd);
        return;
    }
    if id == CMD_NOTICE_DISMISS {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            advance_island_notice(hwnd, &mut app);
        });
        return;
    }
    if matches!(id, CMD_STYLE_POPUP | CMD_STYLE_ISLAND) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            app.reset_alerts.style = if id == CMD_STYLE_POPUP {
                reset_alert::Style::Popup
            } else {
                reset_alert::Style::Island
            };
            app.save_settings();
        });
        return;
    }
    if matches!(id, CMD_TASK_SOURCE_LOCAL | CMD_TASK_SOURCE_HOOK | CMD_TASK_SOURCE_BOTH) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            app.task_source = match id {
                CMD_TASK_SOURCE_HOOK => task_notice::Source::Hook,
                CMD_TASK_SOURCE_BOTH => task_notice::Source::Both,
                _ => task_notice::Source::Local,
            };
            session_watch::set_enabled(app.task_source != task_notice::Source::Hook);
            app.pending_notices.retain(|notice| !notice.task);
            app.save_settings();
        });
        return;
    }
    if matches!(id, CMD_TASK_INSTALL | CMD_TASK_REVIEW) {
        let result = task_notice::codex_home()
            .ok_or_else(|| "未找到 Codex 配置目录".to_string())
            .and_then(|home| {
                if id == CMD_TASK_INSTALL {
                    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                    task_notice::install(&home, &exe)?;
                }
                task_notice::open_review(&home)?;
                Ok(home.join("hooks.json"))
            });
        let (message, icon) = match result {
            Ok(_) => {
                if id == CMD_TASK_INSTALL {
                    APP.with(|cell| {
                        let mut app = cell.borrow_mut();
                        app.task_notifications = true;
                        app.save_settings();
                    });
                    APP.with(|cell| {
                        let mut app = cell.borrow_mut();
                        app.permission_prompts = true;
                        app.save_settings();
                    });
                }
                ("已启动 Codex 审阅终端，请先关闭此提示框。\n\n1. 等待 Codex 界面出现，输入 /hooks。\n2. 检查并信任‘QuotaDock 任务通知’（Stop）和‘QuotaDock 权限确认’（PermissionRequest）。\n3. 重新打开桌面版 Codex 会话。\n\n权限确认默认选中拒绝；允许或拒绝后仅对当前请求生效。也可从托盘菜单关闭 QuotaDock 确认框，恢复 Codex 原生授权提示。\n\n如果终端显示启动错误，则尚未完成确认，请按错误提示重试。".into(), MB_ICONINFORMATION)
            }
            Err(error) => (
                format!(
                    "设置未完成：{error}\n\n可先检查 hooks.json，再手动进入 Codex 的 /hooks 审阅。"
                ),
                MB_ICONERROR,
            ),
        };
        MessageBoxW(
            hwnd,
            wide(&message).as_ptr(),
            wide("QuotaDock · Codex Hook").as_ptr(),
            MB_OK | icon,
        );
        return;
    }
    if matches!(
        id,
        CMD_TASK_ENABLED | CMD_TASK_PREVIEW | CMD_PERMISSION_ENABLED
    ) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            if id == CMD_TASK_ENABLED {
                app.task_notifications = !app.task_notifications;
                if !app.task_notifications {
                    app.pending_notices.retain(|notice| !notice.task);
                    if app.island_notice.as_ref().is_some_and(|notice| notice.task) {
                        finish_island_notice(hwnd, &mut app);
                        show_next_notice(hwnd, &mut app);
                    }
                }
                app.save_settings();
            } else if id == CMD_PERMISSION_ENABLED {
                app.permission_prompts = !app.permission_prompts;
                app.save_settings();
            } else {
                app.pending_notices
                    .push_back(task_notice::notice(task_notice::preview()));
                show_next_notice(hwnd, &mut app);
            }
        });
        return;
    }
    if id == CMD_AUTOSTART {
        if let Err(code) = set_autostart(!autostart_enabled()) {
            let message = wide(&format!("设置开机自启动失败（错误码 {code}）。"));
            MessageBoxW(
                hwnd,
                message.as_ptr(),
                wide("QuotaDock").as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
        return;
    }
    if id == CMD_ALERT_PREVIEW {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            enqueue_preview(&mut app);
            show_next_notice(hwnd, &mut app);
        });
        return;
    }
    if matches!(
        id,
        CMD_ALERT_MASTER | CMD_ALERT_7D_TIME | CMD_ALERT_7D_BONUS | CMD_ALERT_5H_TIME
    ) {
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            match id {
                CMD_ALERT_MASTER => {
                    app.reset_alerts.enabled = !app.reset_alerts.enabled;
                    if !app.reset_alerts.enabled {
                        app.pending_notices.retain(|notice| notice.task);
                        if app
                            .island_notice
                            .as_ref()
                            .is_some_and(|notice| !notice.task)
                        {
                            finish_island_notice(hwnd, &mut app);
                            show_next_notice(hwnd, &mut app);
                        }
                    }
                }
                CMD_ALERT_7D_TIME => app.reset_alerts.weekly_timed = !app.reset_alerts.weekly_timed,
                CMD_ALERT_7D_BONUS => {
                    app.reset_alerts.weekly_bonus = !app.reset_alerts.weekly_bonus
                }
                CMD_ALERT_5H_TIME => app.reset_alerts.hourly_timed = !app.reset_alerts.hourly_timed,
                _ => {}
            }
            app.schedule();
            app.save_settings();
        });
        return;
    }
    APP.with(|cell| {
        let mut app = cell.borrow_mut();
        let mut reposition = false;
        match id {
            CMD_TOGGLE => {
                app.visible = !app.visible;
                ShowWindow(hwnd, if app.visible { SW_SHOW } else { SW_HIDE });
                app.schedule();
            }
            CMD_COMPACT => {
                app.compact = true;
                app.fusion = false;
                app.dock = docking::Edge::None;
                app.strip = false;
                app.expanded = false;
                reposition = true;
            }
            CMD_STRIP => {
                app.compact = true;
                app.fusion = false;
                app.dock = docking::Edge::None;
                app.strip = true;
                app.expanded = false;
                reposition = true;
            }
            CMD_FUSION => {
                app.compact = true;
                app.fusion = true;
                app.dock = docking::Edge::None;
                update_dock(hwnd, &mut app);
            }
            CMD_FULL | CMD_TOGGLE_FULL => {
                if app.island_notice.is_some() {
                    return;
                }
                if app.compact {
                    let mut rect: RECT = zeroed();
                    GetWindowRect(hwnd, &mut rect);
                    let mut origin = POINT {
                        x: rect.left,
                        y: rect.top,
                    };
                    if app.fusion && app.dock.vertical() {
                        let height = (500.0 * app.scale * app.dpi).round() as i32;
                        let width = (28.0 * app.scale * app.dpi).round() as i32;
                        if let Some((anchor, fraction)) = app.side_hover_anchor {
                            origin.y = (anchor - height as f32 * fraction).round() as i32;
                        }
                        let work = work_area(hwnd);
                        origin.x = if app.dock == docking::Edge::Right {
                            work.right - width
                        } else {
                            work.left
                        };
                        origin.y = origin
                            .y
                            .clamp(work.top, (work.bottom - height).max(work.top));
                    }
                    app.return_view = Some(CompactView {
                        fusion: app.fusion,
                        strip: app.strip,
                        dock: app.dock,
                        origin,
                    });
                    app.compact = false;
                } else if id == CMD_TOGGLE_FULL {
                    app.compact = true;
                    if let Some(view) = app.return_view.take() {
                        app.fusion = view.fusion;
                        app.strip = view.strip;
                        app.dock = view.dock;
                        SetWindowPos(
                            hwnd,
                            null_mut(),
                            view.origin.x,
                            view.origin.y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                app.expanded = false;
                app.side_hover_anchor = None;
            }
            CMD_PIN => app.pinned = !app.pinned,
            CMD_THEME => app.island = !app.island,
            CMD_STEP_1 => app.icon_step = 1,
            CMD_STEP_3 => app.icon_step = 3,
            CMD_STEP_5 => app.icon_step = 5,
            CMD_INTERVAL_1 | CMD_INTERVAL_3 | CMD_INTERVAL_5 | CMD_INTERVAL_10
            | CMD_INTERVAL_30 => {
                app.settings.quota_refresh_ms = ((id - 200) as u64) * 60_000;
                app.schedule();
            }
            _ => {}
        }
        if matches!(
            id,
            CMD_COMPACT
                | CMD_THEME
                | CMD_PIN
                | CMD_STRIP
                | CMD_FUSION
                | CMD_FULL
                | CMD_TOGGLE_FULL
                | CMD_STEP_1
                | CMD_STEP_3
                | CMD_STEP_5
                | CMD_INTERVAL_1
                | CMD_INTERVAL_3
                | CMD_INTERVAL_5
                | CMD_INTERVAL_10
                | CMD_INTERVAL_30
        ) {
            app.save_settings();
        }
        update_tray(hwnd, &mut app);
        if matches!(id, CMD_FUSION | CMD_FULL | CMD_TOGGLE_FULL) {
            begin_transition(hwnd, &mut app);
        } else {
            app.transition = None;
            size_window(hwnd, &app, reposition);
        }
    });
}
unsafe extern "system" fn wndproc(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    match message {
        WM_CREATE => {
            MAIN_HWND.store(hwnd as isize, Ordering::SeqCst);
            SetTimer(hwnd, 1, 1000, None);
            0
        }
        ANIMATION_FRAME => {
            APP.with(|cell| {
                let mut app = cell.borrow_mut();
                if app
                    .transition
                    .as_ref()
                    .is_none_or(|transition| transition.progress() >= 1.0)
                {
                    app.transition = None;
                    set_animation_running(false);
                    if app.restore_after_transition {
                        app.restore_after_transition = false;
                        if let Some(origin) = app.notice_origin.take() {
                            SetWindowPos(
                                hwnd,
                                if app.pinned || (app.compact && app.fusion && app.dock == docking::Edge::ScreenBottom) {
                                    HWND_TOPMOST
                                } else {
                                    HWND_NOTOPMOST
                                },
                                origin.x,
                                origin.y,
                                app.width(),
                                app.target_height(),
                                SWP_NOACTIVATE,
                            );
                        }
                        if !app.visible {
                            ShowWindow(hwnd, SW_HIDE);
                        }
                    }
                }
                paint_layered(hwnd, &app);
            });
            FRAME_QUEUED.store(false, Ordering::Release);
            0
        }
        TASK_MESSAGE => {
            receive_task_notices(hwnd);
            0
        }
        WM_TIMER => {
            if wparam == TASK_LIGHT_TIMER {
                APP.with(|cell| {
                    let app = cell.borrow();
                    if task_light_animates(&app) { paint_layered(hwnd, &app); }
                    else { sync_task_light_timer(hwnd, &app); }
                });
                return 0;
            }
            if wparam == 3 {
                check_hover_exit(hwnd);
                return 0;
            }
            if wparam == 2 {
                APP.with(|cell| {
                    let mut app = cell.borrow_mut();
                    if app
                        .notice_until
                        .is_some_and(|until| Instant::now() >= until)
                    {
                        advance_island_notice(hwnd, &mut app);
                    } else if app.island_notice.is_some() {
                        if app.transition.is_none() {
                            paint_layered(hwnd, &app);
                        }
                    }
                });
                return 0;
            }
            check_hover_exit(hwnd);
            receive_task_notices(hwnd);
            let due = APP.with(|cell| {
                let app = cell.borrow();
                !app.refreshing && Instant::now() >= app.next_refresh
            });
            if due {
                request_refresh();
            }
            0
        }
        NOTICE_DONE => {
            APP.with(|cell| {
                let mut app = cell.borrow_mut();
                app.notice_active = false;
                show_next_notice(hwnd, &mut app);
            });
            0
        }
        REFRESH_MESSAGE => {
            if let Some(result) = QUERY_RESULT.lock().unwrap().take() {
                APP.with(|cell| {
                    let mut app = cell.borrow_mut(); app.refreshing = false;
                    match result {
                        Ok(q) => { let notices = reset_alert::detect(app.quota.as_ref(), &q, &app.reset_alerts, Utc::now());
                            app.pending_notices.extend(notices);
                            app.quota = Some(q.clone()); app.stale = false; app.error = None; app.failures = 0;
                            app.history.push(json!({"fetchedAt":q["fetchedAt"],"primary":q["primary"],"secondary":q["secondary"]}));
                            let cutoff = Utc::now() - chrono::Duration::days(14);
                            app.history.retain(|v| v["fetchedAt"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).is_some_and(|d| d.with_timezone(&Utc) >= cutoff));
                            let excess = app.history.len().saturating_sub(4096);
                            app.history.drain(..excess);
                            save_json(app.dir.join("quota-history.json"), &json!({"version":1,"samples":app.history}));
                            save_json(app.dir.join("quota-cache.json"), &json!({"version":1,"savedAt":Utc::now().to_rfc3339(),"quota":q})); }
                        Err(e) => { app.stale = app.quota.is_some(); app.error = Some(e); app.failures += 1; }
                    }
                    app.schedule(); update_tray(hwnd, &mut app); paint_layered(hwnd, &app); show_next_notice(hwnd, &mut app);
                });
            }
            0
        }
        TRAY_MESSAGE => {
            match (lparam as u32) & 0xffff {
                WM_LBUTTONUP => command(hwnd, CMD_TOGGLE),
                WM_RBUTTONUP | WM_CONTEXTMENU => popup(hwnd),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            command(hwnd, wparam & 0xffff);
            0
        }
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
            1
        }
        WM_MOUSEMOVE => {
            APP.with(|cell| {
                let mut app = cell.borrow_mut();
                app.hover_outside_since = None;
                if app.island_notice.is_none() && app.compact && !app.dragging && !app.expanded {
                    if app.fusion && app.dock.vertical() {
                        let mut rect: RECT = zeroed();
                        GetWindowRect(hwnd, &mut rect);
                        let y = ((lparam as u32 >> 16) & 0xffff) as i16 as f32;
                        app.side_hover_anchor = Some((
                            rect.top as f32 + y,
                            (y / (rect.bottom - rect.top).max(1) as f32).clamp(0.0, 1.0),
                        ));
                    }
                    app.expanded = true;
                    begin_transition(hwnd, &mut app);
                }
            });
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut track);
            0
        }
        MOUSE_LEAVE => {
            SetTimer(hwnd, 3, 100, None);
            0
        }
        WM_LBUTTONDBLCLK => {
            if APP.with(|cell| cell.borrow().island_notice.is_some()) {
                return 0;
            }
            let x = (lparam as u32 & 0xffff) as i16 as i32;
            let y = ((lparam as u32 >> 16) & 0xffff) as i16 as i32;
            let toolbar = APP.with(|cell| {
                let app = cell.borrow();
                !app.compact
                    && (y as f32 / app.render_scale()) < 48.0
                    && (x as f32 / app.render_scale()) >= 246.0
            });
            if !toolbar {
                command(hwnd, CMD_TOGGLE_FULL);
            }
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lparam as u32 & 0xffff) as i16 as i32;
            let y = ((lparam as u32 >> 16) & 0xffff) as i16 as i32;
            let action = APP.with(|cell| {
                let app = cell.borrow();
                let scale = app.render_scale();
                let px = x as f32 / scale;
                let py = y as f32 / scale;
                if app.island_notice.is_some() {
                    if (440.0..=488.0).contains(&px) && (8.0..=60.0).contains(&py) {
                        CMD_NOTICE_DISMISS
                    } else {
                        usize::MAX
                    }
                } else if !app.compact && (17.0..=42.0).contains(&py) {
                    if (307.0..=332.0).contains(&px) {
                        CMD_TOGGLE
                    } else if (278.0..=303.0).contains(&px) {
                        CMD_REFRESH
                    } else if (249.0..=274.0).contains(&px) {
                        CMD_PIN
                    } else {
                        0
                    }
                } else {
                    0
                }
            });
            if action == usize::MAX {
                return 0;
            }
            if action != 0 {
                command(hwnd, action);
            } else {
                ReleaseCapture();
                SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
            }
            0
        }
        WM_RBUTTONUP => {
            popup(hwnd);
            0
        }
        WM_NCLBUTTONDBLCLK => {
            command(hwnd, CMD_TOGGLE_FULL);
            0
        }
        WM_ENTERSIZEMOVE => {
            KillTimer(hwnd, 3);
            APP.with(|cell| {
                let mut app = cell.borrow_mut();
                app.dragging = true;
                app.transition = None;
            });
            0
        }
        WM_EXITSIZEMOVE => {
            APP.with(|cell| {
                let mut app = cell.borrow_mut();
                app.dragging = false;
                if app.fusion && app.compact && app.island_notice.is_none() {
                    update_dock(hwnd, &mut app);
                    app.save_settings();
                    begin_transition(hwnd, &mut app);
                }
            });
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) as u16) as i16;
            APP.with(|cell| {
                let mut app = cell.borrow_mut();
                if app.compact && app.island_notice.is_none() {
                    app.scale =
                        (app.scale + if delta > 0 { 0.05 } else { -0.05 }).clamp(0.46, 1.25);
                    app.transition = None;
                    app.save_settings();
                    size_window(hwnd, &app, false);
                }
            });
            0
        }
        WM_CLOSE => {
            command(
                hwnd,
                if APP.with(|cell| cell.borrow().island_notice.is_some()) {
                    CMD_NOTICE_DISMISS
                } else {
                    CMD_TOGGLE
                },
            );
            0
        }
        WM_DESTROY => {
            APP.with(|cell| {
                let app = cell.borrow();
                if app.tray_added {
                    let data = tray_data(hwnd, app.icon, "");
                    Shell_NotifyIconW(NIM_DELETE, &data);
                }
                if !app.icon.is_null() {
                    DestroyIcon(app.icon);
                }
            });
            MAIN_HWND.store(0, Ordering::Release);
            set_animation_running(false);
            KillTimer(hwnd, 1);
            KillTimer(hwnd, 2);
            KillTimer(hwnd, 3);
            KillTimer(hwnd, TASK_LIGHT_TIMER);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.iter().any(|arg| arg == "--codex-stop") {
        // Never block or continue the Codex turn, even if delivery fails.
        if let Err(error) = task_notice::receive(&data_dir()) {
            eprintln!("QuotaDock hook: {error}");
        }
        unsafe {
            let hwnd = FindWindowW(wide(&window_class()).as_ptr(), null());
            if !hwnd.is_null() {
                PostMessageW(hwnd, TASK_MESSAGE, 0, 0);
            }
        }
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{{}}");
        return;
    }
    if args.iter().any(|arg| arg == "--codex-permission") {
        let decision = task_notice::read_permission_decision(&data_dir());
        let output = match decision {
            Some(true) => {
                json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}})
            }
            Some(false) => {
                json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"用户在 QuotaDock 权限确认框中拒绝了此请求。"}}})
            }
            None => json!({}),
        };
        println!("{output}");
        return;
    }
    if let Some(index) = args.iter().position(|arg| arg == "--install-codex-hook") {
        let home = args
            .get(index + 1)
            .map(PathBuf::from)
            .or_else(task_notice::codex_home);
        let result = home
            .ok_or_else(|| "Codex config directory not found".to_string())
            .and_then(|home| {
                std::env::current_exe()
                    .map_err(|e| e.to_string())
                    .and_then(|exe| task_notice::install(&home, &exe))
            });
        match result {
            Ok(path) => println!("{}", path.display()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if std::env::args_os().any(|arg| arg == "--enable-autostart") {
        std::process::exit(if set_autostart(true).is_ok() { 0 } else { 1 });
    }
    let preview = std::env::args_os().any(|arg| arg == "--preview-alerts");
    unsafe {
        let existing = FindWindowW(wide(&window_class()).as_ptr(), null());
        if !existing.is_null() {
            ShowWindow(existing, SW_SHOW);
            if preview {
                PostMessageW(existing, WM_COMMAND, CMD_ALERT_PREVIEW, 0);
            }
            SetForegroundWindow(existing);
            return;
        }
        SetProcessDPIAware();
        let mut token = 0usize;
        let startup = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: 0,
            SuppressExternalCodecs: 0,
        };
        if GdiplusStartup(&mut token, &startup, null_mut()) != 0 {
            return;
        }
        let class = wide(&window_class());
        let instance = GetModuleHandleW(null());
        let mut wc: WNDCLASSEXW = zeroed();
        wc.cbSize = size_of::<WNDCLASSEXW>() as u32;
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        wc.lpfnWndProc = Some(wndproc);
        wc.style = CS_DBLCLKS;
        wc.hCursor = LoadCursorW(null_mut(), IDC_ARROW);
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("QuotaDock").as_ptr(),
            WS_POPUP,
            0,
            0,
            230,
            40,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            GdiplusShutdown(token);
            return;
        }
        let created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        if created != 0 { /* The registered message is handled in the message loop below. */ }
        APP.with(|cell| {
            let mut app = cell.borrow_mut();
            app.dpi = GetDpiForWindow(hwnd) as f32 / 96.0;
            size_window(hwnd, &app, true);
            update_tray(hwnd, &mut app);
        });
        ShowWindow(hwnd, SW_SHOW);
        start_animation_clock();
        APP.with(|cell| {
            let app = cell.borrow();
            session_watch::set_enabled(app.task_source != task_notice::Source::Hook);
            session_watch::set_status_enabled(app.task_status_enabled || app.task_ribbon_enabled);
        });
        session_watch::start(data_dir());
        if preview {
            APP.with(|cell| {
                let mut app = cell.borrow_mut();
                enqueue_preview(&mut app);
                show_next_notice(hwnd, &mut app);
            });
        }
        request_refresh();
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == created {
                APP.with(|cell| {
                    let mut app = cell.borrow_mut();
                    app.tray_added = false;
                    update_tray(hwnd, &mut app);
                });
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        GdiplusShutdown(token);
    }
}
