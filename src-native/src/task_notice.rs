use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    io::{self, Read, Seek, SeekFrom},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDNO, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST,
    MB_YESNO,
};

const MAX_INPUT: u64 = 4 * 1024 * 1024;
const MAX_AGE: u64 = 120;

pub const TEXT_LENGTHS: [usize; 5] = [40, 80, 140, 280, 500];
pub const TEXT_LINES: [u8; 5] = [1, 2, 3, 4, 6];

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextSettings {
    pub max_chars: usize,
    pub max_lines: u8,
}

impl Default for TextSettings {
    fn default() -> Self { Self { max_chars: 140, max_lines: 2 } }
}

impl TextSettings {
    pub fn normalized(mut self) -> Self {
        if !TEXT_LENGTHS.contains(&self.max_chars) { self.max_chars = 140; }
        if !TEXT_LINES.contains(&self.max_lines) { self.max_lines = 2; }
        self
    }

    pub fn summary(&self, text: &str) -> String {
        // Count Unicode characters, not UTF-8 bytes. Include the ellipsis in
        // the configured cap so a Chinese glyph is never split or overcounted.
        if text.chars().count() <= self.max_chars { return text.into(); }
        let mut output: String = text.chars().take(self.max_chars.saturating_sub(1)).collect();
        output.push('…');
        output
    }
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    #[default]
    Local,
    Hook,
    Both,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self { Self::Local => "本地记录", Self::Hook => "Hook", Self::Both => "本地记录 + Hook" }
    }

    pub fn accepts(self, source: &str) -> bool {
        match source {
            "localSession" => self != Self::Hook,
            "hook" => self != Self::Local,
            _ => false,
        }
    }
}

fn legacy_source() -> String { "hook".into() }

#[derive(Serialize, Deserialize)]
pub struct Event {
    id: String,
    created: u64,
    pub project: String,
    #[serde(default)]
    pub task_name: String,
    pub summary: String,
    #[serde(default = "legacy_source")]
    source: String,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hash(text: &str) -> String {
    let mut value = 0xcbf29ce484222325u64;
    for byte in text.bytes() {
        value = (value ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    format!("{value:016x}")
}

fn excerpt(text: &str, limit: usize) -> String {
    let plain = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let plain = plain.replace(['`', '*', '#'], "");
    let mut result: String = plain.chars().take(limit).collect();
    if plain.chars().count() > limit {
        result.push('…');
    }
    result
}

fn read_hook_input() -> Result<Value, String> {
    let mut reader = io::stdin().take(MAX_INPUT + 1);
    let parsed = serde_json::Deserializer::from_reader(&mut reader)
        .into_iter::<Value>()
        .next();
    if reader.limit() == 0 {
        return Err("Hook input is too large".into());
    }
    parsed
        .ok_or("Hook input is empty")?
        .map_err(|e| e.to_string())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Displays a deny-by-default approval prompt and returns the official
/// PermissionRequest hook decision. An invalid input declines to decide, so
/// Codex falls back to its normal approval flow.
pub fn permission_request(input: &Value) -> Option<bool> {
    if input["hook_event_name"] != "PermissionRequest" {
        return None;
    }
    let tool = input["tool_name"].as_str().unwrap_or("Codex 工具");
    let tool_input = &input["tool_input"];
    let detail = tool_input["description"]
        .as_str()
        .or_else(|| tool_input["command"].as_str())
        .map(str::to_owned)
        .or_else(|| {
            (!tool_input.is_null() && !tool_input.as_object().is_some_and(|args| args.is_empty()))
                .then(|| serde_json::to_string(tool_input).ok())
                .flatten()
        })
        .map(|value| excerpt(&value, 700))
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Codex 请求执行一项需要授权的操作。".into());
    let prompt = format!("工具：{tool}\n\n操作内容：\n{detail}\n\n点击“是”允许本次请求，点击“否”拒绝。未选择时将继续由 Codex 处理授权。\n\n允许仅对此次请求生效。默认选择：拒绝。",);
    let result = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            wide(&prompt).as_ptr(),
            wide("QuotaDock · Codex 权限请求").as_ptr(),
            MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
        )
    };
    match result {
        IDYES => Some(true),
        IDNO => Some(false),
        _ => None,
    }
}

pub fn read_permission_decision(dir: &Path) -> Option<bool> {
    let enabled = fs::read(dir.join("widget-settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|settings| settings["codexPermissionPrompts"].as_bool())
        .unwrap_or(true);
    if !enabled {
        return None;
    }
    let input = read_hook_input().ok()?;
    permission_request(&input)
}

pub fn preview() -> Event {
    Event {
        id: "preview".into(),
        created: now(),
        project: "QuotaDock".into(),
        task_name: "优化任务通知".into(),
        source: "preview".into(),
        summary: "已完成本轮修改。这里会显示 Codex 最后回复的摘要，通知结束后恢复原来的窗口形态。"
            .into(),
    }
}

pub fn notice(event: Event) -> crate::reset_alert::Notice {
    crate::reset_alert::Notice {
        title: format!("QuotaDock · {} · {} · 本轮结束", excerpt(&event.project, 28), if event.task_name.is_empty() { "当前任务".into() } else { excerpt(&event.task_name, 40) }),
        body: event.summary,
        important: false,
        task: true,
    }
}

// Executed as a short-lived, windowless child of Codex. Only a bounded excerpt
// is stored locally; the complete transcript is never copied or executed.
pub fn receive(dir: &Path) -> Result<(), String> {
    record_delivery(dir, "started", None);
    let result = receive_event(dir);
    if let Err(error) = &result {
        record_delivery(dir, "error", Some(error));
    }
    result
}

fn record_delivery(dir: &Path, stage: &str, detail: Option<&str>) {
    crate::save_json(
        dir.join("codex-hook-last.json"),
        &json!({"at":now(),"stage":stage,"detail":detail}),
    );
}

fn receive_event(dir: &Path) -> Result<(), String> {
    let input = read_hook_input()?;
    enqueue_input(dir, &input, "hook")
}

pub fn enqueue_input(dir: &Path, input: &Value, source: &str) -> Result<(), String> {
    if input["hook_event_name"] != "Stop" {
        return Ok(());
    }
    let cwd = input["cwd"].as_str().unwrap_or("");
    let project = cwd
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or("Codex");
    let message = input["last_assistant_message"].as_str().unwrap_or("");
    let session = input["session_id"].as_str().unwrap_or("");
    let turn = input["turn_id"].as_str().unwrap_or("");
    let id = if turn.is_empty() {
        hash(&format!("{session}\n{cwd}\n{message}"))
    } else {
        hash(&format!("{session}\n{turn}"))
    };
    let summary = excerpt(message, 500);
    let event = Event {
        id: id.clone(),
        created: now(),
        project: desktop_project_name(session, cwd).unwrap_or_else(|| excerpt(project, 80)),
        task_name: input["thread_name"].as_str().or_else(|| input["task_name"].as_str())
            .map(|name| excerpt(name, 80)).filter(|name| !name.is_empty())
            .or_else(|| session_name(session)).unwrap_or_default(),
        source: source.into(),
        summary: if summary.trim().is_empty() {
            "本轮已结束，打开 Codex 查看详情。".into()
        } else {
            summary
        },
    };
    let queue = dir.join("codex-notifications");
    fs::create_dir_all(&queue).map_err(|e| e.to_string())?;
    let name = format!("{}-{}-{}", event.created, std::process::id(), id);
    let temp = queue.join(format!("{name}.tmp"));
    let ready = queue.join(format!("{name}.json"));
    fs::write(
        &temp,
        serde_json::to_vec(&event).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(&temp, &ready).map_err(|e| e.to_string())?;
    record_delivery(dir, "queued", Some(source));
    Ok(())
}

fn normalized_root(path: &str) -> String {
    path.replace('/', "\\").trim_start_matches("\\\\?\\")
        .trim_end_matches('\\').to_lowercase()
}

// Resolve the desktop's user-defined project label, rather than the folder
// basename. This file is read only, and only when a completion arrives.
fn desktop_project_name(session: &str, cwd: &str) -> Option<String> {
    let file = fs::File::open(codex_home()?.join(".codex-global-state.json")).ok()?;
    const LIMIT: u64 = 8 * 1024 * 1024;
    if file.metadata().ok()?.len() > LIMIT { return None; }
    let state: Value = serde_json::from_reader(file.take(LIMIT)).ok()?;
    let projects = state["local-projects"].as_object()?;
    let label = |project: &Value| project["name"].as_str()
        .map(|name| excerpt(name, 80)).filter(|name| !name.is_empty());
    if let Some(assignment) = state["thread-project-assignments"].get(session) {
        if assignment["projectKind"] == "local" {
            if let Some(name) = assignment["projectId"].as_str()
                .and_then(|id| projects.get(id)).and_then(label) {
                return Some(name);
            }
        }
    }
    if state["projectless-thread-ids"].as_array()
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(session))) {
        return Some("无项目".into());
    }
    let cwd = normalized_root(cwd);
    if cwd.is_empty() { return None; }
    projects.values().filter_map(|project| {
        let name = label(project)?;
        let length = project["rootPaths"].as_array()?.iter()
            .filter_map(Value::as_str).map(normalized_root)
            .filter(|root| !root.is_empty() && (cwd == *root || cwd.starts_with(&format!("{root}\\"))))
            .map(|root| root.len()).max()?;
        Some((length, name))
    }).max_by_key(|(length, _)| *length).map(|(_, name)| name)
}

// The desktop keeps chat titles in its local index. Read a bounded tail only
// when a completion arrives; do not poll or load the conversation transcript.
fn session_name(session: &str) -> Option<String> {
    if session.is_empty() { return None; }
    let mut file = fs::File::open(codex_home()?.join("session_index.jsonl")).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(1024 * 1024))).ok()?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024).read_to_end(&mut bytes).ok()?;
    let tail = String::from_utf8_lossy(&bytes);
    tail.lines().rev().filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["id"].as_str() == Some(session))
        .and_then(|value| value["thread_name"].as_str().map(|name| excerpt(name, 80)))
        .filter(|name| !name.is_empty())
}

pub struct Inbox {
    seen: VecDeque<String>,
}

impl Inbox {
    pub fn load(dir: &Path) -> Self {
        let mut seen: VecDeque<String> = fs::read(dir.join("codex-notification-seen.json"))
            .ok()
            .filter(|bytes| bytes.len() < 32768)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        while seen.len() > 128 {
            seen.pop_front();
        }
        Self { seen }
    }

    pub fn drain(&mut self, dir: &Path, source: Source) -> Vec<Event> {
        let Ok(entries) = fs::read_dir(dir.join("codex-notifications")) else {
            return Vec::new();
        };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        let mut result = Vec::new();
        let mut changed = false;
        for path in files.into_iter().take(128) {
            let event = fs::metadata(&path)
                .ok()
                .filter(|m| m.len() <= 8192)
                .and_then(|_| fs::read(&path).ok())
                .and_then(|bytes| serde_json::from_slice::<Event>(&bytes).ok());
            // Remove before displaying so a failed deletion cannot trigger repeats.
            if fs::remove_file(&path).is_err() {
                continue;
            }
            let Some(event) = event else {
                continue;
            };
            // An unselected source must not reserve the shared deduplication ID.
            if !source.accepts(&event.source) { continue; }
            if now().saturating_sub(event.created) > MAX_AGE
                || event.created > now() + 10
                || self.seen.contains(&event.id)
            {
                continue;
            }
            self.seen.push_back(event.id.clone());
            while self.seen.len() > 128 {
                self.seen.pop_front();
            }
            changed = true;
            result.push(event);
        }
        if changed {
            crate::save_json(dir.join("codex-notification-seen.json"), &json!(self.seen));
        }
        result
    }
}

pub fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".codex")))
}

// Open an interactive review terminal. Trust is granted by the user inside
// Codex; no trust database, stored hash, or bypass flag is written here.
pub fn open_review(home: &Path) -> Result<(), String> {
    let cli = crate::quota::codex_path();
    let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
    let home = quote(&home.to_string_lossy());
    let cli = quote(&cli.to_string_lossy());
    let script = format!(
        "$Host.UI.RawUI.WindowTitle = 'QuotaDock - /hooks'; \
         Write-Host 'QuotaDock: type /hooks, review and trust both the Stop task notification and PermissionRequest approval hooks.' -ForegroundColor Cyan; \
         $env:CODEX_HOME = {home}; \
         & {cli} --no-daemon; \
         $reviewExit = $LASTEXITCODE; \
         if ($reviewExit -ne 0) {{ \
             Write-Host ('Codex review terminal failed to start or exited with an error. Exit code: ' + $reviewExit) -ForegroundColor Red; \
             Write-Host 'Hook trust has NOT been confirmed. Check the error above and retry when Codex is available.' -ForegroundColor Yellow \
         }} else {{ \
             Write-Host 'Terminal closed. If you trusted both QuotaDock hooks under /hooks, reopen your Codex desktop session.' -ForegroundColor Cyan \
         }}"
    );
    let mut command = Command::new("powershell.exe");
    if let Ok(executable) = std::env::current_exe() {
        if let Some(folder) = executable.parent() {
            command.current_dir(folder);
        }
    }
    command
        .args(["-NoLogo", "-NoProfile", "-NoExit", "-Command", &script])
        // This is deliberately an interactive window for the human trust step.
        .creation_flags(0x00000010)
        .spawn()
        .map_err(|e| format!("无法打开 Codex 审阅终端：{e}"))?;
    Ok(())
}

pub fn install(home: &Path, executable: &Path) -> Result<PathBuf, String> {
    let path = home.join("hooks.json");
    let mut root = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .map_err(|e| format!("Existing hooks.json: {e}"))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.to_string()),
    };
    let original = root.clone();
    let object = root.as_object_mut().ok_or("hooks.json must be an object")?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("hooks must be an object")?;
    for (event, status_message, arg) in [
        ("Stop", "QuotaDock 任务通知", "--codex-stop"),
        (
            "PermissionRequest",
            "QuotaDock 权限确认",
            "--codex-permission",
        ),
    ] {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("{event} must be an array"))?;
        for group in groups.iter_mut() {
            if let Some(handlers) = group["hooks"].as_array_mut() {
                handlers.retain(|h| h["statusMessage"] != status_message);
            }
        }
        groups.retain(|group| group["hooks"].as_array().is_none_or(|a| !a.is_empty()));
        let command = format!("\"{}\" {arg}", executable.display());
        let timeout = if event == "Stop" { 3 } else { 600 };
        groups.push(
            json!({"hooks":[{"type":"command","command":command,"commandWindows":command,
            "timeout":timeout,"async":event == "Stop","statusMessage":status_message}]}),
        );
    }
    if root == original {
        return Ok(path);
    }
    fs::create_dir_all(home).map_err(|e| e.to_string())?;
    if path.exists() {
        let backup = home.join(format!(
            "hooks.json.{}.bak",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::copy(&path, backup).map_err(|e| e.to_string())?;
    }
    let temp = home.join(format!("hooks.{}.tmp", std::process::id()));
    fs::write(
        &temp,
        serde_json::to_vec_pretty(&root).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // Windows rename cannot replace an existing destination; the backup above
    // preserves it if the final replacement fails.
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    fs::rename(&temp, &path).map_err(|e| e.to_string())?;
    Ok(path)
}
