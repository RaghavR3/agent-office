//! MCP connections across every agent. Claude Code keeps sign-ins in the keychain, shared by all
//! sessions, but each running session holds its own connection. The office reads which servers
//! each session has connected, failed or needs a sign-in for from its transcript, and reads the
//! server list from ~/.claude.json (names and addresses only; headers and secrets are skipped).

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How Claude Code names a server in its tool names: anything unusual becomes "_".
pub fn slug(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

/// One session's view of its MCP servers, keyed by slug.
#[derive(Default, Clone, Debug)]
pub struct SessionMcp {
    pub needs_sign_in: HashSet<String>,
    pub connected: HashSet<String>,
    pub failed: HashSet<String>,
    /// The last call to the server failed for a reason other than sign-in.
    pub last_error: HashMap<String, String>,
    /// Every tool call the session has made, for the papers piling up on its desk.
    pub tool_calls: u32,
    tool_server: HashMap<String, String>,
}

fn server_of(tool: &str) -> Option<(String, bool)> {
    let rest = tool.strip_prefix("mcp__")?;
    let (server, name) = rest.split_once("__")?;
    Some((server.to_string(), matches!(name, "authenticate" | "complete_authentication")))
}

/// Errors that mean the connection itself is broken, as opposed to a tool saying no (a plan
/// limit, a bad argument), which says nothing about the server.
fn connection_error(text: &str) -> bool {
    const SIGNS: [&str; 12] = [
        "POSTing to endpoint",
        "Connection closed",
        "ECONNREFUSED",
        "ECONNRESET",
        "ENOTFOUND",
        "ETIMEDOUT",
        "fetch failed",
        "timed out",
        "Just a moment",
        "not connected",
        "Bad Gateway",
        "Service Unavailable",
    ];
    SIGNS.iter().any(|s| text.contains(s))
}

fn short_error(text: &str) -> String {
    if text.contains("Just a moment") || text.contains("cf-chl") {
        return "Cloudflare blocked the last request".into();
    }
    let plain: String = text.split('<').next().unwrap_or(text).split_whitespace().collect::<Vec<_>>().join(" ");
    crate::brief::clip(plain.trim_start_matches("Error").trim_start_matches(':').trim(), 90)
}

impl SessionMcp {
    /// Folds in transcript lines. Only lines that can matter are parsed.
    pub fn feed(&mut self, chunk: &str) {
        for line in chunk.lines() {
            if !line.contains("\"isSidechain\":true") {
                self.tool_calls += line.matches("\"type\":\"tool_use\"").count() as u32;
            }
            let awaiting = !self.tool_server.is_empty() && line.contains("tool_result");
            if !(awaiting || line.contains("deferred_tools_delta") || line.contains("mcp__")) {
                continue;
            }
            let Ok(d) = serde_json::from_str::<Value>(line) else { continue };
            if d["isSidechain"] == Value::Bool(true) {
                continue;
            }
            let a = &d["attachment"];
            if a["type"] == "deferred_tools_delta" {
                for (list, add) in [(&a["addedNames"], true), (&a["readdedNames"], true), (&a["removedNames"], false)] {
                    for (server, auth) in list.as_array().into_iter().flatten().filter_map(|n| n.as_str()).filter_map(server_of) {
                        let set = if auth { &mut self.needs_sign_in } else { &mut self.connected };
                        if add {
                            set.insert(server);
                        } else {
                            set.remove(&server);
                        }
                    }
                }
                if let Some(f) = a["failedMcpServers"].as_array() {
                    self.failed = f.iter().filter_map(|n| n.as_str()).map(slug).collect();
                }
                continue;
            }
            for b in d["message"]["content"].as_array().into_iter().flatten() {
                match b["type"].as_str() {
                    Some("tool_use") => {
                        if let Some((server, false)) = b["name"].as_str().and_then(server_of) {
                            self.tool_server.insert(b["id"].as_str().unwrap_or_default().to_string(), server);
                        }
                    }
                    Some("tool_result") => {
                        let Some(server) = b["tool_use_id"].as_str().and_then(|id| self.tool_server.remove(id)) else { continue };
                        if b["is_error"] == Value::Bool(true) {
                            let text = match &b["content"] {
                                Value::String(s) => s.clone(),
                                c => c.as_array().into_iter().flatten().filter_map(|x| x["text"].as_str()).collect::<Vec<_>>().join(" "),
                            };
                            let lower = text.to_lowercase();
                            if lower.contains("401") || lower.contains("unauthorized") || lower.contains("authenticat") {
                                self.needs_sign_in.insert(server.clone());
                            } else if connection_error(&text) {
                                self.last_error.insert(server, short_error(&text));
                            }
                        } else {
                            self.last_error.remove(&server);
                            self.needs_sign_in.remove(&server);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Reads only what a transcript has added since last time, so long sessions stay cheap.
#[derive(Default)]
pub struct Reader {
    sessions: HashMap<std::path::PathBuf, (u64, SessionMcp)>,
}

impl Reader {
    pub fn read(&mut self, path: &Path) -> Option<&SessionMcp> {
        let len = std::fs::metadata(path).ok()?.len();
        let (offset, state) = self.sessions.entry(path.to_path_buf()).or_default();
        if len < *offset {
            *offset = 0;
            *state = SessionMcp::default();
        }
        if len > *offset {
            let mut f = std::fs::File::open(path).ok()?;
            f.seek(SeekFrom::Start(*offset)).ok()?;
            let mut buf = Vec::with_capacity((len - *offset) as usize);
            f.take(len - *offset).read_to_end(&mut buf).ok()?;
            // Stop at the last full line; the rest is picked up next time.
            let end = buf.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
            state.feed(&String::from_utf8_lossy(&buf[..end]));
            *offset += end as u64;
        }
        Some(state)
    }

    pub fn keep(&mut self, paths: &HashSet<std::path::PathBuf>) {
        self.sessions.retain(|p, _| paths.contains(p));
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    /// The name `claude mcp login` takes.
    pub name: String,
    pub label: String,
    /// "you" (all your sessions), "claude.ai" (a connector on your Claude account), or "folder".
    pub scope: String,
    /// connected | needs_sign_in | failed | error | idle
    pub status: String,
    pub detail: Option<String>,
    /// Agents whose session still needs this server signed in.
    pub waiting: Vec<String>,
    /// A room where it loads, for servers set up for one folder: sign-in has to run there.
    pub room: Option<String>,
}

/// Servers from ~/.claude.json: yours, and claude.ai connectors you've used.
pub fn configured() -> Vec<(String, String)> {
    let path = dirs::home_dir().unwrap_or_default().join(".claude.json");
    let Ok(raw) = std::fs::read_to_string(path) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else { return vec![] };
    let mut out: Vec<(String, String)> = v["mcpServers"].as_object().into_iter().flatten().map(|(k, _)| (k.clone(), "you".into())).collect();
    for n in v["claudeAiMcpEverConnected"].as_array().into_iter().flatten().filter_map(|n| n.as_str()) {
        out.push((n.to_string(), "claude.ai".into()));
    }
    // Folder-level servers, so a session's slug can be turned back into the name to sign in with.
    for (_, p) in v["projects"].as_object().into_iter().flatten() {
        for (k, _) in p["mcpServers"].as_object().into_iter().flatten() {
            if !out.iter().any(|(n, _)| n == k) {
                out.push((k.clone(), "folder".into()));
            }
        }
    }
    out
}

fn label(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("claude.ai ") {
        return rest.to_string();
    }
    // plugin:<plugin>:<server>
    name.strip_prefix("plugin:").and_then(|r| r.rsplit(':').next()).unwrap_or(name).to_string()
}

/// A server only seen in sessions: plugin servers come back to their plugin:<plugin>:<name> form.
fn unknown(sl: &str) -> (String, String) {
    match sl.strip_prefix("plugin_").and_then(|r| r.split_once('_')) {
        Some((plugin, server)) => (format!("plugin:{plugin}:{server}"), "plugin".into()),
        None => (sl.to_string(), "other".into()),
    }
}

/// One line per server across all agents. Each agent comes with its name, its session, and the
/// room it works in.
pub fn summarise(known: &[(String, String)], agents: &[(String, &SessionMcp, String)]) -> Vec<Server> {
    let by_slug: HashMap<String, &(String, String)> = known.iter().map(|k| (slug(&k.0), k)).collect();
    let mut slugs: Vec<String> = known.iter().filter(|(_, scope)| scope != "folder").map(|(n, _)| slug(n)).collect();
    for (_, s, _) in agents {
        for x in s.connected.iter().chain(&s.failed).chain(s.last_error.keys()) {
            if !slugs.contains(x) {
                slugs.push(x.clone());
            }
        }
        // A sign-in prompt only counts for servers you've set up; claude.ai lists every connector.
        for x in &s.needs_sign_in {
            if !slugs.contains(x) && by_slug.get(x).is_some_and(|k| k.1 == "folder") {
                slugs.push(x.clone());
            }
        }
    }
    let mut out: Vec<Server> = slugs
        .into_iter()
        .map(|sl| {
            let (name, scope) = by_slug.get(&sl).map(|k| (k.0.clone(), k.1.clone())).unwrap_or_else(|| unknown(&sl));
            let needing: Vec<&(String, &SessionMcp, String)> = agents.iter().filter(|(_, s, _)| s.needs_sign_in.contains(&sl)).collect();
            let room = needing.first().map(|(_, _, r)| r.clone()).filter(|_| scope == "folder");
            let mut waiting: Vec<String> = needing.iter().map(|(n, _, _)| n.clone()).collect();
            let failed = agents.iter().any(|(_, s, _)| s.failed.contains(&sl));
            let error = agents.iter().find_map(|(_, s, _)| s.last_error.get(&sl).cloned());
            let connected = agents.iter().any(|(_, s, _)| s.connected.contains(&sl));
            if waiting.is_empty() {
                // For a failing server, the agents who ran into it.
                waiting = agents
                    .iter()
                    .filter(|(_, s, _)| s.failed.contains(&sl) || s.last_error.contains_key(&sl))
                    .map(|(n, _, _)| n.clone())
                    .collect();
            }
            let (status, detail) = if !needing.is_empty() {
                ("needs_sign_in", None)
            } else if failed {
                ("failed", Some("Couldn't connect".to_string()))
            } else if let Some(e) = error {
                ("error", Some(e))
            } else if connected {
                ("connected", None)
            } else {
                ("idle", None)
            };
            Server { label: label(&name), name, scope, status: status.into(), detail, waiting, room }
        })
        .collect();
    let rank = |s: &Server| match s.status.as_str() {
        "needs_sign_in" => 0,
        "failed" | "error" => 1,
        "connected" => 2,
        _ => 3,
    };
    out.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.label.to_lowercase().cmp(&b.label.to_lowercase())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_sign_ins_and_failures() {
        let mut s = SessionMcp::default();
        s.feed(r#"{"type":"attachment","attachment":{"type":"deferred_tools_delta","addedNames":["mcp__team-tools__search","mcp__claude_ai_Gmail__authenticate","mcp__linear__authenticate"],"removedNames":[],"failedMcpServers":["posthog"]}}"#);
        assert!(s.connected.contains("team-tools") && s.needs_sign_in.contains("linear") && s.failed.contains("posthog"));
        s.feed(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__team-tools__search","input":{}}]}}"#);
        s.feed(r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"Error POSTing to endpoint: <!DOCTYPE html><title>Just a moment...</title>"}]}}"#);
        assert_eq!(s.last_error.get("team-tools").unwrap(), "Cloudflare blocked the last request");
        s.feed(r#"{"type":"attachment","attachment":{"type":"deferred_tools_delta","addedNames":["mcp__linear__list_issues"],"removedNames":["mcp__linear__authenticate"]}}"#);
        assert!(!s.needs_sign_in.contains("linear"));
        assert_eq!(s.tool_calls, 1, "the one call made, counted once");

        let known = vec![("team-tools".to_string(), "you".to_string()), ("linear".to_string(), "you".to_string())];
        let list = summarise(&known, &[("Edie".into(), &s, "room-1".into())]);
        let names: Vec<(&str, &str)> = list.iter().map(|x| (x.name.as_str(), x.status.as_str())).collect();
        // Gmail was never set up, so its sign-in prompt is left out.
        assert_eq!(names, [("posthog", "failed"), ("team-tools", "error"), ("linear", "connected")]);
    }
}

#[cfg(test)]
mod live_check {
    /// `cargo test live_mcp -- --ignored --nocapture` prints what the office would show today.
    #[test]
    #[ignore]
    fn live_mcp() {
        let raw = std::fs::read_to_string(dirs::home_dir().unwrap().join("Library/Application Support/orca/agent-hooks/last-status.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let mut reader = super::Reader::default();
        let mut views = vec![];
        for (k, e) in v["entries"].as_object().unwrap() {
            if let Some(p) = e["providerSession"]["transcriptPath"].as_str() {
                if let Some(s) = reader.read(std::path::Path::new(p)) {
                    views.push((k[k.len() - 6..].to_string(), s.clone()));
                }
            }
        }
        let refs: Vec<(String, &super::SessionMcp, String)> = views.iter().map(|(n, s)| (n.clone(), s, String::new())).collect();
        for s in super::summarise(&super::configured(), &refs) {
            println!("{:<28} {:<10} {:<14} {:?} {:?}", s.label, s.scope, s.status, s.detail, s.waiting);
        }
    }
}
