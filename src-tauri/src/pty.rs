//! The office's own terminals, for running Claude without Orca or cmux. Each one runs a program
//! straight from an argument list, never through a shell, keeps a copy of its screen, and
//! streams what it draws to any terminal panel that's open on it.

use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

pub const PREFIX: &str = "pty:";
const ROWS: u16 = 40;
const COLS: u16 = 120;

/// What a terminal app or a running Claude session marks its children with. Started from inside
/// one, the office would pass these on, and Claude would take its agents for parts of that session.
const MARKERS: [&str; 15] = [
    "CMUX_SURFACE_ID",
    "CMUX_WORKSPACE_ID",
    "ORCA_TERMINAL_HANDLE",
    "TERM_PROGRAM",
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SSE_PORT",
];

/// What a terminal panel is sent: output to draw, or the exit code once the program ends.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum Event {
    Output(String),
    Exit(u32),
}

pub type Sink = Box<dyn Fn(Event) + Send>;

#[derive(Default)]
struct Title(String);

impl vt100::Callbacks for Title {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.0 = String::from_utf8_lossy(title).into_owned();
    }
}

struct Term {
    parser: vt100::Parser<Title>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    cwd: String,
    last_output: i64,
    exit: Option<u32>,
    utility: bool,
    sinks: HashMap<u64, Sink>,
}

static TERMS: LazyLock<Mutex<HashMap<String, Term>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static SINKS: AtomicU64 = AtomicU64::new(1);

fn terms() -> std::sync::MutexGuard<'static, HashMap<String, Term>> {
    TERMS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A terminal as the office lists it.
#[derive(Clone, Debug)]
pub struct Info {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub last_output: i64,
    pub exit: Option<u32>,
    pub utility: bool,
}

pub struct Spawn {
    pub id: String,
    /// The program and its arguments.
    pub argv: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    /// A one-off command rather than an agent: kept on screen if it fails, so you can read why.
    pub utility: bool,
}

fn now_ms() -> i64 {
    crate::office::now_ms()
}

fn nz(n: u16) -> NonZeroU16 {
    NonZeroU16::new(n.max(1)).unwrap_or(NonZeroU16::MIN)
}

fn command(argv: &[String], cwd: &str, env: &[(String, String)]) -> Result<CommandBuilder> {
    let (program, args) = argv.split_first().ok_or_else(|| anyhow!("Nothing to run"))?;
    let mut cmd = CommandBuilder::new(program);
    cmd.args(args);
    cmd.cwd(cwd);
    // Started from inside another terminal app, the office would otherwise pass on its markers.
    for k in MARKERS {
        cmd.env_remove(k);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    if std::env::var("LANG").is_err() {
        cmd.env("LANG", "en_US.UTF-8");
    }
    cmd.env("PATH", login_path());
    for (k, v) in env {
        cmd.env(k, v);
    }
    Ok(cmd)
}

/// Starts a terminal. The id is chosen by the caller, so an agent keeps its id when it's
/// started again after the app restarts.
pub fn spawn(s: Spawn) -> Result<()> {
    if terms().get(&s.id).is_some_and(|t| t.exit.is_none()) {
        return Err(anyhow!("That terminal is already running"));
    }
    let pair = native_pty_system()
        .openpty(PtySize { rows: ROWS, cols: COLS, pixel_width: 0, pixel_height: 0 })
        .context("Couldn't open a terminal")?;
    let mut child = pair.slave.spawn_command(command(&s.argv, &s.cwd, &s.env)?).context("Couldn't start Claude")?;
    // Only the program keeps the terminal's other end open, so its output ends when it does.
    drop(pair.slave);
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let term = Term {
        parser: vt100::Parser::new_with_callbacks(nz(ROWS), nz(COLS), 0, Title::default()),
        writer,
        master: pair.master,
        killer: child.clone_killer(),
        cwd: s.cwd,
        last_output: now_ms(),
        exit: None,
        utility: s.utility,
        sinks: HashMap::new(),
    };
    terms().insert(s.id.clone(), term);
    let id = s.id.clone();
    std::thread::spawn(move || read_loop(id, reader));
    let id = s.id;
    std::thread::spawn(move || {
        let code = child.wait().map(|st| st.exit_code()).unwrap_or(1);
        finished(&id, code);
    });
    Ok(())
}

fn read_loop(id: String, mut reader: Box<dyn Read + Send>) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut all = terms();
        let Some(t) = all.get_mut(&id) else { break };
        t.parser.process(&buf[..n]);
        t.last_output = now_ms();
        if !t.sinks.is_empty() {
            let ev = Event::Output(b64(&buf[..n]));
            for sink in t.sinks.values() {
                sink(ev.clone());
            }
        }
    }
}

/// A program ended. A failed one-off stays on screen so you can read why; anything else goes.
fn finished(id: &str, code: u32) {
    let mut all = terms();
    let Some(t) = all.get_mut(id) else { return };
    t.exit = Some(code);
    for sink in t.sinks.values() {
        sink(Event::Exit(code));
    }
    if !(t.utility && code != 0) {
        all.remove(id);
    }
}

pub fn list() -> Vec<Info> {
    terms()
        .iter()
        .map(|(id, t)| Info {
            id: id.clone(),
            title: t.parser.callbacks().0.clone(),
            cwd: t.cwd.clone(),
            last_output: t.last_output,
            exit: t.exit,
            utility: t.utility,
        })
        .collect()
}

pub fn alive(id: &str) -> bool {
    terms().get(id).is_some_and(|t| t.exit.is_none())
}

pub fn write(id: &str, bytes: &[u8]) -> Result<()> {
    let mut all = terms();
    let t = all.get_mut(id).filter(|t| t.exit.is_none()).ok_or_else(|| anyhow!("That terminal has closed"))?;
    t.writer.write_all(bytes)?;
    t.writer.flush()?;
    Ok(())
}

/// Types a message into Claude and presses Return. It goes in as one paste, so line breaks stay
/// in the message instead of sending it early.
pub async fn send(id: &str, text: &str) -> Result<()> {
    let bracketed = terms().get(id).is_some_and(|t| t.parser.screen().bracketed_paste());
    let body = text.replace("\r\n", "\n").replace('\u{1b}', "");
    if bracketed {
        write(id, format!("\u{1b}[200~{body}\u{1b}[201~").as_bytes())?;
    } else {
        write(id, body.split('\n').map(str::trim_end).collect::<Vec<_>>().join(" ").as_bytes())?;
    }
    // Claude takes a pasted block as text; Return a moment later sends it.
    tokio::time::sleep(Duration::from_millis(150)).await;
    write(id, b"\r")
}

/// The rows on the terminal's screen right now.
pub fn screen(id: &str) -> Vec<String> {
    terms().get(id).map(|t| t.parser.screen().rows(0, t.parser.screen().size().1.get()).collect()).unwrap_or_default()
}

pub fn resize(id: &str, cols: u16, rows: u16) -> Result<()> {
    let (cols, rows) = (cols.clamp(20, 500), rows.clamp(5, 200));
    let mut all = terms();
    let t = all.get_mut(id).ok_or_else(|| anyhow!("That terminal has closed"))?;
    if t.parser.screen().size() == (nz(rows), nz(cols)) {
        return Ok(());
    }
    t.parser.set_size(nz(rows), nz(cols));
    t.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
    Ok(())
}

/// Starts streaming a terminal to a panel. Returns the panel's id and the screen as it is now,
/// so the panel can draw it before new output arrives.
pub fn attach(id: &str, sink: Sink) -> Result<(u64, String)> {
    let mut all = terms();
    let t = all.get_mut(id).ok_or_else(|| anyhow!("That terminal has closed"))?;
    let key = SINKS.fetch_add(1, Ordering::Relaxed);
    let now = b64(t.parser.screen().state_formatted().as_bytes());
    if let Some(code) = t.exit {
        sink(Event::Exit(code));
    }
    t.sinks.insert(key, sink);
    Ok((key, now))
}

pub fn detach(id: &str, key: u64) {
    if let Some(t) = terms().get_mut(id) {
        t.sinks.remove(&key);
    }
}

/// Ends the program in a terminal (a hang-up, as closing a terminal window does).
pub fn close(id: &str) {
    let mut all = terms();
    match all.get_mut(id) {
        Some(t) if t.exit.is_none() => {
            let _ = t.killer.kill();
        }
        Some(_) => {
            all.remove(id);
        }
        None => {}
    }
}

/// Ends every terminal, when the app quits.
pub fn close_all() {
    for t in terms().values_mut() {
        let _ = t.killer.kill();
    }
}

/// Whether Claude's message box is on screen: a prompt between two rules.
pub fn ready(screen: &[String]) -> bool {
    let rules: Vec<usize> = screen.iter().enumerate().filter(|(_, l)| l.trim_start().starts_with("──")).map(|(i, _)| i).collect();
    rules.windows(2).any(|w| screen[w[0] + 1..w[1]].iter().any(|l| l.trim_start().starts_with('❯')))
}

/// A screen only you can answer, which would swallow a message typed into it. Trusting a new
/// folder is one: its default answer is "No, exit".
pub fn waiting_on_you(screen: &[String]) -> Option<&'static str> {
    let has = |t: &str| screen.iter().any(|l| l.contains(t));
    if has("Yes, I trust this folder") && has("Enter to confirm") {
        Some("Claude is asking whether to trust this folder")
    } else if has("Select login method") || has("Please run /login") {
        Some("Claude needs you to sign in")
    } else if has("Choose the text style") && has("Enter to confirm") {
        Some("Claude is asking for its first-run settings")
    } else {
        None
    }
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ---- finding Claude ------------------------------------------------------------------------

/// The PATH your login shell sets up. An app opened from Finder only gets the system's, which
/// doesn't reach Claude or the tools it runs.
pub fn login_path() -> String {
    static PATH: LazyLock<String> = LazyLock::new(|| {
        let shell = std::env::var("SHELL").ok().filter(|s| s.starts_with('/')).unwrap_or_else(|| "/bin/zsh".into());
        let from_shell = std::process::Command::new(&shell)
            .args(["-lc", "printf %s \"$PATH\""])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| p.contains('/'));
        let home = dirs::home_dir().unwrap_or_default();
        let mut dirs: Vec<String> = from_shell.unwrap_or_default().split(':').filter(|d| !d.is_empty()).map(str::to_string).collect();
        for d in [
            home.join(".local/bin").display().to_string(),
            home.join(".claude/local").display().to_string(),
            "/opt/homebrew/bin".into(),
            "/usr/local/bin".into(),
            "/usr/bin".into(),
            "/bin".into(),
            "/usr/sbin".into(),
            "/sbin".into(),
        ] {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
        dirs.join(":")
    });
    PATH.clone()
}

fn executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Where `claude` is: the path you set in Settings, or the first one on your login PATH.
pub fn find_claude(custom: &str) -> Option<PathBuf> {
    let custom = custom.trim();
    if !custom.is_empty() {
        return Some(PathBuf::from(custom)).filter(|p| executable(p));
    }
    login_path().split(':').map(|d| Path::new(d).join("claude")).find(|p| executable(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(id: &str, f: impl Fn(&[String]) -> bool) -> bool {
        for _ in 0..100 {
            if f(&screen(id)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn keeps_a_failed_one_off_on_screen() {
        let id = format!("{PREFIX}test-{}", uuid::Uuid::new_v4());
        let tmp = std::env::temp_dir().display().to_string();
        spawn(Spawn {
            id: id.clone(),
            argv: vec!["/bin/sh".into(), "-c".into(), "echo first; echo second; exit 3".into()],
            cwd: tmp,
            env: vec![],
            utility: true,
        })
        .unwrap();
        assert!(wait_for(&id, |s| s.iter().any(|l| l.contains("second"))));
        for _ in 0..100 {
            if list().iter().any(|t| t.id == id && t.exit == Some(3)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(list().iter().find(|t| t.id == id).and_then(|t| t.exit), Some(3), "a failed one-off stays to be read");
        assert!(screen(&id).iter().any(|l| l.contains("first")));
        close(&id);
        assert!(!list().iter().any(|t| t.id == id));
    }

    #[test]
    fn agents_go_when_they_exit_and_take_typing() {
        let id = format!("{PREFIX}test-{}", uuid::Uuid::new_v4());
        spawn(Spawn {
            id: id.clone(),
            argv: vec!["/bin/cat".into()],
            cwd: std::env::temp_dir().display().to_string(),
            env: vec![("AGENT_OFFICE_TERMINAL".into(), id.clone())],
            utility: false,
        })
        .unwrap();
        write(&id, b"hello there\r").unwrap();
        assert!(wait_for(&id, |s| s.iter().any(|l| l.contains("hello there"))));
        let seen = std::sync::Arc::new(Mutex::new(String::new()));
        let s2 = seen.clone();
        let (key, now) = attach(&id, Box::new(move |e| {
            if let Event::Output(b) = e {
                s2.lock().unwrap().push_str(&b);
            }
        }))
        .unwrap();
        assert!(!now.is_empty());
        write(&id, b"again\r").unwrap();
        assert!(wait_for(&id, |s| s.iter().any(|l| l.contains("again"))));
        assert!(!seen.lock().unwrap().is_empty(), "an open panel gets the output");
        detach(&id, key);
        close(&id);
        for _ in 0..100 {
            if !list().iter().any(|t| t.id == id) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!list().iter().any(|t| t.id == id), "an agent's terminal goes when its program ends");
    }

    #[test]
    fn knows_when_claude_can_take_a_message() {
        let rows = |r: &[&str]| r.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        let trust = rows(&["────────", " Accessing workspace:", " ❯ No, exit", "   Yes, I trust this folder", " Enter to confirm · Esc to cancel"]);
        assert!(!ready(&trust));
        assert_eq!(waiting_on_you(&trust), Some("Claude is asking whether to trust this folder"));
        let idle = rows(&["⏺ OK", "", "──────── Probe ─", "❯", "────────", "  ⏸ manual mode on"]);
        assert!(ready(&idle) && waiting_on_you(&idle).is_none());
        let talking = rows(&["⏺ Should we trust this folder? Enter to confirm is a key hint.", "────", "❯", "────"]);
        assert!(waiting_on_you(&talking).is_none() && ready(&talking), "words in a reply aren't a prompt");
    }

    #[test]
    fn finds_programs_on_the_login_path() {
        assert!(login_path().split(':').any(|d| d == "/usr/bin"));
        assert!(find_claude("/bin/does-not-exist").is_none());
        assert_eq!(find_claude("/bin/sh"), Some(PathBuf::from("/bin/sh")));
    }
}


