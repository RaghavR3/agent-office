mod brief;
mod cmux;
mod contract;
mod hooks;
mod links;
mod mcp;
mod office;
mod orca;
mod skills;
mod status;
mod transcript;
mod usage;

use office::{file_names, now_ms, sh_quote, with_files, Action, CosRequest, Host, Managed, Office, Pending, Snapshot, HQ};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, Notify};

#[derive(Clone)]
pub struct Shared {
    office: Arc<Mutex<Office>>,
    kick: Arc<Notify>,
    force_terminals: Arc<AtomicBool>,
    force_structure: Arc<AtomicBool>,
}

impl Shared {
    fn nudge(&self) {
        self.force_terminals.store(true, Ordering::Relaxed);
        self.kick.notify_one();
    }
}

type Res<T> = Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// ---- talking to the terminal app ----------------------------------------------------------
// Agent ids say where they live: "cmux:<surface>" for cmux, Orca's terminal handle otherwise.

async fn send_to(sh: &Shared, id: &str, text: &str) -> anyhow::Result<()> {
    match id.strip_prefix("cmux:") {
        Some(surface) => {
            let pw = sh.office.lock().await.p.cmux_password.clone();
            cmux::send_prompt(surface, text, &pw).await
        }
        None => orca::send_prompt(id, text).await,
    }
}

async fn close_agent(sh: &Shared, id: &str) -> anyhow::Result<()> {
    match id.strip_prefix("cmux:") {
        Some(surface) => {
            let (pw, ws) = {
                let o = sh.office.lock().await;
                // Only close the whole workspace if the office opened it for this agent.
                (o.p.cmux_password.clone(), o.p.managed.get(id).and_then(|m| m.workspace.clone()))
            };
            cmux::close(surface, ws.as_deref(), &pw).await
        }
        None => orca::close_terminal(id).await,
    }
}

async fn focus_agent(sh: &Shared, id: &str) -> anyhow::Result<()> {
    match id.strip_prefix("cmux:") {
        Some(surface) => {
            let (pw, ws) = {
                let o = sh.office.lock().await;
                (o.p.cmux_password.clone(), o.cmux_workspace(id))
            };
            cmux::focus(surface, ws.as_deref(), &pw).await
        }
        None => orca::focus_terminal(id).await,
    }
}

/// One line typed straight in, for when the terminal app's own prompt route would hold it back.
async fn type_line(sh: &Shared, id: &str, text: &str) -> anyhow::Result<()> {
    match id.strip_prefix("cmux:") {
        Some(surface) => {
            let pw = sh.office.lock().await.p.cmux_password.clone();
            cmux::send_prompt(surface, text, &pw).await
        }
        None => orca::type_line(id, text).await,
    }
}

async fn press_escape(sh: &Shared, id: &str) -> anyhow::Result<()> {
    match id.strip_prefix("cmux:") {
        Some(surface) => {
            let pw = sh.office.lock().await.p.cmux_password.clone();
            cmux::press_escape(surface, &pw).await
        }
        None => orca::press_escape(id).await,
    }
}

/// Waits until a new session can take a prompt.
async fn wait_ready(sh: &Shared, handle: &str, session_id: &str) {
    if handle.starts_with("cmux:") {
        // Our hook reports the session starting; give Claude a moment to draw its prompt.
        for _ in 0..150 {
            if sh.office.lock().await.live.hooks.started(session_id) {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        return;
    }
    if orca::wait_ready(handle, 45_000).await.is_err() {
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

// ---- spawning ------------------------------------------------------------------------------

/// Opens a new Claude Code session for a room in Orca or cmux. Every value in the command line
/// is generated here; the task text is typed into Claude afterwards, never through the shell.
async fn spawn(
    sh: &Shared,
    room_id: &str,
    area_id: Option<&str>,
    role: &str,
    task: Option<String>,
    resume: Option<(String, String)>,
    desk: Option<u32>,
) -> anyhow::Result<String> {
    // A saved session comes back with its own id and name; anything else starts fresh.
    let session_id = resume.as_ref().map(|(id, _)| id.clone()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let start = match &resume {
        Some((id, _)) => format!("--resume {id}"),
        None => format!("--session-id {session_id}"),
    };
    let pending_id = format!("pending-{session_id}");
    let (selector, cwd, name, command, host, password) = {
        let mut o = sh.office.lock().await;
        let host = o.live.host;
        if host == Host::Cmux && !o.live.hooks_installed {
            anyhow::bail!("Install the status hook in Settings first, so the office can see agents in cmux");
        }
        let room = if role == "cos" {
            o.hq_room()
        } else {
            o.rooms().into_iter().find(|r| r.id == room_id)
        }
        .ok_or_else(|| anyhow::anyhow!("That room isn't there any more"))?;
        // An area is one of the room's named Orca workspaces; the agent starts in that folder.
        let area = area_id.and_then(|id| room.areas.iter().find(|a| a.id == id)).cloned();
        let name = if role == "cos" { o.cos_name() } else { o.pick_name() };
        let contract = o.contract_path(role).display().to_string();
        let attachments = o.attachments_dir().display().to_string();
        let command = if role == "cos" {
            format!(
                "claude {start} --model sonnet --permission-mode auto --tools '' --strict-mcp-config --append-system-prompt-file {} -n {}",
                sh_quote(&contract),
                sh_quote(&name)
            )
        } else {
            format!(
                "claude {start} --permission-mode auto --append-system-prompt-file {} --add-dir {} --disallowedTools AskUserQuestion -n {}",
                sh_quote(&contract),
                sh_quote(&attachments),
                sh_quote(resume.as_ref().map(|(_, n)| n).unwrap_or(&name))
            )
        };
        o.live.pending.push(Pending {
            id: pending_id.clone(),
            name: name.clone(),
            room_id: if role == "cos" { HQ.into() } else { room.id.clone() },
            role: role.into(),
            area: area.as_ref().map(|a| a.name.clone()),
            handle: None,
            desk,
            since: now_ms(),
        });
        let password = o.p.cmux_password.clone();
        match area {
            Some(a) => (format!("id:{}", a.id), a.path, name, command, host, password),
            None => (room.selector, room.path, name, command, host, password),
        }
    };
    sh.nudge();

    let created = match host {
        Host::Orca => orca::create_terminal(&selector, &command, &name).await.map(|h| (h, None)),
        Host::Cmux => cmux::create(&cwd, &command, &name, &password).await.map(|(ws, surface)| (format!("cmux:{surface}"), Some(ws))),
    };
    {
        let mut o = sh.office.lock().await;
        match &created {
            Ok((h, _)) => o.live.pending.iter_mut().filter(|p| p.id == pending_id).for_each(|p| p.handle = Some(h.clone())),
            Err(_) => o.live.pending.retain(|p| p.id != pending_id),
        }
        if let Ok((handle, workspace)) = &created {
            if role == "cos" {
                o.p.cos_handle = Some(handle.clone());
            } else {
                o.p.names.insert(handle.clone(), name.clone());
            }
            if let Some((_, saved_name)) = &resume {
                o.p.session_names.insert(handle.clone(), saved_name.clone());
            }
            if let Some(d) = desk {
                o.p.desks.insert(handle.clone(), d);
            }
            o.p.managed.insert(
                handle.clone(),
                Managed {
                    session_id: session_id.clone(),
                    role: role.into(),
                    task: task.clone(),
                    created_at: now_ms(),
                    cwd,
                    workspace: workspace.clone(),
                    incarnation: None,
                },
            );
            o.live.starting.insert(handle.clone());
            o.save();
        }
    }
    sh.nudge();
    let (handle, _) = created?;

    // Claude needs a moment to draw its prompt before it can take input.
    wait_ready(sh, &handle, &session_id).await;
    let sent = match &task {
        Some(t) if !t.trim().is_empty() => send_to(sh, &handle, t).await,
        _ => Ok(()),
    };
    sh.office.lock().await.live.starting.remove(&handle);
    sh.nudge();
    sent?;
    Ok(handle)
}

async fn ensure_cos(sh: &Shared) -> anyhow::Result<String> {
    if let Some(h) = sh.office.lock().await.cos_alive() {
        return Ok(h);
    }
    spawn(sh, HQ, None, "cos", None, None, None).await
}

async fn send_to_cos(sh: &Shared, request_id: String, text: Option<String>) {
    let result = async {
        let handle = ensure_cos(sh).await?;
        let msg = match text {
            Some(t) => t,
            None => sh.office.lock().await.cos_message(&request_id),
        };
        {
            let mut o = sh.office.lock().await;
            if let Some(r) = o.request_mut(&request_id) {
                r.state = "sent".into();
                r.sent_at = now_ms();
            }
            o.save();
        }
        send_to(sh, &handle, &msg).await
    }
    .await;
    if let Err(e) = result {
        let mut o = sh.office.lock().await;
        let cos = o.cos_name();
        if let Some(r) = o.request_mut(&request_id) {
            r.state = "failed".into();
            r.result = Some(format!("Couldn't reach {cos}: {e}"));
        }
        o.save();
    }
    sh.nudge();
}

async fn run_action(sh: Shared, action: Action) {
    let Action::Dispatch { request, room, area, agent, task } = action;
    let (target, cos, task) = {
        let o = sh.office.lock().await;
        let cos = o.cos_name();
        let files = o.p.requests.iter().find(|r| r.id == request).map(|r| r.files.clone()).unwrap_or_default();
        let task = with_files(&task, &files);
        let found = o
            .rooms()
            .into_iter()
            .find(|r| r.name.eq_ignore_ascii_case(room.trim()) || r.original.eq_ignore_ascii_case(room.trim()) || r.id == room);
        let target = found.map(|r| {
            let reuse = o
                .agents()
                .into_iter()
                .find(|a| {
                    a.room_id == r.id
                        && a.role == "worker"
                        && a.name.eq_ignore_ascii_case(agent.trim())
                        && matches!(a.state.as_str(), "idle" | "done" | "needs_you")
                })
                .map(|a| (a.id, a.name));
            let area_id = r.areas.iter().find(|a| a.name.eq_ignore_ascii_case(area.trim())).map(|a| a.id.clone());
            (r, reuse, area_id)
        });
        (target, cos, task)
    };
    let outcome: Result<(String, String), String> = match target {
        None => Err(format!("{cos} picked a room that isn't in the office: {room}")),
        Some((r, Some((id, name)), _)) => send_to(&sh, &id, &task)
            .await
            .map(|_| (id, format!("Sent to {name} in {}", r.name)))
            .map_err(err),
        Some((r, None, area_id)) => match spawn(&sh, &r.id, area_id.as_deref(), "worker", Some(task.clone()), None, None).await {
            Ok(id) => {
                let name = sh.office.lock().await.p.names.get(&id).cloned().unwrap_or_default();
                Ok((id, format!("Sent to {name} in {}", r.name)))
            }
            Err(e) => Err(e.to_string()),
        },
    };
    let mut o = sh.office.lock().await;
    if let Some(req) = o.request_mut(&request) {
        match outcome {
            Ok((id, text)) => {
                req.state = "routed".into();
                req.result = Some(text);
                req.agent_id = Some(id);
            }
            Err(e) => {
                req.state = "failed".into();
                req.result = Some(e);
            }
        }
    }
    o.save();
    drop(o);
    sh.nudge();
}

// ---- commands ------------------------------------------------------------------------------

#[tauri::command]
async fn snapshot(sh: State<'_, Shared>) -> Res<Snapshot> {
    Ok(sh.office.lock().await.snapshot())
}

/// Only files the office saved itself can ride along with a request.
fn own_attachments(dir: &std::path::Path, files: Vec<String>) -> Res<Vec<String>> {
    let root = dir.canonicalize().map_err(err)?;
    for f in &files {
        let ok = std::path::Path::new(f).canonicalize().is_ok_and(|p| p.starts_with(&root) && p.is_file());
        if !ok {
            return Err("That attachment has gone. Attach it again.".into());
        }
    }
    Ok(files)
}

#[tauri::command]
async fn delegate(sh: State<'_, Shared>, text: String, files: Option<Vec<String>>) -> Res<String> {
    let text = text.trim().to_string();
    let files = files.unwrap_or_default();
    if text.is_empty() {
        return Err(if files.is_empty() { "Write what you need first" } else { "Say what you'd like done with it" }.into());
    }
    let id = {
        let mut o = sh.office.lock().await;
        let files = own_attachments(&o.attachments_dir(), files)?;
        o.p.next_ref += 1;
        let id = format!("r{}", o.p.next_ref);
        o.p.requests.push(CosRequest {
            id: id.clone(),
            text,
            created_at: now_ms(),
            state: "queued".into(),
            files,
            ..Default::default()
        });
        let keep = o.p.requests.len().saturating_sub(60);
        o.p.requests.drain(..keep);
        o.save();
        id
    };
    let s = sh.inner().clone();
    let rid = id.clone();
    tauri::async_runtime::spawn(async move { send_to_cos(&s, rid, None).await });
    sh.nudge();
    Ok(id)
}

#[tauri::command]
async fn choose_route(sh: State<'_, Shared>, request_id: String, label: String, room: String) -> Res<()> {
    let msg = format!("Request {request_id}: the user picked \"{label}\" (room {room}). Route it now.");
    {
        let mut o = sh.office.lock().await;
        let r = o.request_mut(&request_id).ok_or("That request is gone")?;
        r.state = "queued".into();
        r.question = None;
        r.options.clear();
    }
    let s = sh.inner().clone();
    tauri::async_runtime::spawn(async move { send_to_cos(&s, request_id, Some(msg)).await });
    Ok(())
}

/// Answers the chief of staff's question in your own words, with more files if you like.
#[tauri::command]
async fn answer_request(sh: State<'_, Shared>, request_id: String, text: String, files: Option<Vec<String>>) -> Res<()> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("Write an answer first".into());
    }
    let msg = {
        let mut o = sh.office.lock().await;
        let more = own_attachments(&o.attachments_dir(), files.unwrap_or_default())?;
        let r = o.request_mut(&request_id).ok_or("That request is gone")?;
        if r.state != "asking" {
            return Err("That's already been handled".into());
        }
        r.state = "queued".into();
        r.question = None;
        r.options.clear();
        let mut msg = format!("Request {request_id}: the user answered:\n\"\"\"\n{text}\n\"\"\"");
        if !more.is_empty() {
            msg += &format!("\nThey also attached: {} (Agent Office hands these on too).", file_names(&more));
        }
        r.files.extend(more);
        o.save();
        msg
    };
    let s = sh.inner().clone();
    tauri::async_runtime::spawn(async move { send_to_cos(&s, request_id, Some(msg)).await });
    sh.nudge();
    Ok(())
}

const ANSWERS: &str = "My answers to your questions:";

/// Answers Claude's question box from the office: Escape closes the box, then your answers go
/// in as an ordinary message, which works for any mix of questions, options and free text.
#[tauri::command]
async fn answer_ask(sh: State<'_, Shared>, agent_id: String, answers: Vec<String>) -> Res<()> {
    let (questions, name, path) = {
        let o = sh.office.lock().await;
        let a = o.agents().into_iter().find(|a| a.id == agent_id).ok_or("That agent has gone")?;
        (a.ask.ok_or("That question has already been answered")?, a.name, a.transcript)
    };
    if answers.len() != questions.len() || answers.iter().all(|a| a.trim().is_empty()) {
        return Err("Answer at least one question".into());
    }
    // One line, since it's typed straight in and a line break would send it early.
    let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut msg = String::from(ANSWERS);
    for (i, (q, a)) in questions.iter().zip(&answers).enumerate() {
        let a = one_line(a);
        msg += &format!(" ({}) {} → {}", i + 1, one_line(&q.question), if a.is_empty() { "no answer, use your judgement" } else { &a });
    }
    msg += " (I answered in Agent Office, which closed your question box.)";
    let read = |p: &std::path::Path| transcript::read_last(p, 64_000).unwrap_or_default();

    press_escape(&sh, &agent_id).await.map_err(err)?;
    // Wait until Claude has closed the box, which it notes in the transcript.
    if let Some(p) = &path {
        for _ in 0..25 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if transcript::open_question(&read(p)).is_none() {
                break;
            }
        }
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    type_line(&sh, &agent_id, &msg).await.map_err(err)?;
    // Check it landed, rather than trusting the keystrokes.
    if let Some(p) = &path {
        let mut landed = false;
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if transcript::latest(&read(p)).prompt.is_some_and(|t| t.starts_with(ANSWERS)) {
                landed = true;
                break;
            }
        }
        if !landed {
            return Err(format!("{name}'s question box closed, but the answers didn't arrive. Open the terminal and paste them in: {msg}"));
        }
    }
    let mut o = sh.office.lock().await;
    o.live.asks.remove(&agent_id);
    o.p.seen.insert(agent_id, now_ms());
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

// ---- MCP connections ------------------------------------------------------------------------

/// Opens a terminal in HQ for a one-off Claude command, like signing in to a server. It isn't an
/// agent: the office leaves it off the floor, and it closes itself when the command succeeds.
async fn open_utility(sh: &Shared, command: &str, title: &str, server: &str, room_id: Option<&str>) -> anyhow::Result<()> {
    let (host, room, password) = {
        let o = sh.office.lock().await;
        let room = room_id.and_then(|id| o.rooms().into_iter().find(|r| r.id == id)).or_else(|| o.hq_room());
        (o.live.host, room.ok_or_else(|| anyhow::anyhow!("There's no folder to open a terminal in"))?, o.p.cmux_password.clone())
    };
    let handle = match host {
        Host::Orca => orca::create_terminal(&room.selector, command, title).await?,
        Host::Cmux => format!("cmux:{}", cmux::create(&room.path, command, title, &password).await?.1),
    };
    sh.office.lock().await.live.signins.insert(handle.clone(), (server.to_string(), false));
    // Bring it forward: the sign-in may ask for something, and if it fails the error is there.
    let _ = focus_agent(sh, &handle).await;
    sh.nudge();
    Ok(())
}

fn valid_server_name(name: &str) -> bool {
    (1..=60).contains(&name.chars().count()) && name.chars().all(|c| c.is_ascii_alphanumeric() || " ._:-".contains(c))
}

/// Signs in to an MCP server with Claude Code's own `claude mcp login`, in a terminal, so the
/// sign-in lands in your keychain where every session reads it.
#[tauri::command]
async fn mcp_sign_in(sh: State<'_, Shared>, name: String) -> Res<()> {
    let (name, room) = {
        let o = sh.office.lock().await;
        // The chief of staff may shorten a name ("lorikeet" for "claude.ai Lorikeet").
        let want = name.trim().to_lowercase();
        let server = o
            .live
            .mcp_servers
            .iter()
            .find(|s| s.name.to_lowercase() == want || s.label.to_lowercase() == want || mcp::slug(&s.name).to_lowercase() == mcp::slug(&want))
            .cloned()
            .ok_or("That server isn't set up in Claude Code")?;
        if !valid_server_name(&server.name) {
            return Err("That server isn't set up in Claude Code".into());
        }
        (server.name, server.room)
    };
    let command = format!("claude mcp login {} && exit", sh_quote(&name));
    open_utility(&sh, &command, &format!("Sign in to {name}"), &name, room.as_deref()).await.map_err(err)
}

/// Adds a remote MCP server for all your sessions (Claude's user scope) and signs in to it.
#[tauri::command]
async fn mcp_add(sh: State<'_, Shared>, name: String, url: String) -> Res<()> {
    let name = name.trim().to_string();
    let url = url.trim().to_string();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || name.len() > 40 {
        return Err("Name it with letters, numbers, - or _".into());
    }
    let ok_url = url.starts_with("https://") && url.len() < 300 && !url.contains(char::is_whitespace) && !url.contains(['\'', '"', '`', '$']);
    if !ok_url {
        return Err("Use the server's https:// address".into());
    }
    if sh.office.lock().await.live.mcp_known.iter().any(|(n, s)| n == &name && s == "you") {
        return Err(format!("You already have a server called {name}"));
    }
    let command = format!(
        "claude mcp add --scope user --transport http {n} {u} && claude mcp login {n} && exit",
        n = sh_quote(&name),
        u = sh_quote(&url)
    );
    open_utility(&sh, &command, &format!("Add {name}"), &name, None).await.map_err(err)
}

/// Has each agent reconnect the servers that are failing for it, as soon as it's free.
#[tauri::command]
async fn mcp_reconnect(sh: State<'_, Shared>) -> Res<()> {
    let mut o = sh.office.lock().await;
    let trouble: Vec<(String, HashSet<String>)> = o.live.mcp_trouble.iter().filter(|(_, s)| !s.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
    for (id, servers) in trouble {
        o.live.reconnect.entry(id).or_default().extend(servers);
    }
    drop(o);
    sh.nudge();
    Ok(())
}

/// Asks the chief of staff for a fresh rundown now, rather than when something next changes.
#[tauri::command]
async fn brief_now(sh: State<'_, Shared>) -> Res<()> {
    sh.office.lock().await.live.brief_now = true;
    sh.nudge();
    Ok(())
}

/// Takes one item off the rundown, once you've dealt with it or don't care.
#[tauri::command]
async fn dismiss_brief_item(sh: State<'_, Shared>, index: usize) -> Res<()> {
    let mut o = sh.office.lock().await;
    if let Some(b) = o.p.briefing.as_mut() {
        if index < b.items.len() {
            b.items.remove(index);
        }
    }
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

/// At most once an hour, and only if something changed, asks the chief of staff for a rundown:
/// what needs you, what's stuck, and what one agent should know from another. Refresh asks now.
async fn maybe_brief(sh: &Shared) {
    let msg = {
        let mut o = sh.office.lock().await;
        let (key, text, attention) = o.rundown();
        let now = now_ms();
        if key != o.live.brief_seen.0 {
            o.live.brief_seen = (key.clone(), now);
        }
        let forced = o.live.brief_now;
        let settled = now - o.live.brief_seen.1 > 15_000;
        let routing = o.p.requests.iter().any(|r| matches!(r.state.as_str(), "queued" | "sent" | "routing"));
        let awaiting = o.p.briefing_pending && now - o.p.briefing_sent_at < 120_000;
        if attention == 0 {
            // Nothing needs you: say so without asking anyone.
            o.live.brief_now = false;
            if o.p.briefing.as_ref().is_none_or(|b| !b.items.is_empty()) {
                o.p.briefing = Some(office::Briefing { summary: "Nothing needs you right now.".into(), items: vec![], at: now });
                o.p.briefing_key = key;
                o.save();
            }
            None
        } else if routing || awaiting || !(forced || (key != o.p.briefing_key && settled && now - o.p.briefing_sent_at > 3_600_000)) {
            None
        } else {
            o.live.brief_now = false;
            o.p.next_brief += 1;
            let id = format!("b{}", o.p.next_brief);
            o.p.briefing_key = key;
            o.p.briefing_sent_at = now;
            o.p.briefing_pending = true;
            o.p.briefs_since_clear += 1;
            let clear = o.p.briefs_since_clear > 15;
            if clear {
                o.p.briefs_since_clear = 0;
            }
            o.save();
            Some((format!("Briefing {id}:\n{text}"), clear))
        }
    };
    let Some((msg, clear)) = msg else { return };
    let s = sh.clone();
    tauri::async_runtime::spawn(async move {
        let sent = async {
            let cos = ensure_cos(&s).await?;
            // Each rundown carries the whole picture, so old ones can go to keep the session small.
            if clear {
                send_to(&s, &cos, "/clear").await?;
                tokio::time::sleep(Duration::from_millis(1500)).await;
            }
            send_to(&s, &cos, &msg).await
        }
        .await;
        if sent.is_err() {
            s.office.lock().await.p.briefing_pending = false;
        }
    });
}

/// Calls off a hand-off that hasn't gone to anyone yet. A late answer from the chief of staff
/// is ignored, since only requests still waiting on it get routed.
#[tauri::command]
async fn cancel_request(sh: State<'_, Shared>, request_id: String) -> Res<()> {
    let mut o = sh.office.lock().await;
    let r = o.request_mut(&request_id).ok_or("That request is gone")?;
    if !matches!(r.state.as_str(), "queued" | "sent" | "asking") {
        return Err("It's already on its way to an agent".into());
    }
    r.state = "cancelled".into();
    r.dismissed = true;
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn dismiss_request(sh: State<'_, Shared>, request_id: String) -> Res<()> {
    let mut o = sh.office.lock().await;
    if let Some(r) = o.request_mut(&request_id) {
        r.dismissed = true;
    }
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn spawn_agent(sh: State<'_, Shared>, room_id: String, area: Option<String>, task: Option<String>, desk: Option<u32>) -> Res<()> {
    let s = sh.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = spawn(&s, &room_id, area.as_deref(), "worker", task, None, desk).await {
            s.office.lock().await.live.error = Some(format!("Couldn't add an agent: {e}"));
        }
    });
    Ok(())
}

/// Names an agent's Claude session (`/rename`, once the agent is free) and keeps it in the
/// saved list, so it can be brought back after its tab is closed.
#[tauri::command]
async fn rename_session(sh: State<'_, Shared>, agent_id: String, name: String) -> Res<()> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err("Use 1 to 60 characters".into());
    }
    let mut o = sh.office.lock().await;
    let agent = o.agents().into_iter().find(|a| a.id == agent_id).ok_or("That agent has gone")?;
    let session = agent
        .transcript
        .as_ref()
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .ok_or("This session hasn't started yet")?;
    let cwd = o.agent_cwd(&agent_id).unwrap_or_default();
    o.p.session_names.insert(agent_id.clone(), name.clone());
    o.p.saved.retain(|s| s.id != session);
    o.p.saved.push(office::SavedSession { id: session, name: name.clone(), room_id: agent.room_id, cwd, saved_at: now_ms(), open: true });
    let extra = o.p.saved.len().saturating_sub(50);
    o.p.saved.drain(..extra);
    o.live.later.entry(agent_id).or_default().push(format!("/rename {name}"));
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

/// Opens a saved session again in its room, with its history (`claude --resume`).
#[tauri::command]
async fn resume_session(sh: State<'_, Shared>, session_id: String) -> Res<()> {
    let (room_id, area_id, name) = {
        let o = sh.office.lock().await;
        let s = o.p.saved.iter().find(|s| s.id == session_id).cloned().ok_or("That session isn't saved")?;
        if uuid::Uuid::parse_str(&s.id).is_err() {
            return Err("That session id doesn't look right".into());
        }
        if let Some(a) = o.agents().into_iter().find(|a| a.transcript.as_ref().is_some_and(|p| p.file_stem().is_some_and(|f| f == s.id.as_str()))) {
            return Err(format!("{} already has it open", a.name));
        }
        // Claude files a session under the folder it ran in, so it has to come back there.
        let room = o
            .rooms()
            .into_iter()
            .find(|r| r.areas.iter().any(|a| a.path == s.cwd) || r.path == s.cwd)
            .or_else(|| o.rooms().into_iter().find(|r| r.id == s.room_id))
            .ok_or("Its room isn't in the office any more")?;
        let area = room.areas.iter().find(|a| a.path == s.cwd).map(|a| a.id.clone());
        (room.id, area, s.name)
    };
    let s = sh.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = spawn(&s, &room_id, area_id.as_deref(), "worker", None, Some((session_id, name)), None).await {
            s.office.lock().await.live.error = Some(format!("Couldn't bring it back: {e}"));
        }
    });
    Ok(())
}

#[tauri::command]
async fn forget_session(sh: State<'_, Shared>, session_id: String) -> Res<()> {
    let mut o = sh.office.lock().await;
    o.p.saved.retain(|s| s.id != session_id);
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn message_agent(sh: State<'_, Shared>, agent_id: String, text: String) -> Res<()> {
    if text.trim().is_empty() {
        return Err("Write a message first".into());
    }
    {
        let mut o = sh.office.lock().await;
        let at = o.agents().iter().find(|a| a.id == agent_id).map(|a| a.updated_at).unwrap_or(0);
        o.p.seen.insert(agent_id.clone(), at.max(now_ms()));
        o.save();
    }
    send_to(&sh, &agent_id, text.trim()).await.map_err(err)?;
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn remove_agent(sh: State<'_, Shared>, agent_id: String) -> Res<()> {
    {
        let mut o = sh.office.lock().await;
        if o.live.pending.iter().any(|p| p.id == agent_id) {
            o.live.pending.retain(|p| p.id != agent_id);
            drop(o);
            sh.nudge();
            return Ok(());
        }
    }
    // A terminal that's already gone, or that won't close, still comes off the floor.
    let closed = close_agent(&sh, &agent_id).await.is_ok();
    let mut o = sh.office.lock().await;
    if !closed {
        o.p.ignored.push(agent_id.clone());
    }
    o.p.managed.remove(&agent_id);
    o.p.names.remove(&agent_id);
    o.p.seen.remove(&agent_id);
    o.p.session_names.remove(&agent_id);
    o.p.desks.remove(&agent_id);
    o.p.titles.remove(&agent_id);
    if o.p.cos_handle.as_deref() == Some(&agent_id) {
        o.p.cos_handle = None;
    }
    o.live.terminals.retain(|t| t["handle"] != agent_id.as_str());
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

/// Renames the chief of staff. A running session keeps its old name until it next starts, so an
/// idle one is closed and comes back under the new name with the next request.
#[tauri::command]
async fn rename_cos(sh: State<'_, Shared>, name: String) -> Res<String> {
    let name = name.trim().to_string();
    let ok = (1..=20).contains(&name.chars().count())
        && name.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '-' || c == '\'');
    if !ok {
        return Err("Use up to 20 letters, spaces or hyphens".into());
    }
    let idle_cos = {
        let mut o = sh.office.lock().await;
        o.p.cos_name = name.clone();
        o.write_contracts().map_err(err)?;
        o.save();
        let busy = o.p.requests.iter().any(|r| matches!(r.state.as_str(), "queued" | "sent" | "routing"));
        if busy { None } else { o.cos_alive() }
    };
    if let Some(h) = idle_cos {
        let _ = close_agent(&sh, &h).await;
        sh.office.lock().await.p.cos_handle = None;
    }
    sh.nudge();
    Ok(name)
}

/// Makes a folder a room: in Orca it's added as a repo, with cmux the office keeps the list.
#[tauri::command]
async fn add_room(sh: State<'_, Shared>, path: String) -> Res<()> {
    let p = std::path::Path::new(&path);
    if !p.is_absolute() || !p.is_dir() {
        return Err("Pick a folder on this Mac".into());
    }
    let host = sh.office.lock().await.live.host;
    match host {
        Host::Cmux => sh.office.lock().await.add_folder(&path),
        Host::Orca => {
            orca::call("repo.add", serde_json::json!({ "path": path }), Duration::from_secs(20))
                .await
                .map_err(err)?;
        }
    }
    sh.force_structure.store(true, Ordering::Relaxed);
    sh.nudge();
    Ok(())
}

/// Gives a room an office-only name. An empty name goes back to Orca's.
#[tauri::command]
async fn rename_room(sh: State<'_, Shared>, room_id: String, name: String) -> Res<()> {
    let name = name.trim().to_string();
    if name.chars().count() > 40 {
        return Err("Keep room names under 40 characters".into());
    }
    let mut o = sh.office.lock().await;
    if name.is_empty() {
        o.p.room_names.remove(&room_id);
    } else {
        o.p.room_names.insert(room_id, name);
    }
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

/// Leaves a room out of the office (or brings it back). Nothing changes in Orca.
#[tauri::command]
async fn set_room_hidden(sh: State<'_, Shared>, room_id: String, hidden: bool) -> Res<()> {
    let mut o = sh.office.lock().await;
    o.p.hidden_rooms.retain(|id| id != &room_id);
    if hidden {
        o.p.hidden_rooms.push(room_id);
    }
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

/// Moves a room one place along the ring.
#[tauri::command]
async fn move_room(sh: State<'_, Shared>, room_id: String, delta: i32) -> Res<()> {
    let mut o = sh.office.lock().await;
    let mut order: Vec<String> = o.rooms().into_iter().map(|r| r.id).collect();
    let Some(i) = order.iter().position(|id| id == &room_id) else { return Ok(()) };
    let j = (i as i32 + delta).clamp(0, order.len() as i32 - 1) as usize;
    order.swap(i, j);
    let rest: Vec<String> = o.p.room_order.iter().filter(|id| !order.contains(id)).cloned().collect();
    order.extend(rest);
    o.p.room_order = order;
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

/// "Done, start fresh": the agent keeps its desk, area and Orca tab, but its Claude session is
/// cleared so the next task starts with a clean context.
#[tauri::command]
async fn reset_agent(sh: State<'_, Shared>, agent_id: String) -> Res<()> {
    let agent = sh.office.lock().await.agents().into_iter().find(|a| a.id == agent_id).ok_or("That agent has gone")?;
    if matches!(agent.state.as_str(), "working" | "starting" | "waiting") || agent.ask.is_some() {
        return Err(format!("{} is in the middle of something. Wait for them to finish, or stop them in the terminal.", agent.name));
    }
    send_to(&sh, &agent_id, "/clear").await.map_err(err)?;
    let mut o = sh.office.lock().await;
    if let Some(m) = o.p.managed.get_mut(&agent_id) {
        m.task = None;
    }
    // The name belonged to the session just cleared; it stays in the saved list.
    o.p.session_names.remove(&agent_id);
    if !agent.title.is_empty() {
        o.p.cleared_titles.insert(agent_id.clone(), agent.title.clone());
    }
    o.p.seen.insert(agent_id, now_ms());
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn set_notifications(sh: State<'_, Shared>, on: bool) -> Res<()> {
    let mut o = sh.office.lock().await;
    o.p.notifications_off = !on;
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

const MAX_ATTACHMENT: u64 = 50 * 1024 * 1024;

/// Keeps the file's own name, so agents can tell notes.md from a screenshot, in a folder of its own.
fn store_attachment(dir: &std::path::Path, name: &str, bytes: &[u8]) -> Res<String> {
    if bytes.len() as u64 > MAX_ATTACHMENT {
        return Err("That file is over 50 MB".into());
    }
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut clean: String = base
        .chars()
        .map(|c| if c.is_alphanumeric() || " ._-()".contains(c) { c } else { '_' })
        .collect::<String>()
        .trim_start_matches(['.', ' '])
        .trim()
        .chars()
        .take(80)
        .collect();
    if clean.is_empty() {
        clean = "file".into();
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let folder = dir.join(&id[..10]);
    std::fs::create_dir_all(&folder).map_err(err)?;
    let path = folder.join(clean);
    std::fs::write(&path, bytes).map_err(err)?;
    Ok(path.display().to_string())
}

/// Saves a pasted or dropped file and returns the path to mention in the message.
#[tauri::command]
async fn save_attachment(sh: State<'_, Shared>, name: String, data: String) -> Res<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(err)?;
    let dir = sh.office.lock().await.attachments_dir();
    store_attachment(&dir, &name, &bytes)
}

/// Copies a file picked from disk into the attachments folder, so any agent can read it.
#[tauri::command]
async fn import_attachment(sh: State<'_, Shared>, path: String) -> Res<String> {
    let p = std::path::Path::new(&path);
    let meta = std::fs::metadata(p).map_err(err)?;
    if !meta.is_file() {
        return Err("Attach files, not folders".into());
    }
    if meta.len() > MAX_ATTACHMENT {
        return Err("That file is over 50 MB".into());
    }
    let bytes = std::fs::read(p).map_err(err)?;
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string();
    let dir = sh.office.lock().await.attachments_dir();
    store_attachment(&dir, &name, &bytes)
}

/// Asks Orca for fresh plan usage now, rather than at the next half-minute check.
#[tauri::command]
async fn refresh_usage(sh: State<'_, Shared>) -> Res<()> {
    let v = orca::accounts(true).await.map_err(err)?;
    sh.office.lock().await.live.usage = Some(usage::parse(&v));
    sh.nudge();
    Ok(())
}

/// Whether Claude is still running in an agent's terminal.
async fn claude_running(sh: &Shared, id: &str) -> bool {
    match id.strip_prefix("cmux:") {
        Some(surface) => sh.office.lock().await.live.hooks.sessions().any(|s| s.surface.as_deref() == Some(surface) && !s.ended()),
        None => match orca::list_terminals().await {
            Ok(t) => t["terminals"].as_array().into_iter().flatten().any(|x| x["handle"] == id && x["agentIdentity"] == "claude"),
            Err(_) => true,
        },
    }
}

/// Quits Claude in an agent's terminal and resumes the same session there, so it picks up the
/// account now signed in. Same terminal, so its name, desk and area stay as they were.
async fn restart_on_new_account(sh: &Shared, id: &str, command: &str) -> anyhow::Result<()> {
    send_to(sh, id, "/exit").await?;
    let mut gone = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if !claude_running(sh, id).await {
            gone = true;
            break;
        }
    }
    if !gone {
        anyhow::bail!("Claude didn't quit");
    }
    // The shell needs a moment to show its prompt again.
    tokio::time::sleep(Duration::from_millis(600)).await;
    type_line(sh, id, command).await
}

/// Who's in the middle of something: a turn, a permission prompt, an open question, or helpers.
fn busy_names(o: &Office) -> Vec<String> {
    let mut names: Vec<String> = o
        .agents()
        .into_iter()
        .filter(|a| a.role == "worker")
        .filter(|a| matches!(a.state.as_str(), "working" | "starting" | "waiting") || a.ask.is_some() || !a.helpers.is_empty())
        .map(|a| a.name)
        .collect();
    if o.p.requests.iter().any(|r| matches!(r.state.as_str(), "queued" | "sent" | "routing")) || o.p.briefing_pending {
        names.push(o.cos_name());
    }
    names
}

/// Switches which Claude subscription Orca signs in with, then restarts every agent so it's
/// used straight away. Refused while anyone is mid-task, so nothing gets cut off.
#[tauri::command]
async fn switch_account(sh: State<'_, Shared>, account_id: String) -> Res<()> {
    let (sessions, cos) = {
        let o = sh.office.lock().await;
        let busy = busy_names(&o);
        if !busy.is_empty() {
            let who = match busy.len() {
                1 => busy[0].clone(),
                2 => format!("{} and {}", busy[0], busy[1]),
                n => format!("{}, {} and {} more", busy[0], busy[1], n - 2),
            };
            return Err(format!("{who} {} mid-task. Switch once they're done, since switching restarts every agent.", if busy.len() == 1 { "is" } else { "are" }));
        }
        let contract = o.contract_path("worker").display().to_string();
        let attachments = o.attachments_dir().display().to_string();
        // Each agent comes back in its own session, with the flags the office started it with.
        let sessions: Vec<(String, String)> = o
            .agents()
            .into_iter()
            .filter(|a| a.role == "worker" && !a.id.starts_with("pending-"))
            .filter_map(|a| {
                let sid = a.transcript.as_ref()?.file_stem()?.to_str()?.to_string();
                uuid::Uuid::parse_str(&sid).ok()?;
                let command = if o.p.managed.contains_key(&a.id) {
                    format!(
                        "claude --resume {sid} --permission-mode auto --append-system-prompt-file {} --add-dir {} --disallowedTools AskUserQuestion",
                        sh_quote(&contract),
                        sh_quote(&attachments)
                    )
                } else {
                    format!("claude --resume {sid}")
                };
                Some((a.id, command))
            })
            .collect();
        (sessions, o.cos_alive())
    };

    orca::select_claude_account(&account_id).await.map_err(err)?;
    let v = orca::accounts(true).await.map_err(err)?;
    {
        let mut o = sh.office.lock().await;
        o.live.usage = Some(usage::parse(&v));
        o.live.restarting.extend(sessions.iter().map(|(id, _)| id.clone()));
        if cos.is_some() {
            o.p.cos_handle = None;
        }
    }
    sh.nudge();

    let s = sh.inner().clone();
    tauri::async_runtime::spawn(async move {
        // The chief of staff has nothing worth keeping; it comes back on the new account when needed.
        if let Some(h) = cos {
            let _ = close_agent(&s, &h).await;
        }
        let mut failed = vec![];
        for (id, command) in sessions {
            if restart_on_new_account(&s, &id, &command).await.is_err() {
                failed.push(id.clone());
            }
            s.office.lock().await.live.restarting.remove(&id);
            s.nudge();
        }
        if !failed.is_empty() {
            let mut o = s.office.lock().await;
            let names: Vec<String> = o.agents().into_iter().filter(|a| failed.contains(&a.id)).map(|a| a.name).collect();
            o.live.error = Some(format!("Couldn't restart {} on the new account. Open their terminal and run claude --resume.", names.join(", ")));
        }
        s.nudge();
    });
    Ok(())
}

/// Skills and slash commands available where an agent works (or in a room, for a new hire).
#[tauri::command]
async fn skills(sh: State<'_, Shared>, agent_id: Option<String>, room_id: Option<String>) -> Res<Vec<skills::Skill>> {
    let cwd = {
        let o = sh.office.lock().await;
        agent_id
            .as_deref()
            .and_then(|id| o.agent_cwd(id))
            .or_else(|| room_id.as_deref().and_then(|id| o.rooms().into_iter().find(|r| r.id == id)).map(|r| r.path))
    };
    tauri::async_runtime::spawn_blocking(move || skills::list(cwd.as_deref().map(std::path::Path::new)))
        .await
        .map_err(err)
}

#[tauri::command]
async fn open_terminal(sh: State<'_, Shared>, agent_id: String) -> Res<()> {
    focus_agent(&sh, &agent_id).await.map_err(err)
}

// ---- setup ---------------------------------------------------------------------------------

#[tauri::command]
async fn install_hooks(sh: State<'_, Shared>) -> Res<()> {
    hooks::install().map_err(err)?;
    sh.office.lock().await.live.hooks_installed = hooks::installed();
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn uninstall_hooks(sh: State<'_, Shared>) -> Res<()> {
    hooks::uninstall().map_err(err)?;
    sh.office.lock().await.live.hooks_installed = hooks::installed();
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn set_host(sh: State<'_, Shared>, host: String) -> Res<()> {
    if !matches!(host.as_str(), "auto" | "orca" | "cmux") {
        return Err("Pick Automatic, Orca or cmux".into());
    }
    let mut o = sh.office.lock().await;
    o.p.host = if host == "auto" { String::new() } else { host };
    o.pick_host();
    o.save();
    drop(o);
    sh.force_structure.store(true, Ordering::Relaxed);
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn set_cmux_password(sh: State<'_, Shared>, password: String) -> Res<()> {
    let password = password.trim().to_string();
    if password.contains(char::is_whitespace) {
        return Err("cmux passwords can't contain spaces".into());
    }
    let mut o = sh.office.lock().await;
    o.p.cmux_password = password.clone();
    o.save();
    drop(o);
    // Check it straight away so Settings shows whether it worked.
    let access = match cmux::socket_path() {
        Some(_) => cmux::ping(&password).await.err().map(|e| e.to_string()),
        None => None,
    };
    sh.office.lock().await.live.cmux_access = access;
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn mark_seen(sh: State<'_, Shared>, agent_id: String) -> Res<()> {
    let mut o = sh.office.lock().await;
    let at = o.agents().iter().find(|a| a.id == agent_id).map(|a| a.updated_at).unwrap_or(0);
    o.p.seen.insert(agent_id, at);
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

/// Puts an agent's latest news back to unread, so it waits in "Needs you" or "Finished" for later.
#[tauri::command]
async fn mark_unread(sh: State<'_, Shared>, agent_id: String) -> Res<()> {
    let mut o = sh.office.lock().await;
    let at = o.agents().iter().find(|a| a.id == agent_id).map(|a| a.updated_at).unwrap_or(0);
    if at > 0 {
        o.p.seen.insert(agent_id, at - 1);
        o.save();
    }
    drop(o);
    sh.nudge();
    Ok(())
}

#[tauri::command]
async fn work(sh: State<'_, Shared>, agent_id: String) -> Res<Vec<transcript::WorkEvent>> {
    let path = sh
        .office
        .lock()
        .await
        .agents()
        .into_iter()
        .find(|a| a.id == agent_id)
        .and_then(|a| a.transcript)
        .ok_or("This agent hasn't written anything yet")?;
    tauri::async_runtime::spawn_blocking(move || transcript::read_work(&path, 4))
        .await
        .map_err(err)?
        .map_err(err)
}

/// Stops what you set going: a message still waiting its turn is taken back (the agent keeps
/// working), otherwise the agent's turn is interrupted, as Escape does in the terminal.
#[tauri::command]
async fn stop_agent(sh: State<'_, Shared>, agent_id: String) -> Res<String> {
    if agent_id.starts_with("cmux:") {
        press_escape(&sh, &agent_id).await.map_err(err)?;
        return Ok("stopped".into());
    }
    let screen = orca::read_screen(&agent_id).await.map_err(err)?;
    if !orca::has_queued(&screen) {
        orca::press_escape(&agent_id).await.map_err(err)?;
        return Ok("stopped".into());
    }
    // Up brings every queued message back into the box; then clear it a line at a time
    // (Ctrl-U empties a line, Backspace joins it to the one above).
    orca::send_keys(&agent_id, "\u{1b}[A").await.map_err(err)?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let lines = orca::message_box(&orca::read_screen(&agent_id).await.map_err(err)?).len();
    orca::send_keys(&agent_id, &"\u{15}\u{7f}".repeat((lines + 2).min(80))).await.map_err(err)?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let left = orca::message_box(&orca::read_screen(&agent_id).await.map_err(err)?);
    if !left.is_empty() {
        return Err(format!("Took your message back, but couldn't clear it from the box. Open the terminal to delete it: {}", brief::clip(&left.join(" "), 80)));
    }
    Ok("unqueued".into())
}

/// Every helper the agent has started this session, running ones first.
#[tauri::command]
async fn helpers(sh: State<'_, Shared>, agent_id: String) -> Res<Vec<transcript::Helper>> {
    let mut o = sh.office.lock().await;
    let path = o.agents().into_iter().find(|a| a.id == agent_id).and_then(|a| a.transcript).ok_or("This agent hasn't written anything yet")?;
    let tail = transcript::read_last(&path, 400_000).map_err(err)?;
    Ok(match o.live.subagents.list(&path, &tail) {
        Some(all) => all,
        None => transcript::running_helpers(&tail),
    })
}

/// One helper's own thread: what it was asked and what it did.
#[tauri::command]
async fn helper_work(sh: State<'_, Shared>, agent_id: String, helper_id: String) -> Res<Vec<transcript::WorkEvent>> {
    let path = sh.office.lock().await.agents().into_iter().find(|a| a.id == agent_id).and_then(|a| a.transcript).ok_or("This agent hasn't written anything yet")?;
    let thread = transcript::helper_path(&path, &helper_id).ok_or("That helper's thread isn't there")?;
    tauri::async_runtime::spawn_blocking(move || transcript::read_helper_work(&thread))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
async fn links(sh: State<'_, Shared>, agent_id: String) -> Res<links::Links> {
    let agent = sh.office.lock().await.agents().into_iter().find(|a| a.id == agent_id);
    let Some(path) = agent.and_then(|a| a.transcript) else { return Ok(links::Links::default()) };
    tauri::async_runtime::spawn_blocking(move || links::read_links(&path))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
fn open_link(app: AppHandle, url: String) -> Res<()> {
    // Web links, or a preview running on this Mac; nothing else leaves the app.
    let local = ["http://localhost", "http://127.0.0.1", "http://0.0.0.0"]
        .iter()
        .any(|p| url.strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/'])));
    let url = url.replacen("http://0.0.0.0", "http://localhost", 1);
    if !(url.starts_with("https://") || local) || url.contains(char::is_whitespace) {
        return Err("Only https links and previews on this Mac open from here".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(err)
}

// ---- polling -------------------------------------------------------------------------------

/// Tells you, with a macOS notification, when someone finishes or needs you. Only changes seen
/// while the office is open count, so opening the app never sets off a burst of old news.
struct Notifier {
    agents: HashMap<String, (String, i64)>,
    /// What each agent was last notified about, so the same news never goes out twice.
    told: HashMap<String, (String, String)>,
    asks: HashSet<String>,
    primed: bool,
}

impl Notifier {
    fn check(&mut self, app: &AppHandle, snap: &Snapshot) {
        let mut out: Vec<(String, String)> = vec![];
        for a in snap.agents.iter().filter(|a| a.role == "worker") {
            let now = (a.state.clone(), a.updated_at);
            let changed = self.agents.get(&a.id).is_some_and(|prev| prev.0 != now.0 || (now.1 - prev.1).abs() > 5_000);
            let headline = a.brief.as_ref().map(|b| b.headline.clone()).unwrap_or_default();
            if a.state == "working" {
                self.told.remove(&a.id);
            }
            let repeat = self.told.get(&a.id).is_some_and(|(s, h)| *s == a.state && *h == headline);
            if self.primed && changed && a.unread && !repeat {
                self.told.insert(a.id.clone(), (a.state.clone(), headline.clone()));
                match a.state.as_str() {
                    "needs_you" => out.push((format!("{} needs you", a.name), headline)),
                    "waiting" => out.push((format!("{} is waiting for your OK", a.name), headline)),
                    "done" => out.push((format!("{} finished", a.name), headline)),
                    _ => {}
                }
            }
            self.agents.insert(a.id.clone(), now);
        }
        for r in snap.requests.iter().filter(|r| r.state == "asking") {
            if self.asks.insert(r.id.clone()) && self.primed {
                out.push((format!("{} needs you", snap.cos_name), r.question.clone().unwrap_or_default()));
            }
        }
        // Wait for a snapshot with real data before treating anything as new.
        if !snap.agents.is_empty() {
            self.primed = true;
        }
        if !snap.notifications {
            return;
        }
        for (title, body) in out {
            let _ = app.notification().builder().title(title).body(body).show();
        }
    }
}

/// Errors about something you did stay up; connection errors clear once things recover.
fn sticky(e: &str) -> bool {
    ["Couldn't add", "Couldn't restart", "Couldn't bring"].iter().any(|p| e.starts_with(p))
}

async fn poll(app: AppHandle, sh: Shared) {
    let mut tick: u64 = 0;
    let mut last = String::new();
    let mut notifier = Notifier { agents: HashMap::new(), told: HashMap::new(), asks: HashSet::new(), primed: false };
    loop {
        // Every few seconds: is cmux there and letting us in, and is our hook installed?
        if tick % 10 == 0 {
            let pw = sh.office.lock().await.p.cmux_password.clone();
            let cmux_up = cmux::socket_path().is_some();
            let access = if cmux_up { cmux::ping(&pw).await.err().map(|e| e.to_string()) } else { None };
            let hooks_on = hooks::installed();
            let mut o = sh.office.lock().await;
            o.live.cmux_up = cmux_up;
            o.live.cmux_access = access;
            o.live.hooks_installed = hooks_on;
            o.live.mcp_known = mcp::configured();
            o.pick_host();
        }
        if tick % 60 == 0 || sh.force_structure.swap(false, Ordering::Relaxed) {
            let (repos, worktrees, folders) =
                tokio::join!(orca::list_repos(), orca::list_worktrees(), orca::list_folders());
            let mut o = sh.office.lock().await;
            match (repos, worktrees, folders) {
                (Ok(r), Ok(w), Ok(f)) => {
                    o.live.repos = r["repos"].as_array().cloned().unwrap_or_default();
                    o.live.worktrees = w["worktrees"].as_array().cloned().unwrap_or_default();
                    o.live.folders = f["folderWorkspaces"].as_array().cloned().unwrap_or_default();
                    o.live.orca_up = true;
                }
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                    o.live.orca_up = false;
                    if o.live.host == Host::Orca {
                        o.live.error = Some(e.to_string());
                    }
                    // Retry the structure every few seconds while Orca is away.
                    tick = tick - tick % 60 + 54;
                }
            }
            o.pick_host();
            o.refresh_descriptions();
        }
        // Plan usage, every half minute. Orca refreshes it from Anthropic when its copy is old.
        if tick % 60 == 2 {
            let (up, stale) = {
                let o = sh.office.lock().await;
                let at = o.live.usage.as_ref().and_then(|u| u.limits.as_ref()).map(|l| l.updated_at).unwrap_or(0);
                (o.live.orca_up, now_ms() - at > 180_000)
            };
            let fetched = if up { orca::accounts(stale).await.ok() } else { None };
            sh.office.lock().await.live.usage = fetched.map(|v| usage::parse(&v));
        }
        sh.office.lock().await.refresh_hooks();
        let host = sh.office.lock().await.live.host;
        if host == Host::Orca && (tick % 3 == 0 || sh.force_terminals.swap(false, Ordering::Relaxed)) {
            let terms = orca::list_terminals().await;
            let mut o = sh.office.lock().await;
            match terms {
                Ok(t) => {
                    o.live.terminals = t["terminals"].as_array().cloned().unwrap_or_default();
                    o.live.status.refresh();
                    if o.live.error.as_deref().is_some_and(|e| !sticky(e)) {
                        o.live.error = None;
                    }
                    o.reconcile();
                }
                Err(e) => o.live.error = Some(e.to_string()),
            }
        }
        if host == Host::Cmux {
            let mut o = sh.office.lock().await;
            o.sync_cmux();
            if o.live.error.as_deref().is_some_and(|e| !sticky(e)) {
                o.live.error = None;
            }
            o.reconcile();
        }

        // A sign-in terminal that has closed itself means the sign-in worked: agents whose
        // session was waiting on that server reconnect as soon as they're free.
        let reconnect_now = {
            let mut o = sh.office.lock().await;
            let listed: HashSet<String> = o.live.terminals.iter().filter_map(|t| t["handle"].as_str().map(str::to_string)).collect();
            let mut done = vec![];
            for (h, (server, seen)) in o.live.signins.iter_mut() {
                if listed.contains(h) {
                    *seen = true;
                } else if *seen {
                    done.push((h.clone(), server.clone()));
                }
            }
            for (h, server) in done {
                o.live.signins.remove(&h);
                let ids: Vec<String> = o.live.mcp_trouble.iter().filter(|(_, s)| s.contains(&server)).map(|(id, _)| id.clone()).collect();
                for id in ids {
                    o.live.reconnect.entry(id).or_default().insert(server.clone());
                }
            }
            // Only agents at rest: typed into a running turn, the command would wait as a message.
            let free: Vec<(String, Vec<String>)> = o
                .agents()
                .into_iter()
                .filter(|a| matches!(a.state.as_str(), "idle" | "done" | "needs_you") && a.ask.is_none())
                .filter_map(|a| {
                    let mut cmds = o.live.later.get(&a.id).cloned().unwrap_or_default();
                    // This Claude Code takes one server per `/mcp reconnect`; "all" isn't a name it knows.
                    cmds.extend(o.live.reconnect.get(&a.id).into_iter().flatten().map(|s| format!("/mcp reconnect {s}")));
                    (!cmds.is_empty()).then(|| (a.id.clone(), cmds))
                })
                .collect();
            let alive: HashSet<String> = o.agents().into_iter().map(|a| a.id).collect();
            let sent = |id: &String| free.iter().any(|(f, _)| f == id);
            o.live.reconnect.retain(|id, _| alive.contains(id) && !sent(id));
            o.live.later.retain(|id, _| alive.contains(id) && !sent(id));
            free
        };
        for (id, cmds) in reconnect_now {
            let s = sh.clone();
            tauri::async_runtime::spawn(async move {
                for cmd in cmds {
                    let _ = send_to(&s, &id, &cmd).await;
                    tokio::time::sleep(Duration::from_millis(600)).await;
                }
            });
        }

        if tick % 4 == 1 {
            maybe_brief(&sh).await;
        }
        // A chief of staff without the office's settings is closed straight away (it would do
        // the work itself), and whatever was waiting on it goes to a fresh one.
        let rogue = {
            let mut o = sh.office.lock().await;
            o.untrusted_cos().inspect(|h| {
                o.p.cos_handle = None;
                o.p.ignored.push(h.clone());
                o.p.briefing_pending = false;
                o.save();
            })
        };
        if let Some(h) = rogue {
            let _ = close_agent(&sh, &h).await;
            let waiting: Vec<String> =
                sh.office.lock().await.p.requests.iter().filter(|r| r.state == "sent").map(|r| r.id.clone()).collect();
            // One after another, so they all go to the same new chief of staff.
            let s = sh.clone();
            tauri::async_runtime::spawn(async move {
                for id in waiting {
                    send_to_cos(&s, id, None).await;
                }
            });
        }

        // A chief of staff started with older instructions steps aside when idle; the next
        // hand-off or rundown starts a fresh one.
        let retire = {
            let mut o = sh.office.lock().await;
            let busy = o.p.requests.iter().any(|r| matches!(r.state.as_str(), "queued" | "sent" | "routing")) || o.p.briefing_pending;
            match o.cos_alive() {
                Some(h) if o.p.cos_contract != contract::COS_VERSION && !busy => {
                    o.p.cos_handle = None;
                    o.p.cos_contract = contract::COS_VERSION;
                    o.save();
                    Some(h)
                }
                None if o.p.cos_contract != contract::COS_VERSION => {
                    o.p.cos_contract = contract::COS_VERSION;
                    o.save();
                    None
                }
                _ => None,
            }
        };
        if let Some(h) = retire {
            let _ = close_agent(&sh, &h).await;
        }

        let (snap, actions) = {
            let mut o = sh.office.lock().await;
            o.live.status.refresh();
            o.refresh_helpers();
            o.refresh_mcp();
            o.refresh_stable();
            let actions = o.take_cos_reply();
            (o.snapshot(), actions)
        };
        for a in actions {
            tauri::async_runtime::spawn(run_action(sh.clone(), a));
        }
        notifier.check(&app, &snap);
        if let Ok(json) = serde_json::to_string(&snap) {
            if json != last {
                let _ = app.emit("snapshot", &snap);
                last = json;
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            _ = sh.kick.notified() => {}
        }
        tick += 1;
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let office = Office::load(app.path().app_data_dir()?);
            office.write_contracts()?;
            let sh = Shared {
                office: Arc::new(Mutex::new(office)),
                kick: Arc::new(Notify::new()),
                force_terminals: Arc::new(AtomicBool::new(false)),
                force_structure: Arc::new(AtomicBool::new(false)),
            };
            app.manage(sh.clone());
            tauri::async_runtime::spawn(poll(app.handle().clone(), sh));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            delegate,
            choose_route,
            dismiss_request,
            answer_request,
            answer_ask,
            brief_now,
            dismiss_brief_item,
            rename_session,
            resume_session,
            forget_session,
            mcp_sign_in,
            mcp_add,
            mcp_reconnect,
            cancel_request,
            spawn_agent,
            message_agent,
            remove_agent,
            open_terminal,
            skills,
            refresh_usage,
            switch_account,
            install_hooks,
            uninstall_hooks,
            set_host,
            set_cmux_password,
            rename_cos,
            add_room,
            rename_room,
            reset_agent,
            set_notifications,
            save_attachment,
            import_attachment,
            set_room_hidden,
            move_room,
            mark_seen,
            mark_unread,
            work,
            helpers,
            helper_work,
            stop_agent,
            links,
            open_link
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agent Office");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_keep_a_safe_name() {
        let dir = std::env::temp_dir().join(format!("ao-att-{}", uuid::Uuid::new_v4()));
        let p = store_attachment(&dir, "../../etc/Plan 2027.md", b"hi").unwrap();
        assert!(p.starts_with(dir.to_str().unwrap()) && p.ends_with("/Plan 2027.md"));
        assert!(store_attachment(&dir, ".env", b"x").unwrap().ends_with("/env"));
        assert_eq!(own_attachments(&dir, vec![p.clone()]).unwrap(), vec![p]);
        assert!(own_attachments(&dir, vec!["/etc/hosts".into()]).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn files_follow_the_message() {
        assert_eq!(with_files(" Fix it ", &[]), "Fix it");
        assert_eq!(with_files("Fix it", &["/a/b.md".into()]), "Fix it\n\n[Attached file: /a/b.md]");
    }
}
