//! Read-only view of a Claude Code session transcript, condensed for the "Show work" drawer.

use crate::brief::{clip, strip_fenced};
use anyhow::{anyhow, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum WorkEvent {
    /// `queued`: sent while the agent was busy, and not read yet.
    Prompt {
        text: String,
        ts: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        queued: bool,
    },
    Said { text: String, ts: String },
    Tool { name: String, detail: String, ts: String },
}

/// How much of the end of a transcript we read; long sessions run to tens of MB.
const TAIL_BYTES: u64 = 2_000_000;

pub fn projects_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude/projects")
}

/// Claude Code names a project folder by replacing every non-alphanumeric char of the cwd with '-'.
pub fn path_for(cwd: &str, session_id: &str) -> PathBuf {
    let slug: String = cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    projects_dir().join(slug).join(format!("{session_id}.jsonl"))
}

fn is_allowed(path: &Path) -> bool {
    let Ok(real) = path.canonicalize() else { return false };
    if cfg!(test) && std::env::temp_dir().canonicalize().is_ok_and(|t| real.starts_with(t)) {
        return real.extension().is_some_and(|e| e == "jsonl");
    }
    let Ok(root) = projects_dir().canonicalize() else { return false };
    real.starts_with(root) && real.extension().is_some_and(|e| e == "jsonl")
}

/// The end of a transcript, after checking it really is one of Claude Code's.
pub fn read_tail(path: &Path) -> Result<String> {
    read_last(path, TAIL_BYTES)
}

pub fn read_last(path: &Path, bytes_wanted: u64) -> Result<String> {
    if !is_allowed(path) {
        return Err(anyhow!("That transcript isn't in ~/.claude/projects"));
    }
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len > bytes_wanted {
        f.seek(SeekFrom::Start(len - bytes_wanted))?;
    }
    let mut bytes = vec![];
    f.read_to_end(&mut bytes)?;
    // Lossy because the seek can land mid-character; the first partial line is skipped anyway.
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The last few turns of a session. Reads back further when a run of huge tool output pushes
/// your last message out of the usual window.
pub fn read_work(path: &Path, max_turns: usize) -> Result<Vec<WorkEvent>> {
    let len = std::fs::metadata(path)?.len();
    let mut window = TAIL_BYTES;
    loop {
        let mut events = parse_events(&read_last(path, window)?, false);
        let prompts = events.iter().filter(|e| matches!(e, WorkEvent::Prompt { .. })).count();
        if prompts > 0 || window >= len || window >= 32_000_000 {
            keep_turns(&mut events, max_turns);
            return Ok(events);
        }
        window *= 4;
    }
}

/// A helper's own thread: what it was asked, then what it did.
pub fn read_helper_work(path: &Path) -> Result<Vec<WorkEvent>> {
    let len = std::fs::metadata(path)?.len();
    let mut events = parse_events(&read_last(path, TAIL_BYTES)?, true);
    // Its brief is the first line, which a long thread pushes out of the window.
    if len > TAIL_BYTES && !matches!(events.first(), Some(WorkEvent::Prompt { .. })) {
        let mut head = vec![];
        std::fs::File::open(path)?.take(2_000_000).read_to_end(&mut head)?;
        let head = String::from_utf8_lossy(&head);
        if let Some(first) = parse_events(head.lines().next().unwrap_or_default(), true).into_iter().next() {
            events.insert(0, first);
        }
    }
    Ok(events)
}

/// What you typed, from a user message. Orca pastes long messages inside <pasted_content> tags
/// and slash commands are stored as tags; anything else in tags is Claude Code's own plumbing.
pub fn human_text(raw: &str) -> Option<String> {
    static PASTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"</?pasted_content[^>]*>").unwrap());
    let between = |t: &str, open: &str, close: &str| t.split_once(open).and_then(|(_, r)| r.split_once(close)).map(|(x, _)| x.trim().to_string());
    let t = raw.trim();
    if let Some(name) = between(t, "<command-name>", "</command-name>") {
        let name = if name.starts_with('/') { name } else { format!("/{name}") };
        let args = between(t, "<command-args>", "</command-args>").unwrap_or_default();
        return Some(if args.is_empty() { name } else { format!("{name} {args}") });
    }
    let t = PASTED.replace_all(t, "");
    let t = t.trim();
    if t.is_empty() || t.starts_with('<') || t.starts_with("[Request interrupted") {
        return None;
    }
    Some(t.to_string())
}

fn parse_events(buf: &str, side: bool) -> Vec<WorkEvent> {
    let mut events = vec![];
    // Messages sent while the agent was busy wait in a queue: (text, typed by you).
    let mut queue: Vec<(String, bool)> = vec![];
    let prompt = |events: &mut Vec<WorkEvent>, text: String, ts: String| {
        let repeat = events.iter().rev().find_map(|e| match e {
            WorkEvent::Prompt { text: t, .. } => Some(t == &text),
            _ => None,
        });
        if repeat != Some(true) {
            events.push(WorkEvent::Prompt { text, ts, queued: false });
        }
    };
    for line in buf.lines() {
        let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
        if !side && d["isSidechain"] == Value::Bool(true) {
            continue;
        }
        let ts = d["timestamp"].as_str().unwrap_or_default().to_string();
        match d["type"].as_str() {
            Some("user") if d["isMeta"] != Value::Bool(true) => {
                if let Some(text) = user_text(&d["message"]["content"]) {
                    queue.retain(|(t, _)| t != &text);
                    prompt(&mut events, text, ts);
                }
            }
            Some("attachment") if d["attachment"]["type"] == "queued_command" => {
                if let Some(text) = d["attachment"]["prompt"].as_str().and_then(human_text) {
                    queue.retain(|(t, _)| t != &text);
                    prompt(&mut events, text, ts);
                }
            }
            Some("queue-operation") => {
                let raw = d["content"].as_str().unwrap_or_default();
                match d["operation"].as_str() {
                    Some("enqueue") => queue.push(human_text(raw).map(|t| (t, true)).unwrap_or((raw.to_string(), false))),
                    Some("remove") => {
                        let text = human_text(raw).unwrap_or(raw.to_string());
                        if let Some(i) = queue.iter().position(|(t, _)| t == &text) {
                            queue.remove(i);
                        }
                    }
                    Some("dequeue") if !queue.is_empty() => {
                        queue.remove(0);
                    }
                    // Up arrow takes everything queued back into the message box.
                    Some("popAll") => queue.clear(),
                    _ => {}
                }
            }
            Some("assistant") => {
                for b in d["message"]["content"].as_array().into_iter().flatten() {
                    match b["type"].as_str() {
                        Some("text") => {
                            let t = strip_fenced(b["text"].as_str().unwrap_or(""), "brief");
                            if !t.is_empty() {
                                events.push(WorkEvent::Said { text: t, ts: ts.clone() });
                            }
                        }
                        Some("tool_use") => {
                            let name = b["name"].as_str().unwrap_or("tool").to_string();
                            let detail = tool_detail(&name, &b["input"]);
                            events.push(WorkEvent::Tool { name: pretty_tool(&name), detail, ts: ts.clone() });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    for (text, _) in queue.into_iter().filter(|(_, yours)| *yours) {
        events.push(WorkEvent::Prompt { text, ts: String::new(), queued: true });
    }
    events
}

fn keep_turns(events: &mut Vec<WorkEvent>, max_turns: usize) {
    let prompt_idx: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, WorkEvent::Prompt { .. }))
        .map(|(i, _)| i)
        .collect();
    if prompt_idx.len() > max_turns {
        let cut = prompt_idx[prompt_idx.len() - max_turns];
        events.drain(..cut);
    }
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct Latest {
    pub prompt: Option<String>,
    /// The turn was cut short (Esc), so Claude is waiting even though no stop event came.
    pub interrupted: bool,
}

/// The last thing you asked, and whether the turn after it was cut short.
pub fn latest(raw: &str) -> Latest {
    let mut out = Latest::default();
    for line in raw.lines() {
        let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
        if d["isSidechain"] == Value::Bool(true) || d["isMeta"] == Value::Bool(true) {
            continue;
        }
        match d["type"].as_str() {
            Some("user") => {
                let c = &d["message"]["content"];
                let texts: Vec<&str> = match c {
                    Value::String(s) => vec![s.as_str()],
                    _ => c.as_array().into_iter().flatten().filter_map(|b| b["text"].as_str()).collect(),
                };
                if texts.iter().any(|t| t.trim_start().starts_with("[Request interrupted")) {
                    out.interrupted = true;
                } else if let Some(t) = user_text(c) {
                    out.prompt = Some(t);
                    out.interrupted = false;
                }
            }
            Some("attachment") if d["attachment"]["type"] == "queued_command" => {
                if let Some(t) = d["attachment"]["prompt"].as_str().and_then(human_text) {
                    out.prompt = Some(t);
                }
            }
            Some("assistant") => out.interrupted = false,
            _ => {}
        }
    }
    out
}

fn user_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    human_text(&text)
}

pub fn pretty_tool(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("mcp__") {
        let mut parts = rest.splitn(2, "__");
        let server = parts.next().unwrap_or("");
        let tool = parts.next().unwrap_or(rest);
        let server = server.trim_start_matches("claude_ai_").replace(['_', '-'], " ");
        return format!("{} · {}", server.trim(), tool.replace('_', " "));
    }
    name.to_string()
}

pub fn tool_detail(name: &str, input: &Value) -> String {
    let pick = |k: &str| input[k].as_str().map(str::to_string);
    let home = dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default();
    let d = match name {
        "Bash" => pick("description").or_else(|| pick("command")),
        "Read" | "Write" | "Edit" | "NotebookEdit" => pick("file_path"),
        "Grep" | "Glob" => pick("pattern"),
        "WebFetch" => pick("url"),
        "WebSearch" => pick("query"),
        "Agent" | "Task" => pick("description"),
        "Skill" => pick("skill"),
        _ => input
            .as_object()
            .and_then(|o| o.values().find_map(|v| v.as_str().map(str::to_string))),
    }
    .unwrap_or_default();
    clip(&d.replace(&home, "~").replace('\n', " "), 120)
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Helper {
    /// Claude Code's id for it (its thread is `subagents/agent-<id>.jsonl`); empty when unknown.
    pub id: String,
    /// What the agent asked its helper to do, in its own words.
    pub task: String,
    pub kind: String,
    pub running: bool,
    pub started_at: i64,
    pub updated_at: i64,
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

/// Milliseconds since 1970 for a transcript timestamp like "2026-11-22T11:31:08.945Z".
fn iso_ms(s: &str) -> Option<i64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let t = time.trim_end_matches('Z');
    let mut parts = t.split(':');
    let (hh, mm) = (parts.next()?.parse::<i64>().ok()?, parts.next()?.parse::<i64>().ok()?);
    let secs: f64 = parts.next()?.parse().ok()?;
    // Days from the civil calendar (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + hh * 3_600 + mm * 60) as f64 * 1000.0 + secs * 1000.0) as i64)
}

/// Subagents the agent has started and not yet heard back from. Background ones answer at
/// once with "Async agent launched", then report back later in a task notification.
pub fn running_helpers(raw: &str) -> Vec<Helper> {
    use std::collections::{HashMap, HashSet};
    // Anything started over three hours ago is long gone, even if its report never showed up.
    let cutoff = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
        - 3 * 3_600_000;
    let tag = |text: &str, open: &str, close: &str| -> Vec<String> {
        text.split(open).skip(1).filter_map(|p| p.split(close).next()).map(|x| x.trim().to_string()).collect()
    };
    let mut started: Vec<(String, Helper)> = vec![];
    let mut finished: HashSet<String> = HashSet::new();
    let mut background: HashSet<String> = HashSet::new();
    let mut agent_of: HashMap<String, String> = HashMap::new();
    let mut since_turn_end: HashSet<String> = HashSet::new();
    // Claude Code notes at each turn's end how many background agents are still going.
    let mut pending_at_turn_end: Option<usize> = None;
    for line in raw.lines() {
        let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
        if d["isSidechain"] == Value::Bool(true) {
            continue;
        }
        if d["type"] == "system" && d["subtype"] == "turn_duration" {
            pending_at_turn_end = Some(d["pendingBackgroundAgentCount"].as_u64().unwrap_or(0) as usize);
            since_turn_end.clear();
            continue;
        }
        let content = &d["message"]["content"];
        // A helper's report is queued the moment it finishes, before (or instead of) being delivered,
        // and names it by tool call, by agent id, or both.
        let notice = if d["type"] == "queue-operation" { d["content"].as_str().unwrap_or_default().to_string() } else { result_text(content) };
        if notice.contains("<task-notification>") || notice.contains("<agent-message") {
            finished.extend(tag(&notice, "<tool-use-id>", "</tool-use-id>"));
            for agent in tag(&notice, "<task-id>", "</task-id>").into_iter().chain(tag(&notice, "<agent-message from=\"", "\"")) {
                if let Some(id) = agent_of.get(&agent) {
                    finished.insert(id.clone());
                }
            }
        }
        for b in content.as_array().into_iter().flatten() {
            match b["type"].as_str() {
                Some("tool_use")
                    if matches!(b["name"].as_str(), Some("Agent" | "Task"))
                        && d["timestamp"].as_str().and_then(iso_ms).is_none_or(|t| t > cutoff) =>
                {
                    let input = &b["input"];
                    let task = input["description"].as_str().or(input["prompt"].as_str()).unwrap_or("Helping out");
                    let kind = input["subagent_type"].as_str().unwrap_or("helper");
                    let id = b["id"].as_str().unwrap_or_default().to_string();
                    since_turn_end.insert(id.clone());
                    let at = d["timestamp"].as_str().and_then(iso_ms).unwrap_or(0);
                    let helper = Helper { id: String::new(), task: clip(task.lines().next().unwrap_or(task), 70), kind: kind.to_string(), running: true, started_at: at, updated_at: at };
                    started.push((id, helper));
                }
                Some("tool_result") => {
                    let text = result_text(&b["content"]);
                    let Some(id) = b["tool_use_id"].as_str() else { continue };
                    if text.trim_start().starts_with("Async agent launched") {
                        background.insert(id.to_string());
                        if let Some(agent) = text.split("agentId: ").nth(1).and_then(|r| r.split_whitespace().next()) {
                            agent_of.insert(agent.trim_end_matches(['.', ')', ',']).to_string(), id.to_string());
                        }
                    } else {
                        finished.insert(id.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    let mut running: Vec<(String, Helper)> = started.into_iter().filter(|(id, _)| !finished.contains(id)).collect();
    if let Some(n) = pending_at_turn_end {
        // Since the last turn ended, only new helpers and at most `n` background ones can be going.
        let (new, old): (Vec<_>, Vec<_>) = running.into_iter().partition(|(id, _)| since_turn_end.contains(id));
        let old: Vec<_> = old.into_iter().filter(|(id, _)| background.contains(id)).collect();
        let keep = old.len().saturating_sub(n);
        running = old.into_iter().skip(keep).chain(new).collect();
    }
    running.into_iter().map(|(_, h)| h).collect()
}

/// Each helper's thread lives in `<session>/subagents/agent-<id>.jsonl`, next to a small
/// `.meta.json` saying what it was asked. Threads are only re-read when they grow.
#[derive(Default)]
pub struct Subagents {
    seen: HashMap<PathBuf, (u64, Helper, bool)>,
}

const HOUR: i64 = 3_600_000;

fn ms(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The folder a session keeps its helpers' threads in.
fn subagents_dir(transcript: &Path) -> PathBuf {
    transcript.with_extension("").join("subagents")
}

/// The thread of one of the session's helpers, if the id is one Claude Code would make.
pub fn helper_path(transcript: &Path, id: &str) -> Option<PathBuf> {
    let ok = !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric());
    let p = subagents_dir(transcript).join(format!("agent-{id}.jsonl"));
    (ok && p.is_file()).then_some(p)
}

/// Whether a helper has finished: its last word is a reply that ends its turn.
fn helper_done(path: &Path) -> bool {
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut window = 256_000;
    loop {
        let Ok(raw) = read_last(path, window) else { return false };
        let last = raw
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|d| matches!(d["type"].as_str(), Some("user" | "assistant")));
        if let Some(d) = last {
            return d["type"] == "assistant" && d["message"]["stop_reason"] == "end_turn";
        }
        if window >= len || window >= 8_000_000 {
            return false;
        }
        window *= 4;
    }
}

impl Subagents {
    /// Every helper the session has started, newest first. None when the session has no
    /// helpers folder (older Claude Code), so the caller can read the transcript instead.
    pub fn list(&mut self, transcript: &Path, tail: &str) -> Option<Vec<Helper>> {
        let entries = std::fs::read_dir(subagents_dir(transcript)).ok()?;
        let now = ms(Ok(std::time::SystemTime::now()));
        let mut out: Vec<(Helper, bool)> = vec![];
        for e in entries.flatten() {
            let p = e.path();
            let Some(id) = p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("agent-")?.strip_suffix(".jsonl")).map(str::to_string) else {
                continue;
            };
            let Ok(m) = e.metadata() else { continue };
            let (len, updated_at) = (m.len(), ms(m.modified()));
            let entry = match self.seen.get(&p).filter(|(l, ..)| *l == len) {
                Some((_, h, bg)) => (h.clone(), *bg),
                None => {
                    let meta: Value = std::fs::read_to_string(p.with_extension("meta.json"))
                        .ok()
                        .and_then(|t| serde_json::from_str(&t).ok())
                        .unwrap_or(Value::Null);
                    let task = meta["description"].as_str().unwrap_or("Helping out");
                    let h = Helper {
                        id,
                        task: clip(task.lines().next().unwrap_or(task), 70),
                        kind: meta["agentType"].as_str().unwrap_or("helper").to_string(),
                        running: !helper_done(&p),
                        started_at: ms(m.created()),
                        updated_at,
                    };
                    let bg = meta["requestShape"] == "background";
                    self.seen.insert(p.clone(), (len, h.clone(), bg));
                    (h, bg)
                }
            };
            out.push(entry);
        }
        // Gone quiet for three hours: whatever it was waiting on, it isn't coming back.
        for (h, _) in out.iter_mut() {
            h.running &= now - h.updated_at < 3 * HOUR;
        }
        // At each turn's end Claude Code notes how many background helpers are still going.
        // Ones started before then can't outnumber that, and no foreground one is still going.
        let turn_end = tail
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|d| d["type"] == "system" && d["subtype"] == "turn_duration" && d["isSidechain"] != Value::Bool(true))
            .map(|d| (d["timestamp"].as_str().and_then(iso_ms).unwrap_or(0), d["pendingBackgroundAgentCount"].as_u64().unwrap_or(0) as usize));
        if let Some((ended, pending)) = turn_end {
            let mut older: Vec<&mut (Helper, bool)> = out.iter_mut().filter(|(h, _)| h.running && h.started_at < ended).collect();
            older.sort_by_key(|(h, _)| -h.updated_at);
            for (i, (h, bg)) in older.into_iter().enumerate() {
                h.running = *bg && i < pending;
            }
        }
        let mut list: Vec<Helper> = out.into_iter().map(|(h, _)| h).collect();
        list.sort_by_key(|h| (!h.running, -h.started_at));
        Some(list)
    }

    pub fn keep(&mut self, transcripts: &[PathBuf]) {
        self.seen.retain(|p, _| transcripts.iter().any(|t| p.starts_with(subagents_dir(t))));
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AskOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// A question Claude put up with its built-in question box (AskUserQuestion).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AskQuestion {
    pub question: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default)]
    pub options: Vec<AskOption>,
}

/// The questions Claude is waiting on you to answer, if its question box is still up.
pub fn open_question(raw: &str) -> Option<Vec<AskQuestion>> {
    let mut open: Option<(String, Vec<AskQuestion>)> = None;
    for line in raw.lines() {
        let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
        if d["isSidechain"] == Value::Bool(true) {
            continue;
        }
        for b in d["message"]["content"].as_array().into_iter().flatten() {
            match b["type"].as_str() {
                Some("tool_use") if b["name"] == "AskUserQuestion" => {
                    let qs: Vec<AskQuestion> = serde_json::from_value(b["input"]["questions"].clone()).unwrap_or_default();
                    open = Some((b["id"].as_str().unwrap_or_default().to_string(), qs));
                }
                Some("tool_result") if open.as_ref().is_some_and(|(id, _)| b["tool_use_id"] == id.as_str()) => open = None,
                _ => {}
            }
        }
    }
    open.map(|(_, qs)| qs).filter(|qs| !qs.is_empty())
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    fn prompts(events: &[WorkEvent]) -> Vec<(String, bool)> {
        events
            .iter()
            .filter_map(|e| match e {
                WorkEvent::Prompt { text, queued, .. } => Some((text.clone(), *queued)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn reads_what_you_typed_however_it_was_stored() {
        assert_eq!(human_text("\n\n<pasted_content id=\"a658\">\nWhich library?\n</pasted_content id=\"a658\">").as_deref(), Some("Which library?"));
        assert_eq!(
            human_text("<command-message>x:review</command-message>\n<command-name>/x:review</command-name>\n<command-args>PR 12</command-args>").as_deref(),
            Some("/x:review PR 12")
        );
        assert_eq!(human_text("<task-notification>\n<task-id>a1</task-id></task-notification>"), None);
        assert_eq!(human_text("[Request interrupted by user]"), None);
    }

    #[test]
    fn follows_messages_sent_while_busy() {
        let line = |v: serde_json::Value| v.to_string();
        let log = [
            line(serde_json::json!({"type":"user","message":{"role":"user","content":"Build the page"},"timestamp":"t1"})),
            line(serde_json::json!({"type":"queue-operation","operation":"enqueue","content":"use the arrow too"})),
            line(serde_json::json!({"type":"queue-operation","operation":"enqueue","content":"<task-notification><task-id>a1</task-id></task-notification>"})),
        ]
        .join("\n");
        // Waiting its turn.
        assert_eq!(prompts(&parse_events(&log, false)), [("Build the page".into(), false), ("use the arrow too".into(), true)]);
        // Read mid-turn.
        let read = format!(
            "{log}\n{}\n{}",
            line(serde_json::json!({"type":"queue-operation","operation":"remove","content":"use the arrow too"})),
            line(serde_json::json!({"type":"attachment","attachment":{"type":"queued_command","prompt":"use the arrow too","origin":{"kind":"human"}},"timestamp":"t2"}))
        );
        assert_eq!(prompts(&parse_events(&read, false)), [("Build the page".into(), false), ("use the arrow too".into(), false)]);
        // Taken back with the up arrow.
        let back = format!("{log}\n{}", line(serde_json::json!({"type":"queue-operation","operation":"popAll","content":"use the arrow too"})));
        assert_eq!(prompts(&parse_events(&back, false)), [("Build the page".into(), false)]);
    }

    #[test]
    fn tells_running_helpers_from_finished_ones() {
        let dir = std::env::temp_dir().join(format!("ao-subagents-{}", uuid::Uuid::new_v4()));
        let sub = dir.join("s1/subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let transcript = dir.join("s1.jsonl");
        std::fs::write(&transcript, "").unwrap();
        let thread = |id: &str, last: &str, desc: &str| {
            let first = serde_json::json!({"type":"user","isSidechain":true,"message":{"content":desc}});
            std::fs::write(sub.join(format!("agent-{id}.jsonl")), format!("{first}\n{last}\n")).unwrap();
            let meta = serde_json::json!({"agentType":"general-purpose","description":desc,"requestShape":"background"});
            std::fs::write(sub.join(format!("agent-{id}.meta.json")), meta.to_string()).unwrap();
        };
        thread("a1", r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"Done"}]}}"#, "Convert the hero");
        thread("a2", r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash","input":{}}]}}"#, "Rebrand sign-up");
        let list = Subagents::default().list(&transcript, "").unwrap();
        assert_eq!(list.iter().map(|h| (h.id.as_str(), h.running)).collect::<Vec<_>>(), [("a2", true), ("a1", false)]);
        assert_eq!(list[0].task, "Rebrand sign-up");
        // The turn ended after both started, with none still going: nothing is running.
        let tail = r#"{"type":"system","subtype":"turn_duration","timestamp":"2099-01-01T00:00:00.000Z"}"#;
        assert!(Subagents::default().list(&transcript, tail).unwrap().iter().all(|h| !h.running));
        let work = read_helper_work(&helper_path(&transcript, "a1").unwrap()).unwrap();
        assert_eq!(prompts(&work), [("Convert the hero".into(), false)]);
        assert!(helper_path(&transcript, "../x").is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn background_helpers_run_until_they_report_back() {
        let a = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"description":"Research channels"}}]}}"#;
        let launched = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"Async agent launched successfully. agentId: a1"}]}]}}"#;
        let done = r#"{"type":"user","message":{"content":"<task-notification>\n<task-id>a1</task-id>\n<tool-use-id>t1</tool-use-id>\n<status>completed</status>"}}"#;
        assert_eq!(running_helpers(&format!("{a}\n{launched}")).len(), 1);
        assert!(running_helpers(&format!("{a}\n{launched}\n{done}")).is_empty());
        // Queued but never delivered still means it finished.
        let queued = r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>a1</task-id>\n<tool-use-id>t1</tool-use-id>\n<status>completed</status>"}"#;
        assert!(running_helpers(&format!("{a}\n{launched}\n{queued}")).is_empty());
    }

    #[test]
    fn trusts_the_turn_end_count_and_agent_ids() {
        let a = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"description":"Publish to Notion"}}]}}"#;
        let launched = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Async agent launched successfully. agentId: ac3d02 (internal ID)"}]}}"#;
        let pending = r#"{"type":"system","subtype":"turn_duration","pendingBackgroundAgentCount":1}"#;
        let quiet = r#"{"type":"system","subtype":"turn_duration"}"#;
        let handback = r#"{"type":"queue-operation","operation":"enqueue","content":"<agent-message from=\"ac3d02\"> [Subagent hand-back] done"}"#;
        assert_eq!(running_helpers(&[a, launched, pending].join("\n")).len(), 1);
        assert!(running_helpers(&[a, launched, pending, handback].join("\n")).is_empty(), "named by agent id");
        assert!(running_helpers(&[a, launched, pending, quiet].join("\n")).is_empty(), "nothing pending at the last turn's end");
    }

    /// `AO_TRANSCRIPTS=a.jsonl:b.jsonl cargo test live_helpers -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_helpers() {
        for p in std::env::var("AO_TRANSCRIPTS").unwrap_or_default().split(':').filter(|p| !p.is_empty()) {
            let raw = std::fs::read_to_string(p).unwrap();
            println!("{p}: {:?}", running_helpers(&raw).iter().map(|h| h.task.clone()).collect::<Vec<_>>());
        }
    }

    #[test]
    fn reads_transcript_times() {
        assert_eq!(iso_ms("1970-01-02T00:00:01.500Z"), Some(86_401_500));
        assert_eq!(iso_ms("2026-11-23T02:23:39.431Z"), Some(1_795_400_619_431));
    }

    #[test]
    fn finds_an_unanswered_question() {
        let ask = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"q1","name":"AskUserQuestion","input":{"questions":[{"question":"Which price?","header":"Pricing","multiSelect":false,"options":[{"label":"Flat","description":"One price"}]}]}}]}}"#;
        let answered = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"q1","content":"Flat"}]}}"#;
        let qs = open_question(ask).unwrap();
        assert_eq!((qs[0].header.as_str(), qs[0].options[0].label.as_str()), ("Pricing", "Flat"));
        assert!(open_question(&format!("{ask}\n{answered}")).is_none());
    }

    #[test]
    fn notices_an_interrupted_turn() {
        let you = r#"{"type":"user","message":{"content":"Fix the sync"}}"#;
        let tool = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#;
        let stop = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"no"},{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#;
        let l = latest(&[you, tool, stop].join("\n"));
        assert_eq!(l, Latest { prompt: Some("Fix the sync".into()), interrupted: true });
        assert!(!latest(&[you, tool].join("\n")).interrupted);
    }

    #[test]
    fn finds_helpers_still_running() {
        let a = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"description":"Search the billing code","subagent_type":"Explore"}},{"type":"tool_use","id":"t2","name":"Agent","input":{"description":"Check the tests"}}]}}"#;
        let r = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"done"}]}}"#;
        let h = running_helpers(&format!("{a}\n{r}"));
        assert_eq!(h.iter().map(|h| (h.task.as_str(), h.kind.as_str())).collect::<Vec<_>>(), [("Search the billing code", "Explore")]);
    }
}

#[cfg(test)]
mod live_check {
    /// `AO_TRANSCRIPT=<path> cargo test live_session -- --ignored --nocapture` shows what the card would.
    #[test]
    #[ignore]
    fn live_session() {
        let path = std::path::PathBuf::from(std::env::var("AO_TRANSCRIPT").unwrap());
        let t = std::time::Instant::now();
        let work = super::read_work(&path, 4).unwrap();
        let prompts: Vec<_> = work.iter().filter_map(|e| match e { super::WorkEvent::Prompt { text, queued, .. } => Some((super::clip(text, 70), *queued)), _ => None }).collect();
        println!("read_work {:?}: {} events, prompts {:?}", t.elapsed(), work.len(), prompts);
        let tail = super::read_last(&path, 400_000).unwrap();
        println!("latest prompt: {:?}", super::latest(&tail).prompt.map(|p| super::clip(&p, 70)));
        let t = std::time::Instant::now();
        let mut s = super::Subagents::default();
        for h in s.list(&path, &tail).unwrap() {
            println!("  {} {} | {} ({})", if h.running { "RUNNING" } else { "done   " }, h.id, h.task, h.kind);
        }
        println!("first list {:?}", t.elapsed());
        let t = std::time::Instant::now();
        s.list(&path, &tail);
        println!("cached list {:?}", t.elapsed());
    }
}
