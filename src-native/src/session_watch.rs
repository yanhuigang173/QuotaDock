//! Compatibility with local desktop runtimes that list Stop hooks but do not
//! dispatch them. Watch append-only session completion records, without polling
//! transcripts or interpreting tool output as a completion signal.
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    ptr::{null, null_mut},
    sync::{atomic::{AtomicBool, AtomicI64, Ordering}, Mutex, OnceLock},
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, INVALID_HANDLE_VALUE},
    Storage::FileSystem::*,
    UI::WindowsAndMessaging::PostMessageW,
};

const CHUNK: u64 = 1024 * 1024;
const MAX_LINE: usize = 4 * 1024 * 1024;
static ENABLED: AtomicBool = AtomicBool::new(true);
static ENABLED_AT: AtomicI64 = AtomicI64::new(0);
static STATUS_ENABLED: AtomicBool = AtomicBool::new(true);
static STATUS_RESCAN: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskState {
    #[default]
    Idle,
    Running,
    Completed,
    Interrupted,
}

impl TaskState {
    pub fn color(self) -> u32 {
        match self {
            Self::Idle => 0xff626d7a,
            Self::Running => 0xff4f9dff,
            Self::Completed => 0xff9bea58,
            Self::Interrupted => 0xffffab62,
        }
    }
    pub fn label(self) -> &'static str {
        match self { Self::Idle => "无任务", Self::Running => "任务进行中", Self::Completed => "任务已完成", Self::Interrupted => "任务已中断" }
    }
}

struct Activity {
    turn: String,
    state: TaskState,
    at: DateTime<Utc>,
}

#[derive(Default)]
struct Tracker {
    sessions: HashMap<String, Activity>,
    terminal: Option<(TaskState, std::time::Instant)>,
}
static TRACKER: OnceLock<Mutex<Tracker>> = OnceLock::new();

pub fn set_status_enabled(enabled: bool) {
    let previous = STATUS_ENABLED.swap(enabled, Ordering::AcqRel);
    if enabled && !previous { STATUS_RESCAN.store(true, Ordering::Release); }
    if !enabled {
        *TRACKER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()) = Tracker::default();
    }
}

pub fn task_state() -> TaskState {
    let tracker = TRACKER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if tracker.sessions.values().any(|activity| activity.state == TaskState::Running) {
        TaskState::Running
    } else if let Some((state, _)) = tracker.terminal.filter(|(state, at)| {
        let seconds = if *state == TaskState::Completed { 180 } else { 60 };
        at.elapsed() < Duration::from_secs(seconds)
    }) {
        state
    } else {
        TaskState::Idle
    }
}

fn record_activity(session: &str, turn: &str, state: TaskState, at: DateTime<Utc>) {
    let mut tracker = TRACKER.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if !STATUS_ENABLED.load(Ordering::Acquire) { return; }
    if let Some(previous) = tracker.sessions.get(session) {
        // A late completion from an earlier turn must not clear a newer task.
        if previous.at > at || (state != TaskState::Running && previous.state == TaskState::Running && previous.turn != turn) {
            return;
        }
        if previous.turn == turn && previous.state == state { return; }
    }
    if tracker.sessions.len() >= 256 && !tracker.sessions.contains_key(session) {
        if let Some(oldest) = tracker.sessions.iter()
            .filter(|(_, activity)| activity.state != TaskState::Running)
            .min_by_key(|(_, activity)| activity.at).map(|(id, _)| id.clone()) {
            tracker.sessions.remove(&oldest);
        } else { return; }
    }
    tracker.sessions.insert(session.into(), Activity { turn: turn.into(), state, at });
    if state != TaskState::Running { tracker.terminal = Some((state, std::time::Instant::now())); }
    drop(tracker);
    let hwnd = crate::MAIN_HWND.load(Ordering::Acquire) as HWND;
    if !hwnd.is_null() { unsafe { PostMessageW(hwnd, crate::TASK_MESSAGE, 0, 0); } }
}

fn observe_activity(line: &[u8], session: &str, since: DateTime<Utc>) -> Option<Activity> {
    if !STATUS_ENABLED.load(Ordering::Acquire) { return None; }
    if ![b"\"task_started\"".as_slice(), b"\"task_complete\"", b"\"turn_aborted\""]
        .iter().any(|marker| line.windows(marker.len()).any(|bytes| bytes == *marker)) { return None; }
    let value = serde_json::from_slice::<Value>(line).ok()?;
    if value["type"] != "event_msg" { return None; }
    let payload = &value["payload"];
    let state = match payload["type"].as_str() {
        Some("task_started") => TaskState::Running,
        Some("task_complete") => TaskState::Completed,
        Some("turn_aborted") => TaskState::Interrupted,
        _ => return None,
    };
    let turn = payload["turn_id"].as_str()?;
    let at = value["timestamp"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok()).map(|at| at.with_timezone(&Utc))?;
    if at > Utc::now() + chrono::Duration::seconds(10) { return None; }
    if at >= since { record_activity(session, turn, state, at); }
    Some(Activity { turn: turn.into(), state, at })
}

pub fn set_enabled(enabled: bool) {
    if enabled && !ENABLED.load(Ordering::Acquire) {
        ENABLED_AT.store(Utc::now().timestamp_millis(), Ordering::Release);
    }
    ENABLED.store(enabled, Ordering::Release);
}

struct Session {
    id: String,
    cwd: String,
    offset: u64,
    pending: Vec<u8>,
    skip_first: bool,
    initial_activity: bool,
}

impl Session {
    fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let mut header = String::new();
        BufReader::new(file.take(64 * 1024))
            .read_line(&mut header)
            .ok()?;
        let value: Value = serde_json::from_str(&header).ok()?;
        if value["type"] != "session_meta" {
            return None;
        }
        let meta = &value["payload"];
        // Subagent sessions use an object-valued source. Only notify main chats.
        if !meta["source"].is_string() || meta.get("parent_thread_id").is_some_and(|v| !v.is_null())
        {
            return None;
        }
        let length = fs::metadata(path).ok()?.len();
        let offset = length.saturating_sub(CHUNK);
        Some(Self {
            id: meta["id"].as_str()?.into(),
            cwd: meta["cwd"].as_str()?.into(),
            offset,
            pending: Vec::new(),
            skip_first: offset != 0,
            initial_activity: true,
        })
    }

    fn read(&mut self, path: &Path, since: DateTime<Utc>, activity_since: DateTime<Utc>) -> Vec<Value> {
        let mut result = Vec::new();
        let Ok(mut file) = File::open(path) else {
            return result;
        };
        let Ok(metadata) = file.metadata() else {
            return result;
        };
        if metadata.len() < self.offset {
            self.offset = 0;
            self.pending.clear();
            self.skip_first = false;
        }
        if metadata.len() == self.offset {
            return result;
        }
        // Bound recovery after a large write; completions are small records at
        // the tail. A skipped prefix is never parsed as a complete JSON line.
        if metadata.len() - self.offset > 8 * CHUNK {
            self.offset = metadata.len().saturating_sub(CHUNK);
            self.pending.clear();
            self.skip_first = self.offset != 0;
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return result;
        }
        let mut latest_activity = None;
        loop {
            let mut bytes = Vec::new();
            if file.by_ref().take(CHUNK).read_to_end(&mut bytes).is_err() || bytes.is_empty() {
                break;
            }
            self.offset += bytes.len() as u64;
            self.pending.extend(bytes);
            let mut start = 0;
            for end in 0..self.pending.len() {
                if self.pending[end] != b'\n' {
                    continue;
                }
                let line = &self.pending[start..end];
                if self.skip_first {
                    self.skip_first = false;
                } else {
                    if let Some(activity) = observe_activity(line, &self.id, activity_since) {
                        if self.initial_activity { latest_activity = Some(activity); }
                    }
                    if ENABLED.load(Ordering::Acquire) {
                        if let Some(input) = completion(line, &self.id, &self.cwd, since) { result.push(input); }
                    }
                }
                start = end + 1;
            }
            if start != 0 {
                self.pending.drain(..start);
            }
            if self.pending.len() < 16 * 1024 && self.pending.capacity() > 128 * 1024 {
                self.pending.shrink_to(16 * 1024);
            }
            if self.pending.len() > MAX_LINE {
                self.pending.clear();
                self.skip_first = true;
            }
        }
        if self.initial_activity {
            // Recover a turn that started before QuotaDock, only after this
            // session changes again. Old completed/aborted turns are not replayed.
            if let Some(activity) = latest_activity {
                if activity.state == TaskState::Running && activity.at < activity_since {
                    record_activity(&self.id, &activity.turn, activity.state, activity.at);
                }
            }
            self.initial_activity = false;
        }
        result
    }
}

fn completion(line: &[u8], session: &str, cwd: &str, since: DateTime<Utc>) -> Option<Value> {
    let marker = b"\"task_complete\"";
    if !line.windows(marker.len()).any(|bytes| bytes == marker) {
        return None;
    }
    let value: Value = serde_json::from_slice(line).ok()?;
    if value["type"] != "event_msg" || value["payload"]["type"] != "task_complete" {
        return None;
    }
    let at = DateTime::parse_from_rfc3339(value["timestamp"].as_str()?)
        .ok()?
        .with_timezone(&Utc);
    if at < since
        || (Utc::now() - at).num_seconds() > 120
        || at > Utc::now() + chrono::Duration::seconds(10)
    {
        return None;
    }
    let payload = &value["payload"];
    Some(
        json!({"hook_event_name":"Stop","session_id":session,"turn_id":payload["turn_id"].as_str()?,
        "cwd":cwd,"last_assistant_message":payload["last_agent_message"].as_str().unwrap_or("")}),
    )
}

pub fn start(data: PathBuf) {
    let Some(home) = crate::task_notice::codex_home() else {
        return;
    };
    thread::spawn(move || {
        let root = home.join("sessions");
        let since = Utc::now();
        // Opening a directory for change notification does not read historical
        // chats. Existing files are opened only after a new filesystem event.
        let path = crate::wide(&root.to_string_lossy());
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            crate::save_json(data.join("codex-local-receiver.json"), &json!({"state":"unavailable","started":since.to_rfc3339(),"sessions":root}));
            return;
        }
        crate::save_json(data.join("codex-local-receiver.json"), &json!({"state":"watching","started":since.to_rfc3339(),"sessions":root}));
        let mut sessions: HashMap<PathBuf, Session> = HashMap::new();
        // ReadDirectoryChangesW requires DWORD alignment.
        let mut storage = vec![0u32; 16 * 1024];
        let buffer = unsafe { std::slice::from_raw_parts_mut(storage.as_mut_ptr() as *mut u8, storage.len() * 4) };
        loop {
            let mut count = 0;
            let ok = unsafe {
                ReadDirectoryChangesW(
                    handle,
                    buffer.as_mut_ptr() as *mut _,
                    buffer.len() as u32,
                    1,
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_SIZE
                        | FILE_NOTIFY_CHANGE_LAST_WRITE,
                    &mut count,
                    null_mut(),
                    None,
                )
            };
            if ok == 0 {
                break;
            }
            if !ENABLED.load(Ordering::Acquire) && !STATUS_ENABLED.load(Ordering::Acquire) {
                sessions.clear();
                continue;
            }
            if STATUS_RESCAN.swap(false, Ordering::AcqRel) { sessions.clear(); }
            let notice_since = DateTime::from_timestamp_millis(ENABLED_AT.load(Ordering::Acquire))
                .map(|enabled_at| enabled_at.max(since)).unwrap_or(since);
            let mut at = 0usize;
            while at + 12 <= count as usize {
                let next = u32::from_le_bytes(buffer[at..at + 4].try_into().unwrap()) as usize;
                let length =
                    u32::from_le_bytes(buffer[at + 8..at + 12].try_into().unwrap()) as usize;
                if length % 2 != 0 || at + 12 + length > count as usize {
                    break;
                }
                let name: Vec<u16> = buffer[at + 12..at + 12 + length]
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let relative = PathBuf::from(String::from_utf16_lossy(&name));
                if relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
                    && relative.extension().is_some_and(|e| e == "jsonl")
                {
                    let file = root.join(relative);
                    if !sessions.contains_key(&file) {
                        if sessions.len() >= 128 {
                            sessions.clear();
                        }
                        if let Some(session) = Session::open(&file) {
                            sessions.insert(file.clone(), session);
                        }
                    }
                    if let Some(session) = sessions.get_mut(&file) {
                        for input in session.read(&file, notice_since, since) {
                            if crate::task_notice::enqueue_input(&data, &input, "localSession")
                                .is_ok()
                            {
                                let hwnd = crate::MAIN_HWND.load(Ordering::Acquire) as HWND;
                                if !hwnd.is_null() {
                                    unsafe {
                                        PostMessageW(hwnd, crate::TASK_MESSAGE, 0, 0);
                                    }
                                }
                            }
                        }
                    }
                }
                if next == 0 {
                    break;
                }
                if next < 12 {
                    break;
                }
                at += next;
            }
            // Yield after notification bursts without polling the filesystem.
            thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            CloseHandle(handle);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_new_complete_turns() {
        let since = Utc::now() - chrono::Duration::seconds(1);
        let mut value = json!({"type":"event_msg","timestamp":Utc::now().to_rfc3339(),
            "payload":{"type":"task_complete","turn_id":"t1","last_agent_message":"1"}});
        let parse = |v: &Value| {
            completion(
                &serde_json::to_vec(v).unwrap(),
                "session",
                "D:/Project",
                since,
            )
        };
        let parsed = parse(&value).unwrap();
        assert_eq!(parsed["last_assistant_message"], "1");
        assert_eq!(parsed["session_id"], "session");
        value["timestamp"] = json!((since - chrono::Duration::seconds(1)).to_rfc3339());
        assert!(parse(&value).is_none());
        value["timestamp"] = json!(Utc::now().to_rfc3339());
        value["type"] = json!("response_item");
        assert!(parse(&value).is_none());
        value["type"] = json!("event_msg");
        value["payload"]["type"] = json!("task_started");
        assert!(parse(&value).is_none());
    }
}
