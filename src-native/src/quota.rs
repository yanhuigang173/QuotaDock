use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::{
    env, fs,
    io::{BufRead, BufReader, Write},
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

pub(crate) fn codex_path() -> PathBuf {
    if let Some(path) = env::var_os("CODEX_CLI_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return path;
        }
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        let root = PathBuf::from(local).join("OpenAI/Codex/bin");
        let direct = root.join("codex.exe");
        if direct.is_file() {
            return direct;
        }
        if let Ok(entries) = fs::read_dir(root) {
            let mut candidates: Vec<_> = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("codex.exe"))
                .filter(|path| path.is_file())
                .collect();
            candidates.sort_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok());
            if let Some(path) = candidates.pop() {
                return path;
            }
        }
    }
    PathBuf::from("codex")
}

fn send(
    stdin: &mut impl Write,
    id: u64,
    method: &str,
    params: Option<Value>,
) -> Result<(), String> {
    let mut payload = json!({"id": id, "method": method});
    if let Some(params) = params {
        payload["params"] = params;
    }
    writeln!(stdin, "{payload}").map_err(|e| e.to_string())
}

fn receive(rx: &mpsc::Receiver<Value>, id: u64) -> Result<Value, String> {
    loop {
        let message = rx
            .recv_timeout(Duration::from_secs(12))
            .map_err(|e| format!("Codex request timed out or exited: {e}"))?;
        if message.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            return Err(error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Codex app-server error")
                .to_owned());
        }
        return message
            .get("result")
            .cloned()
            .ok_or_else(|| "Codex returned no result".into());
    }
}

pub fn read_quota() -> Result<Value, String> {
    let mut child = Command::new(codex_path())
        .args(["app-server", "--listen", "stdio://"])
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot start Codex CLI: {e}"))?;
    let stdout = child.stdout.take().ok_or("Cannot read Codex output")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(value);
            }
        }
    });
    let result = (|| {
        let mut stdin = child.stdin.take().ok_or("Cannot write to Codex")?;
        send(
            &mut stdin,
            1,
            "initialize",
            Some(json!({
                "clientInfo": {"name":"quotadock", "title":"QuotaDock", "version":env!("CARGO_PKG_VERSION")},
                "capabilities": null
            })),
        )?;
        receive(&rx, 1)?;
        send(&mut stdin, 2, "account/rateLimits/read", None)?;
        receive(&rx, 2)
    })();
    let _ = child.kill();
    let _ = child.wait();
    let response = result?;
    let snapshot = response
        .pointer("/rateLimitsByLimitId/codex")
        .ok_or("Codex did not return the codex rate-limit snapshot")?;
    normalize_snapshot(snapshot)
}

fn normalize_window(raw: Option<&Value>) -> Result<Option<Value>, String> {
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let used = raw
        .get("usedPercent")
        .and_then(Value::as_f64)
        .ok_or("Codex quota window is missing a numeric usedPercent")?;
    if !used.is_finite() {
        return Err("Codex quota usedPercent is invalid".into());
    }
    let used = used.round().clamp(0.0, 100.0) as i64;
    let resets_at = match raw.get("resetsAt").and_then(Value::as_i64) {
        Some(seconds) => Some(
            DateTime::<Utc>::from_timestamp(seconds, 0)
                .ok_or("Codex quota reset timestamp is invalid")?
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
        None => None,
    };
    Ok(Some(json!({
        "usedPercent": used,
        "remainingPercent": 100-used,
        "windowDurationMins": raw.get("windowDurationMins"),
        "resetsAt": resets_at
    })))
}

fn normalize_snapshot(raw: &Value) -> Result<Value, String> {
    let primary = normalize_window(raw.get("primary"))?;
    let secondary = normalize_window(raw.get("secondary"))?;
    let active = primary
        .as_ref()
        .or(secondary.as_ref())
        .ok_or("Codex snapshot has no usable quota window")?;
    Ok(json!({
        "limitId":raw.get("limitId").filter(|v| !v.is_null()).unwrap_or(&json!("codex")),
        "limitName":raw.get("limitName").filter(|v| !v.is_null()).unwrap_or(&json!("Codex")),
        "planType":raw.get("planType").filter(|v| !v.is_null()).unwrap_or(&json!("unknown")),
        "reachedType":raw.get("rateLimitReachedType"),
        "credits":raw.get("credits"),
        "remainingPercent":active["remainingPercent"],
        "usedPercent":active["usedPercent"],
        "resetsAt":active["resetsAt"],
        "primary":primary,
        "secondary":secondary,
        "fetchedAt":Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }))
}
