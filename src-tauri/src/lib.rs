mod brief;
mod cmux;
mod contract;
mod hooks;
mod links;
mod mcp;
mod office;
mod orca;
mod pty;
mod skills;
mod status;
mod transcript;
mod usage;

use office::{file_names, host_of, now_ms, sh_quote, with_files, Action, CosRequest, Host, Managed, Office, Pending, Snapshot, HQ};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, Notify};

#[derive(Clone)]
pub struct Shared {
    /// None only in tests, which run without a window.
    app: Option<AppHandle>,
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
// Agent ids say where they live: "cmux:<surface>" for cmux, "pty:<id>" for the office's own
// terminals, Orca's terminal handle otherwise.

async fn send_to(sh: &Shared, id: &str, text: &str) -> anyhow::Result<()> {
    match host_of(id) {
        Host::Cmux => {
            let pw = sh.office.lock().await.p.cmux_password.clone();
            cmux::send_prompt(&id[5..], text, &pw).await
        }
        Host::Builtin => {
            // Typed into a permission prompt or the folder-trust question, a message would pick
            // one of its options (Return on the trust question is "No, exit"); like Orca, hold
            // it until Claude is free. Its session has started once our hook has heard from it.
            let mut o = sh.office.lock().await;
            let held = match o.live.hook_status.get(id) {
                None => true,
                Some(s) => s.state == "waiting" || o.live.blocked.contains_key(id),
            };
            if held {
                o.live.later.entry(id.to_string()).or_default().push(text.to_string());
                return Ok(());
            }
            drop(o);
            pty::send(id, text).await
        }
        Host::Orca => orca::send_prompt(id, text).await,
    }
}

async fn close_agent(sh: &Shared, id: &str) -> anyhow::Result<()> {
    match host_of(id) {
        Host::Cmux => {
            let (pw, ws) = {
                let o = sh.office.lock().await;
                // Only close the whole workspace if the office opened it for this agent.
                (o.p.cmux_password.clone(), o.p.managed.get(id).and_then(|m| m.workspace.clone()))
            };
            cmux::close(&id[5..], ws.as_deref(), &pw).await
        }
        Host::Builtin => {
            pty::close(id);
            Ok(())
        }
        Host::Orca => orca::close_terminal(id).await,
    }
}

async fn focus_agent(sh: &Shared, id: &str) -> anyhow::Result<()> {
    match host_of(id) {
        Host::Cmux => {
            let (pw, ws) = {
                let o = sh.office.lock().await;
                (o.p.cmux_password.clone(), o.cmux_workspace(id))
            };
            cmux::focus(&id[5..], ws.as_deref(), &pw).await
        }
        // The office's own terminals open in its terminal panel.
        Host::Builtin => {
            let app = sh.app.as_ref().ok_or_else(|| anyhow::anyhow!("There's no window to open it in"))?;
            show_window(app);
            app.emit("open-terminal", id).map_err(Into::into)
        }
        Host::Orca => orca::focus_terminal(id).await,
    }
}

/// One line typed straight in, for when the terminal app's own prompt route would hold it back.
async fn type_line(sh: &Shared, id: &str, text: &str) -> anyhow::Result<()> {
    match host_of(id) {
        Host::Cmux => {
            let pw = sh.office.lock().await.p.cmux_password.clone();
            cmux::send_prompt(&id[5..], text, &pw).await
        }
        Host::Builtin => pty::send(id, text).await,
        Host::Orca => orca::type_line(id, text).await,
    }
}

async fn press_escape(sh: &Shared, id: &str) -> anyhow::Result<()> {
    match host_of(id) {
        Host::Cmux => {
            let pw = sh.office.lock().await.p.cmux_password.clone();
            cmux::press_escape(&id[5..], &pw).await
        }
        Host::Builtin => pty::write(id, b"\x1b"),
        Host::Orca => orca::press_escape(id).await,
    }
}

async fn read_screen(id: &str) -> anyhow::Result<Vec<String>> {
    match host_of(id) {
        Host::Builtin => Ok(pty::screen(id)),
        _ => orca::read_screen(id).await,
    }
}

async fn send_keys(id: &str, keys: &str) -> anyhow::Result<()> {
    match host_of(id) {
        Host::Builtin => pty::write(id, keys.as_bytes()),
        _ => orca::send_keys(id, keys).await,
    }
}

/// Waits until a new session can take a prompt. Says whether it can: in the office's own
/// terminals, Claude may first need you to trust the folder or sign in.
async fn wait_ready(sh: &Shared, handle: &str, session_id: &str) -> bool {
    match host_of(handle) {
        Host::Cmux => {
            // Our hook reports the session starting; give Claude a moment to draw its prompt.
            for _ in 0..150 {
                if sh.office.lock().await.live.hooks.started(session_id) {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            true
        }
        Host::Builtin => {
            for _ in 0..90 {
                let screen = pty::screen(handle);
                if !pty::alive(handle) || pty::waiting_on_you(&screen).is_some() {
                    return false;
                }
                // The screen alone can mislead mid-redraw; the session starting can't.
                if pty::ready(&screen) && sh.office.lock().await.live.hooks.started(session_id) {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            false
        }
        Host::Orca => {
            if orca::wait_ready(handle, 45_000).await.is_err() {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            true
        }
    }
}

/// Claude's arguments for an agent. Every value is generated here; the task text is typed into
/// Claude afterwards, never put on a command line.
fn claude_args(role: &str, start: [String; 2], contract: &str, attachments: &str, name: &str) -> Vec<String> {
    let mut args: Vec<String> = start.into();
    let rest: &[&str] = if role == "cos" {
        &["--model", "sonnet", "--permission-mode", "auto", "--tools", "", "--strict-mcp-config", "--append-system-prompt-file", contract, "-n", name]
    } else {
        &["--permission-mode", "auto", "--append-system-prompt-file", contract, "--add-dir", attachments, "--disallowedTools", "AskUserQuestion", "-n", name]
    };
    args.extend(rest.iter().map(|a| a.to_string()));
    args
}

/// Starts Claude in one of the office's own terminals, with the office's hook and status line.
fn start_builtin(o: &Office, handle: &str, cwd: &str, args: Vec<String>) -> anyhow::Result<()> {
    let claude = o.live.claude.clone().ok_or_else(|| anyhow::anyhow!("Claude Code isn't installed, or isn't where the office looks. Set its path in Settings"))?;
    let mut argv = vec![claude.display().to_string()];
    argv.extend(args);
    argv.extend(["--settings".to_string(), hooks::session_settings()]);
    let mut env = vec![("AGENT_OFFICE_TERMINAL".to_string(), handle.to_string())];
    if let Some(theirs) = hooks::your_status_line() {
        env.push(("AGENT_OFFICE_STATUSLINE".into(), theirs));
    }
    pty::spawn(pty::Spawn { id: handle.into(), argv, cwd: cwd.into(), env, utility: false })
}

/// Starts again every agent that was running in the office's own terminals when the app last
/// quit, each in its own session, under the same id, so it keeps its name and desk.
fn resume_builtin(o: &mut Office) {
    o.live.builtin_resumed = true;
    let contract = |role: &str| o.contract_path(role).display().to_string();
    let attachments = o.attachments_dir().display().to_string();
    let todo: Vec<(String, Managed, String, String)> = o
        .p
        .managed
        .iter()
        .filter(|(h, m)| host_of(h) == Host::Builtin && !pty::alive(h) && uuid::Uuid::parse_str(&m.session_id).is_ok())
        // A chief of staff only comes back if it's still the one.
        .filter(|(h, m)| m.role != "cos" || o.p.cos_handle.as_deref() == Some(h.as_str()))
        .map(|(h, m)| {
            let name = if m.role == "cos" {
                o.cos_name()
            } else {
                o.p.session_names.get(h).or_else(|| o.p.names.get(h)).cloned().unwrap_or_else(|| "Agent".into())
            };
            (h.clone(), m.clone(), name, contract(&m.role))
        })
        .collect();
    for (h, m, name, contract) in todo {
        let had_turns = transcript::path_for(&m.cwd, &m.session_id).exists();
        let flag = if had_turns { "--resume" } else { "--session-id" };
        let args = claude_args(&m.role, [flag.into(), m.session_id.clone()], &contract, &attachments, &name);
        match start_builtin(o, &h, &m.cwd, args) {
            // One that never got going (it was waiting on you to trust its folder, say) still has
            // its task to do.
            Ok(()) if !had_turns => {
                if let Some(t) = m.task.clone().filter(|t| !t.trim().is_empty()) {
                    o.live.later.entry(h.clone()).or_default().push(t);
                }
            }
            Ok(()) => {}
            Err(e) => o.live.error = Some(format!("Couldn't bring {name} back: {e}")),
        }
    }
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// ---- spawning ------------------------------------------------------------------------------

/// Opens a new Claude Code session for a room in the terminal app. Every value in the command
/// line is generated here; the task text is typed into Claude afterwards, never through a shell.
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
        Some((id, _)) => ["--resume".to_string(), id.clone()],
        None => ["--session-id".to_string(), session_id.clone()],
    };
    let pending_id = format!("pending-{session_id}");
    let (selector, cwd, name, args, host, password) = {
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
        let shown = if role == "cos" { &name } else { resume.as_ref().map(|(_, n)| n).unwrap_or(&name) };
        let args = claude_args(role, start, &contract, &attachments, shown);
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
            Some(a) => (format!("id:{}", a.id), a.path, name, args, host, password),
            None => (room.selector, room.path, name, args, host, password),
        }
    };
    sh.nudge();

    let command = format!("claude {}", args.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" "));
    let created = match host {
        Host::Orca => orca::create_terminal(&selector, &command, &name).await.map(|h| (h, None)),
        Host::Cmux => cmux::create(&cwd, &command, &name, &password).await.map(|(ws, surface)| (format!("cmux:{surface}"), Some(ws))),
        Host::Builtin => {
            let handle = format!("{}{}", pty::PREFIX, uuid::Uuid::new_v4());
            start_builtin(&*sh.office.lock().await, &handle, &cwd, args).map(|_| (handle, None))
        }
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
    let ready = wait_ready(sh, &handle, &session_id).await;
    let sent = match &task {
        Some(t) if !t.trim().is_empty() && ready => send_to(sh, &handle, t).await,
        // Claude needs you first (to trust the folder, say); the task goes in once it's free.
        Some(t) if !t.trim().is_empty() => {
            sh.office.lock().await.live.later.entry(handle.clone()).or_default().push(t.clone());
            Ok(())
        }
        _ => Ok(()),
    };
    sh.office.lock().await.live.starting.remove(&handle);
    sh.nudge();
    sent?;
    Ok(handle)
}

async fn ensure_cos(sh: &Shared) -> anyhow::Result<String> {
    {
        let mut o = sh.office.lock().await;
        if let Some(h) = o.cos_alive() {
            return Ok(h);
        }
        // One left in the office's own terminals while the office shows another app has nothing
        // worth keeping; the new one replaces it rather than leaving it running out of sight.
        if let Some(old) = o.p.cos_handle.clone().filter(|h| host_of(h) == Host::Builtin && o.live.host != Host::Builtin) {
            pty::close(&old);
            o.p.managed.remove(&old);
            o.p.cos_handle = None;
            o.save();
        }
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
async fn open_utility(sh: &Shared, args: &[String], title: &str, server: &str, room_id: Option<&str>) -> anyhow::Result<()> {
    let (host, room, password, claude) = {
        let o = sh.office.lock().await;
        let room = room_id.and_then(|id| o.rooms().into_iter().find(|r| r.id == id)).or_else(|| o.hq_room());
        (o.live.host, room.ok_or_else(|| anyhow::anyhow!("There's no folder to open a terminal in"))?, o.p.cmux_password.clone(), o.live.claude.clone())
    };
    let command = format!("claude {} && exit", args.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" "));
    let handle = match host {
        Host::Orca => orca::create_terminal(&room.selector, &command, title).await?,
        Host::Cmux => format!("cmux:{}", cmux::create(&room.path, &command, title, &password).await?.1),
        Host::Builtin => {
            let claude = claude.ok_or_else(|| anyhow::anyhow!("Claude Code isn't installed, or isn't where the office looks"))?;
            let handle = format!("{}{}", pty::PREFIX, uuid::Uuid::new_v4());
            let argv = std::iter::once(claude.display().to_string()).chain(args.iter().cloned()).collect();
            pty::spawn(pty::Spawn { id: handle.clone(), argv, cwd: room.path.clone(), env: vec![], utility: true })?;
            handle
        }
    };
    if !server.is_empty() {
        sh.office.lock().await.live.signins.insert(handle.clone(), (server.to_string(), false));
    }
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
    let args = ["mcp".to_string(), "login".into(), name.clone()];
    open_utility(&sh, &args, &format!("Sign in to {name}"), &name, room.as_deref()).await.map_err(err)
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
    // Adding needs nothing from you, so it runs out of sight; signing in gets a terminal.
    let claude = sh.office.lock().await.live.claude.clone().ok_or("Claude Code isn't installed, or isn't where the office looks")?;
    let (n, u) = (name.clone(), url.clone());
    let added = tauri::async_runtime::spawn_blocking(move || {
        std::process::Command::new(claude)
            .args(["mcp", "add", "--scope", "user", "--transport", "http", &n, &u])
            .env("PATH", pty::login_path())
            .stdin(std::process::Stdio::null())
            .output()
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    if !added.status.success() {
        let why = String::from_utf8_lossy(&added.stderr).trim().to_string();
        return Err(format!("Couldn't add {name}: {}", if why.is_empty() { "Claude Code said no".into() } else { brief::clip(&why, 200) }));
    }
    let args = ["mcp".to_string(), "login".into(), name.clone()];
    open_utility(&sh, &args, &format!("Add {name}"), &name, None).await.map_err(err)
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
        o.p.ask_cos_name = false;
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
        Host::Cmux | Host::Builtin => sh.office.lock().await.add_folder(&path),
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

#[tauri::command]
async fn set_dream_house(sh: State<'_, Shared>, on: bool) -> Res<()> {
    let mut o = sh.office.lock().await;
    o.p.dream_house_off = !on;
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
    if host_of(id) == Host::Builtin {
        return pty::alive(id);
    }
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
/// `here` counts only agents in the office's own terminals, which quitting would stop.
fn busy_names(o: &Office, here: bool) -> Vec<String> {
    let mut names: Vec<String> = o
        .agents()
        .into_iter()
        .filter(|a| a.role == "worker" && (!here || host_of(&a.id) == Host::Builtin))
        .filter(|a| matches!(a.state.as_str(), "working" | "starting" | "waiting") || a.ask.is_some() || !a.helpers.is_empty())
        .map(|a| a.name)
        .collect();
    // Out of sight while the office shows another app, but quitting stops them all the same.
    if here {
        for (id, name) in o.hidden_builtin() {
            if o.live.hook_status.get(&id).is_some_and(|s| matches!(s.state.as_str(), "working" | "waiting")) {
                names.push(name);
            }
        }
    }
    let cos_here = !here || o.cos_alive().is_some_and(|h| host_of(&h) == Host::Builtin);
    if cos_here && (o.p.requests.iter().any(|r| matches!(r.state.as_str(), "queued" | "sent" | "routing")) || o.p.briefing_pending) {
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
        if o.live.host != Host::Orca {
            return Err("Switching accounts needs Orca, which keeps the saved accounts".into());
        }
        let busy = busy_names(&o, false);
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

// ---- the office's own terminals --------------------------------------------------------------

/// Opens a terminal panel on one of the office's own terminals: what's on screen now, then
/// everything it draws until the panel closes.
#[tauri::command]
fn terminal_attach(id: String, on_event: Channel<pty::Event>) -> Res<(u64, String)> {
    if host_of(&id) != Host::Builtin {
        return Err("That terminal isn't one of the office's".into());
    }
    pty::attach(&id, Box::new(move |e| {
        let _ = on_event.send(e);
    }))
    .map_err(err)
}

#[tauri::command]
fn terminal_detach(id: String, key: u64) {
    pty::detach(&id, key);
}

#[tauri::command]
fn terminal_write(id: String, data: String) -> Res<()> {
    pty::write(&id, data.as_bytes()).map_err(err)
}

#[tauri::command]
fn terminal_resize(id: String, cols: u16, rows: u16) -> Res<()> {
    pty::resize(&id, cols, rows).map_err(err)
}

/// Lets go of a one-off terminal (a sign-in, say) when you close its panel: one that failed,
/// once you've read why, or one you've walked away from, which nothing else could reopen.
#[tauri::command]
fn terminal_dismiss(id: String) {
    if pty::is_utility(&id) {
        pty::close(&id);
    }
}

/// Signs in to Claude Code, in a terminal panel, for the office's own terminals.
#[tauri::command]
async fn claude_sign_in(sh: State<'_, Shared>) -> Res<()> {
    let args = ["auth".to_string(), "login".into()];
    open_utility(&sh, &args, "Sign in to Claude", "", None).await.map_err(err)
}

/// Quits, but first asks while agents in the office's own terminals are mid-task, since
/// quitting stops them. They all come back on the next launch.
fn request_quit(app: &AppHandle) {
    let sh = app.state::<Shared>().inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let busy = busy_names(&*sh.office.lock().await, true);
        if busy.is_empty() {
            app.exit(0);
        } else {
            show_window(&app);
            let _ = app.emit("confirm-quit", busy);
        }
    });
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

// ---- setup ---------------------------------------------------------------------------------

/// Where Claude Code is, for the office's own terminals. Empty looks on your login PATH.
#[tauri::command]
async fn set_claude_path(sh: State<'_, Shared>, path: String) -> Res<()> {
    let path = path.trim().to_string();
    if !path.is_empty() && pty::find_claude(&path).is_none() {
        return Err("There's no program at that path".into());
    }
    let mut o = sh.office.lock().await;
    o.p.claude_path = path;
    o.live.claude = pty::find_claude(&o.p.claude_path);
    o.save();
    drop(o);
    sh.nudge();
    Ok(())
}

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
    if !matches!(host.as_str(), "auto" | "orca" | "cmux" | "builtin") {
        return Err("Pick Automatic, Orca, cmux or Built-in".into());
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
    let screen = read_screen(&agent_id).await.map_err(err)?;
    if !orca::has_queued(&screen) {
        press_escape(&sh, &agent_id).await.map_err(err)?;
        return Ok("stopped".into());
    }
    // Up brings every queued message back into the box; then clear it a line at a time
    // (Ctrl-U empties a line, Backspace joins it to the one above).
    send_keys(&agent_id, "\u{1b}[A").await.map_err(err)?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let lines = orca::message_box(&read_screen(&agent_id).await.map_err(err)?).len();
    send_keys(&agent_id, &"\u{15}\u{7f}".repeat((lines + 2).min(80))).await.map_err(err)?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let left = orca::message_box(&read_screen(&agent_id).await.map_err(err)?);
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

/// Commands and messages waiting for agents that are now free, taken off their lists.
fn due_commands(o: &mut Office) -> Vec<(String, Vec<String>)> {
    // Only agents at rest: typed into a running turn, the command would wait as a message.
    let free: Vec<(String, Vec<String>)> = o
        .agents()
        .into_iter()
        .filter(|a| matches!(a.state.as_str(), "idle" | "done" | "needs_you") && a.ask.is_none())
        // In the office's own terminals, only once the session has started (our hook has heard
        // from it) and Claude's message box is showing.
        .filter(|a| {
            host_of(&a.id) != Host::Builtin
                || (o.live.hook_status.contains_key(&a.id) && !o.live.blocked.contains_key(&a.id) && pty::ready(&pty::screen(&a.id)))
        })
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
            o.live.claude = pty::find_claude(&o.p.claude_path);
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
        // Without Orca, usage comes from the status line of the office's own terminals.
        if tick % 10 == 3 {
            let mut o = sh.office.lock().await;
            if !o.live.orca_up {
                let path = hooks::usage_path();
                let at = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);
                o.live.usage = at.and_then(|at| {
                    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
                    usage::from_status_line(&v, at)
                });
            }
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
        if host == Host::Builtin {
            let mut o = sh.office.lock().await;
            if !o.live.builtin_resumed && o.live.claude.is_some() {
                resume_builtin(&mut o);
            }
            o.sync_builtin();
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
            due_commands(&mut o)
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
                app: Some(app.handle().clone()),
                office: Arc::new(Mutex::new(office)),
                kick: Arc::new(Notify::new()),
                force_terminals: Arc::new(AtomicBool::new(false)),
                force_structure: Arc::new(AtomicBool::new(false)),
            };
            app.manage(sh.clone());
            app.set_menu(menu(app.handle())?)?;
            tauri::async_runtime::spawn(poll(app.handle().clone(), sh));
            Ok(())
        })
        .on_menu_event(|app, e| {
            if e.id() == "quit" {
                request_quit(app);
            }
        })
        // Closing the window hides it, so agents in the office's own terminals keep working.
        .on_window_event(|w, e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
                let _ = w.hide();
            }
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
            set_dream_house,
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
            open_link,
            terminal_attach,
            terminal_detach,
            terminal_write,
            terminal_resize,
            terminal_dismiss,
            claude_sign_in,
            set_claude_path,
            quit_app
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Agent Office")
        .run(|app, event| match event {
            // Clicking the Dock icon brings a hidden window back.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_window(app),
            tauri::RunEvent::Exit => pty::close_all(),
            _ => {}
        });
}

/// The app menu, with Quit going through `request_quit` so it can ask first.
fn menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem as P, Submenu};
    let quit = MenuItem::with_id(app, "quit", "Quit Agent Office", true, Some("CmdOrCtrl+Q"))?;
    let app_menu = Submenu::with_items(
        app,
        "Agent Office",
        true,
        &[&P::about(app, Some("About Agent Office"), None)?, &P::separator(app)?, &P::hide(app, None)?, &P::hide_others(app, None)?, &P::show_all(app, None)?, &P::separator(app)?, &quit],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[&P::undo(app, None)?, &P::redo(app, None)?, &P::separator(app)?, &P::cut(app, None)?, &P::copy(app, None)?, &P::paste(app, None)?, &P::select_all(app, None)?],
    )?;
    let window = Submenu::with_items(app, "Window", true, &[&P::minimize(app, None)?, &P::close_window(app, None)?])?;
    Menu::with_items(app, &[&app_menu, &edit, &window])
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

    /// A hire in the office's own terminals, in a folder Claude hasn't been trusted with: the
    /// task waits while Claude asks, then goes in once you've answered. Runs real Claude Code
    /// (one short message on your plan): `cargo test hire_end_to_end -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn hire_end_to_end() {
        let base = std::env::temp_dir().join(format!("ao-e2e-{}", uuid::Uuid::new_v4()));
        let room = base.join("room");
        std::fs::create_dir_all(&room).unwrap();
        let room = room.canonicalize().unwrap().display().to_string();
        let mut office = Office::load(base.join("data"));
        office.p.host = "builtin".into();
        office.p.cos_name = "Raggy".into();
        office.p.folders = vec![room.clone()];
        office.live.claude = pty::find_claude("");
        office.write_contracts().unwrap();
        office.pick_host();
        assert_eq!(office.live.host, Host::Builtin);
        let sh = Shared {
            app: None,
            office: Arc::new(Mutex::new(office)),
            kick: Arc::new(Notify::new()),
            force_terminals: Arc::new(AtomicBool::new(false)),
            force_structure: Arc::new(AtomicBool::new(false)),
        };
        let tick = || async {
            let mut o = sh.office.lock().await;
            o.refresh_hooks();
            o.sync_builtin();
            o.reconcile();
            o.refresh_helpers();
            due_commands(&mut o)
        };
        let task = "Reply with just the word OK, then your brief.";
        let id = spawn(&sh, &format!("dir:{room}"), None, "worker", Some(task.into()), None, None).await.unwrap();
        tick().await;
        {
            let o = sh.office.lock().await;
            let a = o.agents().into_iter().find(|a| a.id == id).unwrap();
            eprintln!("before trusting: {} {:?}", a.state, a.brief.as_ref().map(|b| &b.headline));
            assert_eq!(a.state, "needs_you", "Claude asking to trust the folder needs you");
            assert_eq!(o.live.later.get(&id).map(|l| l.len()), Some(1), "the task waits");
        }
        // You open the terminal and trust the folder: down to "Yes", then Return.
        for _ in 0..40 {
            if pty::screen(&id).iter().any(|l| l.trim_start().starts_with("❯") && l.contains("Yes")) {
                break;
            }
            pty::write(&id, b"\x1b[B").unwrap();
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        pty::write(&id, b"\r").unwrap();
        let mut done = None;
        for _ in 0..120 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            for (to, cmds) in tick().await {
                for c in cmds {
                    send_to(&sh, &to, &c).await.unwrap();
                }
            }
            let o = sh.office.lock().await;
            let a = o.agents().into_iter().find(|a| a.id == id).unwrap();
            if a.state == "done" {
                done = Some(a);
                break;
            }
        }
        let a = done.expect("the agent finished");
        eprintln!("finished: {} {:?} task={:?} prompt={:?}", a.name, a.brief.as_ref().map(|b| &b.headline), a.task, a.prompt);
        assert_eq!(a.prompt.as_deref(), Some(task));
        let m = sh.office.lock().await.p.managed.get(&id).cloned().unwrap();
        assert!(transcript::path_for(&m.cwd, &m.session_id).exists(), "resuming will find its session");

        // You quit; on the next launch the same agent comes back in its own session.
        pty::close(&id);
        for _ in 0..40 {
            if !pty::alive(&id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        resume_builtin(&mut *sh.office.lock().await);
        assert!(pty::alive(&id), "started again under the same id");
        let mut back = None;
        for _ in 0..60 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            tick().await;
            let o = sh.office.lock().await;
            if pty::ready(&pty::screen(&id)) {
                back = o.agents().into_iter().find(|a| a.id == id);
                break;
            }
        }
        let back = back.expect("the agent is back at its desk");
        eprintln!("back: {} {} prompt={:?}", back.name, back.state, back.prompt);
        assert_eq!(back.name, a.name);
        assert!(pty::screen(&id).iter().any(|l| l.contains("Reply with just the word OK")), "with its conversation");
        pty::close(&id);
        std::fs::remove_dir_all(&base).ok();
    }

    /// Showing another app, the office keeps agents in its own terminals in view as names, and a
    /// new chief of staff there replaces one left running in them.
    #[tokio::test]
    async fn built_in_agents_out_of_sight() {
        let base = std::env::temp_dir().join(format!("ao-away-{}", uuid::Uuid::new_v4()));
        let worker = format!("{}away-w-{}", pty::PREFIX, uuid::Uuid::new_v4());
        let cos = format!("{}away-c-{}", pty::PREFIX, uuid::Uuid::new_v4());
        for id in [&worker, &cos] {
            let argv = vec!["/bin/cat".to_string()];
            pty::spawn(pty::Spawn { id: id.clone(), argv, cwd: std::env::temp_dir().display().to_string(), env: vec![], utility: false }).unwrap();
        }
        let mut office = Office::load(base.clone());
        office.p.cos_name = "Penny".into();
        office.p.names.insert(worker.clone(), "Jude".into());
        office.p.cos_handle = Some(cos.clone());
        office.p.managed.insert(cos.clone(), Managed { role: "cos".into(), ..Default::default() });
        office.live.host = Host::Builtin;
        assert!(office.hidden_builtin().is_empty(), "in view on Built-in");
        office.live.host = Host::Cmux;
        // Other tests run terminals of their own at the same time; only these two count here.
        let mut names: Vec<String> = office.hidden_builtin().into_iter().filter(|(id, _)| *id == worker || *id == cos).map(|(_, n)| n).collect();
        names.sort();
        assert_eq!(names, ["Jude", "Penny"]);
        let sh = Shared {
            app: None,
            office: Arc::new(Mutex::new(office)),
            kick: Arc::new(Notify::new()),
            force_terminals: Arc::new(AtomicBool::new(false)),
            force_structure: Arc::new(AtomicBool::new(false)),
        };
        // cmux has no hook here, so the new chief of staff can't start; the old one still goes.
        assert!(ensure_cos(&sh).await.is_err());
        for _ in 0..40 {
            if !pty::alive(&cos) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!pty::alive(&cos), "the old chief of staff is closed");
        assert!(pty::alive(&worker), "workers keep going");
        let o = sh.office.lock().await;
        assert!(o.p.cos_handle.is_none() && !o.p.managed.contains_key(&cos));
        drop(o);
        pty::close(&worker);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn files_follow_the_message() {
        assert_eq!(with_files(" Fix it ", &[]), "Fix it");
        assert_eq!(with_files("Fix it", &["/a/b.md".into()]), "Fix it\n\n[Attached file: /a/b.md]");
    }
}
