//! Agent Office's own Claude Code hook, so it can see what agents are doing without relying on
//! the terminal app's status feed.
//!
//! Each event overwrites one small file per Claude process in ~/.agent-office/sessions, named by
//! the process id, so a session keeps its file across /clear and the folder never grows past
//! one file per running session. Only sessions inside Orca or cmux are recorded. The hook is
//! plain sh, always exits 0, and is marked so it can be found and removed again.

use crate::status::PaneStatus;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

const MARK: &str = "agent-office-hook";
const EVENTS: [&str; 9] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Stop",
    "StopFailure",
];

/// $PPID is the Claude process that ran the hook. Files are private to you (umask 077), and
/// the file's own modified time says when the event happened.
const COMMAND: &str = concat!(
    ": agent-office-hook; ",
    r#"[ -n "${CMUX_SURFACE_ID:-}${ORCA_TERMINAL_HANDLE:-}" ] || exit 0; "#,
    r#"umask 077; d="$HOME/.agent-office/sessions"; [ -d "$d" ] || mkdir -p "$d" || exit 0; p=$(cat); "#,
    r#"printf '{"pid":%s,"surface":"%s","workspace":"%s","orca":"%s","event":%s}\n' "$PPID" "${CMUX_SURFACE_ID:-}" "${CMUX_WORKSPACE_ID:-}" "${ORCA_TERMINAL_HANDLE:-}" "${p:-null}" > "$d/$PPID.$$" && mv -f "$d/$PPID.$$" "$d/$PPID.json"; "#,
    "exit 0"
);

fn settings_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude/settings.json")
}

pub fn sessions_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".agent-office/sessions")
}

fn read_settings() -> Result<Value> {
    match std::fs::read_to_string(settings_path()) {
        Ok(s) => serde_json::from_str(&s)
            .map_err(|e| anyhow!("~/.claude/settings.json isn't valid JSON ({e}), so it was left alone")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.into()),
    }
}

fn write_settings(v: &Value) -> Result<()> {
    let path = settings_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.agent-office-tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(v)? + "\n")?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn has_ours(groups: &Value) -> bool {
    groups.as_array().is_some_and(|gs| {
        gs.iter().any(|g| g["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| h["command"].as_str().is_some_and(|c| c.contains(MARK)))))
    })
}

pub fn installed() -> bool {
    read_settings().is_ok_and(|v| EVENTS.iter().all(|ev| has_ours(&v["hooks"][ev])))
}

/// Adds our hook to each event, after whatever hooks are already there. Keeps a one-time backup.
pub fn install() -> Result<()> {
    let mut v = read_settings()?;
    let path = settings_path();
    let backup = path.with_extension("json.before-agent-office");
    if path.exists() && !backup.exists() {
        std::fs::copy(&path, &backup)?;
    }
    add_hooks(&mut v)?;
    write_settings(&v)
}

/// Takes our hook back out, leaving everyone else's exactly as they were.
pub fn uninstall() -> Result<()> {
    let mut v = read_settings()?;
    remove_hooks(&mut v);
    write_settings(&v)
}

fn add_hooks(v: &mut Value) -> Result<()> {
    if !v.is_object() {
        return Err(anyhow!("~/.claude/settings.json isn't a JSON object, so it was left alone"));
    }
    if !v["hooks"].is_object() {
        v["hooks"] = json!({});
    }
    for ev in EVENTS {
        if has_ours(&v["hooks"][ev]) {
            continue;
        }
        let mut list = v["hooks"][ev].as_array().cloned().unwrap_or_default();
        list.push(json!({ "hooks": [{ "type": "command", "command": COMMAND, "timeout": 5 }] }));
        v["hooks"][ev] = Value::Array(list);
    }
    Ok(())
}

fn remove_hooks(v: &mut Value) {
    let Some(hooks) = v["hooks"].as_object_mut() else { return };
    for ev in EVENTS {
        let Some(groups) = hooks.get_mut(ev).and_then(Value::as_array_mut) else { continue };
        for g in groups.iter_mut() {
            if let Some(hs) = g["hooks"].as_array_mut() {
                hs.retain(|h| !h["command"].as_str().is_some_and(|c| c.contains(MARK)));
            }
        }
        groups.retain(|g| g["hooks"].as_array().is_none_or(|hs| !hs.is_empty()));
        if groups.is_empty() {
            hooks.remove(ev);
        }
    }
}

/// The latest thing one Claude process did, as our hook recorded it.
#[derive(Clone, Debug, Default)]
pub struct Session {
    pub session_id: String,
    pub cwd: String,
    pub transcript: Option<String>,
    pub event: String,
    pub reason: Option<String>,
    pub tool: Option<String>,
    pub tool_input: Option<String>,
    pub prompt: Option<String>,
    pub last_message: Option<String>,
    /// cmux's id for the terminal, when the session runs in cmux.
    pub surface: Option<String>,
    pub workspace: Option<String>,
    /// Orca's terminal handle, when the session runs in Orca.
    pub orca: Option<String>,
    pub at_ms: i64,
}

impl Session {
    fn parse(v: &Value, at_ms: i64) -> Option<Session> {
        let e = &v["event"];
        let s = |x: &Value| x.as_str().filter(|s| !s.is_empty()).map(str::to_string);
        let input = &e["tool_input"];
        Some(Session {
            session_id: s(&e["session_id"])?,
            cwd: s(&e["cwd"]).unwrap_or_default(),
            transcript: s(&e["transcript_path"]),
            event: s(&e["hook_event_name"]).unwrap_or_default(),
            reason: s(&e["reason"]),
            tool: s(&e["tool_name"]),
            tool_input: ["file_path", "command", "description", "pattern", "url", "query", "skill"]
                .iter()
                .find_map(|k| s(&input[*k])),
            prompt: s(&e["prompt"]),
            last_message: s(&e["last_assistant_message"]),
            surface: s(&v["surface"]),
            workspace: s(&v["workspace"]),
            orca: s(&v["orca"]),
            at_ms,
        })
    }

    pub fn ended(&self) -> bool {
        self.event == "SessionEnd" && self.reason.as_deref() != Some("clear")
    }

    /// In the same shape as Orca's status feed, so the rest of the office reads both alike.
    pub fn status(&self) -> PaneStatus {
        let state = match self.event.as_str() {
            "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => "working",
            "PermissionRequest" => "waiting",
            "Stop" | "StopFailure" => "done",
            _ => "idle",
        };
        let tool_event = matches!(self.event.as_str(), "PreToolUse" | "PostToolUse" | "PermissionRequest");
        PaneStatus {
            state: state.into(),
            prompt: self.prompt.clone(),
            last_message: if self.event == "StopFailure" {
                Some("Claude stopped because of an error. Open the terminal to see what went wrong.".into())
            } else {
                self.last_message.clone()
            },
            tool_name: self.tool.clone().filter(|_| tool_event),
            tool_input: self.tool_input.clone().filter(|_| tool_event),
            transcript_path: self.transcript.clone(),
            updated_at: self.at_ms,
        }
    }
}

fn alive(pid: i32) -> bool {
    // Signal 0 only checks the process exists; EPERM means it exists but isn't ours.
    pid > 1 && (unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

/// Our hook's files, re-read only when they change.
#[derive(Default)]
pub struct HookCache {
    files: HashMap<PathBuf, (SystemTime, Option<Session>)>,
}

impl HookCache {
    pub fn refresh(&mut self) {
        let Ok(dir) = std::fs::read_dir(sessions_dir()) else {
            self.files.clear();
            return;
        };
        let mut seen = vec![];
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let pid: i32 = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse().ok()).unwrap_or(0);
            if !alive(pid) {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) else { continue };
            seen.push(path.clone());
            if self.files.get(&path).is_some_and(|(m, _)| *m == mtime) {
                continue;
            }
            let at_ms = mtime.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
            let session = std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|v| Session::parse(&v, at_ms));
            self.files.insert(path, (mtime, session));
        }
        self.files.retain(|p, _| seen.contains(p));
    }

    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.files.values().filter_map(|(_, s)| s.as_ref())
    }

    /// Whether a session we started has drawn its prompt yet.
    pub fn started(&self, session_id: &str) -> bool {
        self.sessions().any(|s| s.session_id == session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn run_hook(home: &std::path::Path, env: &[(&str, &str)], payload: &str) {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(COMMAND).env("HOME", home).env_remove("CMUX_SURFACE_ID").env_remove("ORCA_TERMINAL_HANDLE");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.stdin(std::process::Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn the_hook_keeps_one_file_per_claude_process() {
        let home = std::env::temp_dir().join(format!("ao-hook-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let stop = r#"{"session_id":"abc-123","cwd":"/tmp/x","hook_event_name":"Stop","last_assistant_message":"All done"}"#;
        run_hook(&home, &[], stop);
        let dir = home.join(".agent-office/sessions");
        assert!(!dir.exists(), "sessions outside Orca and cmux aren't recorded");

        run_hook(&home, &[("CMUX_SURFACE_ID", "surf-1")], r#"{"session_id":"abc-123","hook_event_name":"UserPromptSubmit","prompt":"Fix it"}"#);
        run_hook(&home, &[("CMUX_SURFACE_ID", "surf-1")], stop);
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(files[0].path()).unwrap()).unwrap();
        let s = Session::parse(&v, 0).unwrap();
        assert_eq!(s.surface.as_deref(), Some("surf-1"));
        assert_eq!(files[0].file_name().to_str().unwrap(), format!("{}.json", std::process::id()));
        let st = s.status();
        assert_eq!((st.state.as_str(), st.last_message.as_deref()), ("done", Some("All done")));
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn install_and_remove_leave_other_hooks_alone() {
        let theirs = json!({ "type": "command", "command": "orca-hook" });
        let original = json!({ "model": "opus", "hooks": { "Stop": [{ "hooks": [theirs] }] } });
        let mut v = original.clone();
        add_hooks(&mut v).unwrap();
        add_hooks(&mut v).unwrap();
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2, "installing twice adds it once");
        assert!(EVENTS.iter().all(|ev| has_ours(&v["hooks"][ev])));
        remove_hooks(&mut v);
        assert_eq!(v, original);
    }

    #[test]
    fn clear_is_not_the_end() {
        let s = |reason: &str| Session { event: "SessionEnd".into(), reason: Some(reason.into()), ..Default::default() };
        assert!(!s("clear").ended());
        assert!(s("prompt_input_exit").ended());
    }
}
