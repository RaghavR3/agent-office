//! Client for Orca's local runtime socket: one newline-delimited JSON request per connection.
//! The auth token is read from Orca's own metadata file and never leaves this process.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeMeta {
    transports: Vec<Transport>,
    auth_token: String,
}

#[derive(Deserialize)]
struct Transport {
    kind: String,
    endpoint: String,
}

pub fn orca_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library/Application Support/orca")
}

pub async fn call(method: &str, params: Value, timeout: Duration) -> Result<Value> {
    let raw = tokio::fs::read_to_string(orca_dir().join("orca-runtime.json"))
        .await
        .context("Orca isn't running")?;
    let meta: RuntimeMeta = serde_json::from_str(&raw).context("Orca's runtime file is unreadable")?;
    let endpoint = meta
        .transports
        .iter()
        .find(|t| t.kind == "unix")
        .map(|t| t.endpoint.clone())
        .ok_or_else(|| anyhow!("Orca exposes no local socket"))?;

    let id = uuid::Uuid::new_v4().to_string();
    let mut req = json!({ "id": id, "authToken": meta.auth_token, "method": method, "params": params });
    if method == "terminal.send" && req["params"]["agentPrompt"] == json!(true) {
        // Orca treats agent prompts as durable mutations keyed by this id.
        req["orchestrationRequestId"] = json!(uuid::Uuid::new_v4().to_string());
    }
    let line = format!("{req}\n");

    let fut = async {
        let stream = UnixStream::connect(&endpoint)
            .await
            .context("Couldn't reach Orca")?;
        let (r, mut w) = stream.into_split();
        w.write_all(line.as_bytes()).await?;
        let mut lines = BufReader::new(r).lines();
        while let Some(l) = lines.next_line().await? {
            if l.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(&l)?;
            if v.get("_keepalive").is_some() {
                continue;
            }
            if v["id"] != json!(id) {
                return Err(anyhow!("Orca answered a different request"));
            }
            if v["ok"] == json!(true) {
                return Ok(v["result"].clone());
            }
            let msg = v["error"]["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| v["error"].to_string());
            return Err(anyhow!("Orca: {msg}"));
        }
        Err(anyhow!("Orca closed the connection"))
    };
    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| anyhow!("Orca took too long to answer {method}"))?
}

const QUICK: Duration = Duration::from_secs(8);

pub async fn list_repos() -> Result<Value> {
    call("repo.list", json!({}), QUICK).await
}
pub async fn list_worktrees() -> Result<Value> {
    call("worktree.list", json!({ "limit": 10000 }), QUICK).await
}
pub async fn list_folders() -> Result<Value> {
    call("folderWorkspace.list", json!({}), QUICK).await
}
pub async fn list_terminals() -> Result<Value> {
    call("terminal.list", json!({ "limit": 500, "includeVisualLayouts": false }), QUICK).await
}

pub async fn create_terminal(worktree: &str, command: &str, title: &str) -> Result<String> {
    let r = call(
        "terminal.create",
        json!({
            "worktree": worktree,
            "command": command,
            "title": title,
            "focus": false,
            "rendererBacked": true,
            "activate": false
        }),
        Duration::from_secs(20),
    )
    .await?;
    r["terminal"]["handle"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Orca didn't return a terminal handle"))
}

pub async fn wait_ready(handle: &str, timeout_ms: u64) -> Result<()> {
    call(
        "terminal.wait",
        json!({ "terminal": handle, "for": "tui-idle", "timeoutMs": timeout_ms }),
        Duration::from_millis(timeout_ms + 5000),
    )
    .await
    .map(|_| ())
}

pub async fn send_prompt(handle: &str, text: &str) -> Result<()> {
    call(
        "terminal.send",
        json!({
            "terminal": handle,
            "text": text,
            "enter": true,
            "agentPrompt": true,
            "client": { "id": "agent-office", "type": "desktop" }
        }),
        Duration::from_secs(30),
    )
    .await
    .map(|_| ())
}

/// Saved Claude accounts and plan usage. `refresh` has Orca ask Anthropic for fresh numbers.
pub async fn accounts(refresh: bool) -> Result<Value> {
    call("accounts.list", json!({ "refreshUsage": refresh }), Duration::from_secs(20)).await
}

pub async fn select_claude_account(id: &str) -> Result<()> {
    call("accounts.selectClaude", json!({ "accountId": id }), Duration::from_secs(20)).await.map(|_| ())
}

/// Types one line straight into the terminal and presses Return. Unlike `send_prompt`, Orca
/// doesn't hold it back when its status still says the agent is waiting for permission.
pub async fn type_line(handle: &str, text: &str) -> Result<()> {
    call(
        "terminal.send",
        json!({ "terminal": handle, "text": text, "enter": true, "client": { "id": "agent-office", "type": "desktop" } }),
        Duration::from_secs(15),
    )
    .await
    .map(|_| ())
}

/// Presses Escape in the terminal, which closes Claude's question box.
pub async fn press_escape(handle: &str) -> Result<()> {
    send_keys(handle, "\u{1b}").await
}

/// Sends keys as if typed, without pressing Return.
pub async fn send_keys(handle: &str, keys: &str) -> Result<()> {
    call("terminal.send", json!({ "terminal": handle, "text": keys, "client": { "id": "agent-office", "type": "desktop" } }), QUICK)
        .await
        .map(|_| ())
}

/// The rows on the terminal's screen right now.
pub async fn read_screen(handle: &str) -> Result<Vec<String>> {
    let r = call("terminal.read", json!({ "terminal": handle }), QUICK).await?;
    Ok(r["terminal"]["tail"].as_array().into_iter().flatten().filter_map(|l| l.as_str()).map(str::to_string).collect())
}

/// What's in Claude's message box: the rows between the last two rules on its screen.
pub fn message_box(screen: &[String]) -> Vec<String> {
    let rules: Vec<usize> = screen.iter().enumerate().filter(|(_, l)| l.trim_start().starts_with("──")).map(|(i, _)| i).collect();
    let [.., top, bottom] = rules[..] else { return vec![] };
    screen[top + 1..bottom]
        .iter()
        .map(|l| l.trim_start().trim_start_matches('❯').trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Messages you sent while Claude was busy, still waiting their turn.
pub fn has_queued(screen: &[String]) -> bool {
    message_box(screen).iter().any(|l| l.contains("Press up to edit queued messages")) || screen.iter().any(|l| l.contains("ctrl+enter to send now"))
}

pub async fn close_terminal(handle: &str) -> Result<()> {
    call("terminal.close", json!({ "terminal": handle }), QUICK).await.map(|_| ())
}

pub async fn focus_terminal(handle: &str) -> Result<()> {
    call("terminal.focus", json!({ "terminal": handle, "navigation": "host" }), QUICK)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claudes_message_box() {
        let screen: Vec<String> = [
            "  Later, the Coast Guard called with thanks.",
            "❯ SECOND MESSAGE: reply with exactly the word banana",
            "  ctrl+enter to send now",
            "──────────────────────────────────── StopProbe ─",
            "❯ Press up to edit queued messages",
            "────────────────────────────────────────────────",
            "  ✨ Haiku 4.5 | 💬 25% ctx used",
        ]
        .map(String::from)
        .to_vec();
        assert!(has_queued(&screen));
        let mut typed = screen.clone();
        typed[1] = "  more story".into();
        typed[2] = "  and more".into();
        typed[4] = "❯ QUEUED ONE: reply".into();
        typed.insert(5, "  second line of it".into());
        assert_eq!(message_box(&typed), ["QUEUED ONE: reply", "second line of it"]);
        assert!(!has_queued(&typed));
        typed[4] = "❯".into();
        typed.remove(5);
        assert!(message_box(&typed).is_empty());
    }
}
