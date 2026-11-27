//! Client for cmux's local socket: one newline-terminated JSON request per connection, preceded
//! by `auth <password>` when you've given the office a socket password.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

const QUICK: Duration = Duration::from_secs(6);

pub fn socket_path() -> Option<PathBuf> {
    ["/tmp/cmux.sock", "/tmp/cmux-nightly.sock"].iter().map(PathBuf::from).find(|p| p.exists())
}

pub async fn call(method: &str, params: Value, password: &str, timeout: Duration) -> Result<Value> {
    let path = socket_path().ok_or_else(|| anyhow!("cmux isn't running"))?;
    let id = uuid::Uuid::new_v4().to_string();
    let line = format!("{}\n", json!({ "id": id, "method": method, "params": params }));
    let fut = async {
        let stream = UnixStream::connect(&path).await.context("Couldn't reach cmux")?;
        let (r, mut w) = stream.into_split();
        let mut lines = BufReader::new(r).lines();
        if !password.is_empty() {
            w.write_all(format!("auth {password}\n").as_bytes()).await?;
            let reply = lines.next_line().await?.unwrap_or_default();
            if reply.starts_with("ERROR:") && !reply.contains("Unknown command 'auth'") {
                return Err(anyhow!("cmux didn't accept the socket password"));
            }
        }
        w.write_all(line.as_bytes()).await?;
        while let Some(l) = lines.next_line().await? {
            let l = l.trim();
            if l.is_empty() {
                continue;
            }
            if l.starts_with("ERROR:") {
                return Err(anyhow!("cmux: {}", l.trim_start_matches("ERROR:").trim()));
            }
            let v: Value = serde_json::from_str(l)?;
            if v["id"] != json!(id) {
                continue;
            }
            if v["ok"] == json!(true) {
                return Ok(v["result"].clone());
            }
            let msg = v["error"]["message"].as_str().map(str::to_string).unwrap_or_else(|| v["error"].to_string());
            return Err(anyhow!("cmux: {msg}"));
        }
        Err(anyhow!(NO_ACCESS))
    };
    tokio::time::timeout(timeout, fut).await.map_err(|_| anyhow!("cmux took too long to answer {method}"))?
}

/// cmux hangs up on programs it doesn't trust, without saying why.
pub const NO_ACCESS: &str = "cmux closed the connection. Its socket only lets in cmux's own terminals until you allow other apps.";

pub async fn ping(password: &str) -> Result<()> {
    call("system.ping", json!({}), password, QUICK).await.map(|_| ())
}

/// Opens a workspace in `cwd` and types `command` into its shell. Returns (workspace, surface).
pub async fn create(cwd: &str, command: &str, title: &str, password: &str) -> Result<(String, String)> {
    let r = call(
        "workspace.create",
        json!({ "cwd": cwd, "title": title, "initial_input": format!("{command}\r"), "focus": false }),
        password,
        Duration::from_secs(20),
    )
    .await?;
    let ws = r["workspace_id"].as_str().map(str::to_string).ok_or_else(|| anyhow!("cmux didn't return a workspace"))?;
    let surface = match r["surface_id"].as_str() {
        Some(s) => s.to_string(),
        None => {
            let l = call("surface.list", json!({ "workspace_id": ws }), password, QUICK).await?;
            l["surfaces"][0]["id"].as_str().map(str::to_string).ok_or_else(|| anyhow!("cmux didn't return a terminal"))?
        }
    };
    Ok((ws, surface))
}

/// Types a message into Claude and presses Return. Newer cmux pastes it whole; older builds
/// get it as keystrokes, where a newline would send early, so line breaks become spaces.
pub async fn send_prompt(surface: &str, text: &str, password: &str) -> Result<()> {
    let pasted = call(
        "terminal.paste",
        json!({ "surface_id": surface, "text": text, "submit_key": "return" }),
        password,
        QUICK,
    )
    .await;
    match pasted {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains(NO_ACCESS) || e.to_string().contains("password") => Err(e),
        Err(_) => {
            let flat = text.split('\n').map(str::trim_end).collect::<Vec<_>>().join(" ");
            call("surface.send_text", json!({ "surface_id": surface, "text": flat }), password, QUICK).await?;
            tokio::time::sleep(Duration::from_millis(120)).await;
            call("surface.send_key", json!({ "surface_id": surface, "key": "enter" }), password, QUICK).await.map(|_| ())
        }
    }
}

pub async fn press_escape(surface: &str, password: &str) -> Result<()> {
    call("surface.send_key", json!({ "surface_id": surface, "key": "escape" }), password, QUICK).await.map(|_| ())
}

pub async fn close(surface: &str, workspace: Option<&str>, password: &str) -> Result<()> {
    match workspace {
        Some(ws) => call("workspace.close", json!({ "workspace_id": ws }), password, QUICK).await,
        None => call("surface.close", json!({ "surface_id": surface }), password, QUICK).await,
    }
    .map(|_| ())
}

pub async fn focus(surface: &str, workspace: Option<&str>, password: &str) -> Result<()> {
    if let Some(ws) = workspace {
        call("workspace.select", json!({ "workspace_id": ws }), password, QUICK).await?;
    }
    call("surface.focus", json!({ "surface_id": surface }), password, QUICK).await.map(|_| ())
}
