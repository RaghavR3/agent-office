//! The office model. With Orca, rooms are Orca's repos and folder workspaces and agents are its
//! Claude terminals. With cmux, rooms are folders and agents are the Claude sessions our hook
//! hears from in cmux terminals. Status comes from Orca's feed and our hook, whichever is newer.

use crate::brief::{self, clip, Brief};
use crate::hooks::{HookCache, Session};
use crate::status::{PaneStatus, StatusCache};
use crate::transcript;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const DEFAULT_COS_NAME: &str = "Raggy";
pub const HQ: &str = "hq";

const NAMES: &[&str] = &[
    "Otto", "Wren", "Iris", "Hugo", "Mabel", "Ada", "Bea", "Cleo", "Dex", "Elio", "Faye", "Gus",
    "Hazel", "Ivo", "June", "Kit", "Lark", "Milo", "Nell", "Odile", "Pip", "Quill", "Rosa", "Saul",
    "Tess", "Uma", "Vera", "Wes", "Yara", "Zeno", "Arlo", "Bram", "Cass", "Dora", "Edie", "Finn",
    "Greta", "Hal", "Inez", "Jude", "Koa", "Lulu", "Mae", "Nico", "Orla", "Pax", "Remy", "Sage",
    "Theo", "Una", "Vida", "Wynn", "Alma", "Bo", "Clem", "Dot", "Esme", "Fern", "Gil", "Hollis",
];

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct Persisted {
    pub names: HashMap<String, String>,
    pub managed: HashMap<String, Managed>,
    pub cos_handle: Option<String>,
    pub cos_processed_at: i64,
    pub seen: HashMap<String, i64>,
    pub requests: Vec<CosRequest>,
    pub next_ref: u32,
    pub cos_name: String,
    /// Set on first start, until you've named the chief of staff.
    pub ask_cos_name: bool,
    /// Office-only room names, keyed by room id. Orca keeps its own names.
    pub room_names: HashMap<String, String>,
    /// Rooms left out of the office. They stay in Orca.
    pub hidden_rooms: Vec<String>,
    /// The order rooms go round the ring, clockwise from the top left.
    pub room_order: Vec<String>,
    /// The session title an agent had when you cleared it, so the stale title isn't shown as its task.
    pub cleared_titles: HashMap<String, String>,
    pub notifications_off: bool,
    /// Which terminal app to show: "auto" (Orca when it's running, else cmux), "orca" or "cmux".
    pub host: String,
    pub cmux_password: String,
    /// Where `claude` is, when it isn't on your login PATH.
    pub claude_path: String,
    /// The rooms when the office runs on cmux, as folder paths.
    pub folders: Vec<String>,
    /// Names you gave sessions, by agent; shown as what they're on.
    pub session_names: HashMap<String, String>,
    /// Sessions you named, so you can bring them back after their tab is gone.
    pub saved: Vec<SavedSession>,
    /// The chief of staff's latest rundown of what needs you.
    pub briefing: Option<Briefing>,
    /// What the last rundown was asked about, so an unchanged office isn't asked about again.
    pub briefing_key: String,
    pub briefing_sent_at: i64,
    pub briefing_pending: bool,
    pub next_brief: u32,
    pub briefs_since_clear: u32,
    /// The instructions the running chief of staff was started with.
    pub cos_contract: u32,
    /// Desks you picked for agents when hiring them, by agent.
    pub desks: HashMap<String, u32>,
    /// Each terminal's session title, so an agent can be recognised if its terminal changes.
    pub titles: HashMap<String, String>,
    /// Agents whose terminal went away (Orca restarting, say), kept a week in case the same
    /// session comes back in a new terminal.
    pub retired: Vec<Retired>,
    /// Terminals you removed that Orca couldn't close; kept off the floor until they're gone.
    pub ignored: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Retired {
    pub title: String,
    pub name: String,
    pub managed: Option<Managed>,
    pub desk: Option<u32>,
    pub session_name: Option<String>,
    pub was_cos: bool,
    pub at: i64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Briefing {
    pub summary: String,
    pub items: Vec<BriefItem>,
    pub at: i64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct BriefItem {
    /// decide | unblock | review | connect | next
    pub kind: String,
    pub text: String,
    pub agents: Vec<String>,
    pub action: Option<BriefAction>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct BriefAction {
    /// open | mcp | tell | handoff
    #[serde(rename = "type")]
    pub kind: String,
    pub agent: String,
    pub text: String,
    pub server: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SavedSession {
    /// Claude's session id.
    pub id: String,
    pub name: String,
    pub room_id: String,
    pub cwd: String,
    pub saved_at: i64,
    /// Whether an agent has it open right now (worked out for each snapshot).
    pub open: bool,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Managed {
    pub session_id: String,
    pub role: String,
    pub task: Option<String>,
    pub created_at: i64,
    pub cwd: String,
    /// The cmux workspace opened for this agent, closed again when it's removed.
    pub workspace: Option<String>,
    /// Orca's id for the process it launched. A new one means Orca relaunched the session
    /// itself (after a restart), without the settings the office started it with.
    pub incarnation: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CosRequest {
    pub id: String,
    pub text: String,
    pub created_at: i64,
    pub sent_at: i64,
    /// queued → sent → (asking → sent) → routed | replied | failed
    pub state: String,
    pub question: Option<String>,
    pub options: Vec<RouteOption>,
    pub result: Option<String>,
    pub agent_id: Option<String>,
    pub dismissed: bool,
    /// Files you attached. They go to whoever takes the task; the chief of staff only sees names.
    pub files: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct RouteOption {
    pub label: String,
    pub room: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Route {
    #[serde(rename = "ref")]
    reference: String,
    action: String,
    room: String,
    agent: String,
    area: String,
    task: String,
    question: String,
    options: Vec<RouteOption>,
    text: String,
}

pub enum Action {
    Dispatch { request: String, room: String, area: String, agent: String, task: String },
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub id: String,
    pub name: String,
    /// The name Orca uses, shown when the office name differs.
    pub original: String,
    pub path: String,
    pub kind: String,
    pub color: Option<String>,
    /// Workspaces in this repo that you've named in Orca (Building, GTM/Ops...). Branch-named ones don't count.
    pub areas: Vec<Area>,
    #[serde(skip)]
    pub selector: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Area {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub path: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub id: String,
    pub name: String,
    pub room_id: String,
    pub role: String,
    pub managed: bool,
    pub title: String,
    /// What the agent was given to do, in a few words, so you can tell who's on what.
    pub task: String,
    /// starting | working | waiting | needs_you | done | idle
    pub state: String,
    pub brief: Option<Brief>,
    pub activity: Option<String>,
    pub prompt: Option<String>,
    pub updated_at: i64,
    pub unread: bool,
    pub area: Option<String>,
    /// Subagents this agent is running right now.
    pub helpers: Vec<transcript::Helper>,
    /// Questions from Claude's own question box, waiting for an answer.
    pub ask: Option<Vec<transcript::AskQuestion>>,
    /// The desk you picked for them, if you did.
    pub desk: Option<u32>,
    /// Tool calls in its current session: how much paper is on the desk.
    pub work: u32,
    /// Claude is on a screen only you can answer in its terminal, like trusting a folder.
    pub blocked: bool,
    #[serde(skip)]
    pub transcript: Option<PathBuf>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub connected: bool,
    pub error: Option<String>,
    pub rooms: Vec<Room>,
    pub hidden_rooms: Vec<Room>,
    pub agents: Vec<AgentView>,
    pub requests: Vec<CosRequest>,
    pub cos_name: String,
    pub ask_cos_name: bool,
    pub notifications: bool,
    pub setup: Setup,
    pub usage: Option<crate::usage::Usage>,
    pub mcp: Vec<crate::mcp::Server>,
    pub saved: Vec<SavedSession>,
    pub briefing: Option<Briefing>,
    pub briefing_busy: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Setup {
    /// The terminal app the office is showing.
    pub host: &'static str,
    pub preference: String,
    pub orca: bool,
    pub cmux: bool,
    /// Whether our status hook is in ~/.claude/settings.json.
    pub hooks: bool,
    /// Why the office can't drive cmux, when it can't.
    pub cmux_access: Option<String>,
    pub cmux_password: bool,
    /// Whether Claude Code was found, for the office's own terminals.
    pub claude: bool,
    pub claude_path: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Host {
    #[default]
    Orca,
    Cmux,
    /// The office's own terminals, so nothing but Claude Code is needed.
    Builtin,
}

/// Which terminal app an agent's id belongs to.
pub fn host_of(handle: &str) -> Host {
    if handle.starts_with("cmux:") {
        Host::Cmux
    } else if handle.starts_with(crate::pty::PREFIX) {
        Host::Builtin
    } else {
        Host::Orca
    }
}

pub struct Pending {
    pub id: String,
    pub name: String,
    pub room_id: String,
    pub role: String,
    pub area: Option<String>,
    /// Set once Orca has made the terminal. The placeholder stays until Orca lists it, so the
    /// new agent never vanishes between the two.
    pub handle: Option<String>,
    pub desk: Option<u32>,
    /// When the hire started; a placeholder whose terminal never shows up is let go.
    pub since: i64,
}

#[derive(Default)]
pub struct Live {
    pub repos: Vec<Value>,
    pub worktrees: Vec<Value>,
    pub folders: Vec<Value>,
    pub terminals: Vec<Value>,
    pub status: StatusCache,
    pub error: Option<String>,
    pub pending: Vec<Pending>,
    /// Terminals created by us that haven't received their first prompt yet.
    pub starting: HashSet<String>,
    /// Agents being restarted on a newly chosen Claude account.
    pub restarting: HashSet<String>,
    pub descriptions: HashMap<String, String>,
    /// Running subagents per agent, with the transcript's length and last-changed time when read.
    pub helpers: HashMap<String, (u64, Vec<transcript::Helper>, i64)>,
    /// Each session's helpers, read from Claude Code's subagents folder.
    pub subagents: transcript::Subagents,
    /// The last thing you said to each agent, from its transcript.
    pub prompts: HashMap<String, String>,
    /// Per agent, a fingerprint of its state and message, and when that last changed.
    pub stable: HashMap<String, (u64, i64)>,
    /// Tool calls each agent's current session has made.
    pub work: HashMap<String, u32>,
    pub asks: HashMap<String, Vec<transcript::AskQuestion>>,
    /// Agents whose last turn was cut short (Esc), whatever a status feed still says.
    pub interrupted: HashSet<String>,
    pub host: Host,
    pub orca_up: bool,
    pub cmux_up: bool,
    pub cmux_access: Option<String>,
    /// Claude Code, for the office's own terminals.
    pub claude: Option<PathBuf>,
    /// Whether agents left running in the office's own terminals have been started again.
    pub builtin_resumed: bool,
    /// Agents whose Claude is stuck on a screen only you can answer (trusting a folder,
    /// signing in), with why and since when.
    pub blocked: HashMap<String, (String, i64)>,
    pub hooks_installed: bool,
    pub hooks: HookCache,
    /// Our hook's view of each agent, keyed like agents are (Orca handle or "cmux:<surface>").
    pub hook_status: HashMap<String, PaneStatus>,
    /// Per transcript: the length and time it was last read at, and what it said.
    pub digests: HashMap<PathBuf, (u64, i64, transcript::Latest)>,
    /// Claude plan usage and accounts, from Orca.
    pub usage: Option<crate::usage::Usage>,
    pub mcp: crate::mcp::Reader,
    /// MCP servers from ~/.claude.json, as (name, scope).
    pub mcp_known: Vec<(String, String)>,
    pub mcp_servers: Vec<crate::mcp::Server>,
    /// Per agent, the servers (by name) its session needs signed in, couldn't reach, or saw fail.
    pub mcp_trouble: HashMap<String, HashSet<String>>,
    /// Terminals the office opened to sign in to a server, and whether they've shown up yet.
    pub signins: HashMap<String, (String, bool)>,
    /// Servers each agent should reconnect once it's free.
    pub reconnect: HashMap<String, HashSet<String>>,
    /// Other commands to type into an agent once it's free, like `/rename`.
    pub later: HashMap<String, Vec<String>>,
    /// The office as last summarised for a rundown, and when it last changed.
    pub brief_seen: (String, i64),
    pub brief_now: bool,
}

pub struct Office {
    pub p: Persisted,
    pub live: Live,
    pub data_dir: PathBuf,
}

impl Office {
    pub fn load(data_dir: PathBuf) -> Self {
        let p = match std::fs::read_to_string(data_dir.join("office.json")) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Persisted { ask_cos_name: true, ..Default::default() },
            Err(_) => Persisted::default(),
        };
        Office { p, live: Live::default(), data_dir }
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(&self.data_dir);
        let tmp = self.data_dir.join("office.json.tmp");
        if let Ok(s) = serde_json::to_string_pretty(&self.p) {
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(tmp, self.data_dir.join("office.json"));
            }
        }
    }

    pub fn cos_name(&self) -> String {
        let n = self.p.cos_name.trim();
        if n.is_empty() { DEFAULT_COS_NAME.into() } else { n.to_string() }
    }

    pub fn write_contracts(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.attachments_dir())?;
        std::fs::write(self.contract_path("worker"), crate::contract::WORKER)?;
        std::fs::write(self.contract_path("cos"), crate::contract::cos(&self.cos_name()))
    }

    /// Where images you attach are saved. Agents started here can read it without asking.
    pub fn attachments_dir(&self) -> PathBuf {
        self.data_dir.join("attachments")
    }

    pub fn contract_path(&self, role: &str) -> PathBuf {
        self.data_dir.join(format!("{role}-contract.md"))
    }

    // ---- rooms -------------------------------------------------------------------------------

    /// The rooms in the office: your names, your order, hidden ones left out.
    pub fn rooms(&self) -> Vec<Room> {
        let mut rooms: Vec<Room> = self.all_rooms().into_iter().filter(|r| !self.p.hidden_rooms.contains(&r.id)).collect();
        for r in &mut rooms {
            if let Some(n) = self.p.room_names.get(&r.id) {
                r.name = n.clone();
            }
        }
        // Stable, so rooms you haven't placed keep Orca's order after the ones you have.
        rooms.sort_by_key(|r| self.p.room_order.iter().position(|x| x == &r.id).unwrap_or(usize::MAX));
        rooms
    }

    pub fn hidden_rooms(&self) -> Vec<Room> {
        self.all_rooms().into_iter().filter(|r| self.p.hidden_rooms.contains(&r.id)).collect()
    }

    /// Every room the terminal app knows, under its own names.
    fn all_rooms(&self) -> Vec<Room> {
        if self.live.host != Host::Orca {
            return self.folder_rooms();
        }
        let mut out = vec![];
        for r in &self.live.repos {
            let id = r["id"].as_str().unwrap_or_default();
            let path = r["path"].as_str().unwrap_or_default();
            if id.is_empty() || path.starts_with("/private/") || path.starts_with("/tmp/") {
                continue;
            }
            let main = self.live.worktrees.iter().find(|w| {
                w["repoId"] == id && (w["isMainWorktree"] == true || w["path"] == path)
            });
            let Some(main) = main else { continue };
            let mut areas: Vec<Area> = self
                .live
                .worktrees
                .iter()
                .filter(|w| w["repoId"] == id && w["displayNameMode"] == "fixed")
                .filter(|w| !w["path"].as_str().unwrap_or("").starts_with("/private/"))
                .map(|w| Area {
                    id: w["id"].as_str().unwrap_or_default().to_string(),
                    name: w["displayName"].as_str().unwrap_or_default().to_string(),
                    path: w["path"].as_str().unwrap_or_default().to_string(),
                })
                .collect();
            // The main workspace first, so it's the default when hiring.
            areas.sort_by_key(|a| a.id != main["id"].as_str().unwrap_or_default());
            out.push(Room {
                id: id.to_string(),
                name: r["displayName"].as_str().unwrap_or(id).to_string(),
                original: r["displayName"].as_str().unwrap_or(id).to_string(),
                path: path.to_string(),
                kind: "repo".into(),
                color: r["badgeColor"].as_str().map(str::to_string),
                areas,
                selector: format!("id:{}", main["id"].as_str().unwrap_or_default()),
            });
        }
        for f in &self.live.folders {
            if f["isArchived"] == true {
                continue;
            }
            let id = format!("folder:{}", f["id"].as_str().unwrap_or_default());
            let name = f["name"].as_str().unwrap_or("folder");
            out.push(Room {
                id: id.clone(),
                name: name.trim_end_matches(" workspace").to_string(),
                original: name.trim_end_matches(" workspace").to_string(),
                path: f["folderPath"].as_str().unwrap_or_default().to_string(),
                kind: "folder".into(),
                color: None,
                areas: vec![],
                selector: id,
            });
        }
        out
    }

    /// With cmux or the office's own terminals, rooms are folders: ones you added, and ones
    /// Claude has run in. A repo's git worktrees are its areas.
    fn folder_rooms(&self) -> Vec<Room> {
        self.p
            .folders
            .iter()
            .map(|path| {
                let name = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string();
                Room {
                    id: format!("dir:{path}"),
                    name: name.clone(),
                    original: name,
                    path: path.clone(),
                    kind: if Path::new(path).join(".git").exists() { "repo" } else { "folder" }.into(),
                    color: None,
                    areas: worktree_areas(path),
                    selector: String::new(),
                }
            })
            .collect()
    }

    /// The room a folder belongs to, and the area within it: the deepest match wins.
    fn folder_place(rooms: &[Room], cwd: &str) -> (String, Option<String>) {
        let within = |p: &str| cwd == p || cwd.starts_with(&format!("{p}/"));
        let area = rooms.iter().flat_map(|r| r.areas.iter().map(move |a| (r, a))).filter(|(_, a)| within(&a.path)).max_by_key(|(_, a)| a.path.len());
        if let Some((r, a)) = area {
            return (r.id.clone(), Some(a.name.clone()));
        }
        let room = rooms.iter().filter(|r| within(&r.path)).max_by_key(|r| r.path.len());
        (room.map(|r| r.id.clone()).unwrap_or_default(), None)
    }

    /// Makes a room for a folder Claude runs in, unless one already covers it.
    fn ensure_folder(&mut self, cwd: &str) -> bool {
        if cwd.is_empty() || self.p.folders.iter().any(|f| cwd == f || cwd.starts_with(&format!("{f}/"))) {
            return false;
        }
        let home = dirs::home_dir().unwrap_or_default();
        let dir = Path::new(cwd);
        // The repo it's in, if any, so a session in a subfolder doesn't get a room of its own.
        let root = dir.ancestors().take_while(|a| *a != home).find(|a| a.join(".git").exists()).unwrap_or(dir);
        // A linked worktree is an area of its repo, not a room of its own.
        let root = main_checkout(root).unwrap_or_else(|| root.to_path_buf());
        let root = root.display().to_string();
        let skip = root == home.display().to_string() || root == "/" || ["/tmp", "/private", "/var", "/Volumes"].iter().any(|p| root.starts_with(p));
        if skip || self.p.folders.contains(&root) {
            return false;
        }
        self.p.folders.push(root);
        true
    }

    pub fn add_folder(&mut self, path: &str) {
        if !self.p.folders.iter().any(|f| f == path) {
            self.p.folders.push(path.to_string());
        }
        let id = format!("dir:{path}");
        self.p.hidden_rooms.retain(|h| h != &id);
        self.save();
    }

    /// Where the chief of staff sits in the terminal app. Its room in the office is always HQ.
    pub fn hq_room(&self) -> Option<Room> {
        if self.live.host != Host::Orca {
            let home = dirs::home_dir().unwrap_or_default();
            let docs = home.join("Documents");
            let path = if docs.is_dir() { docs } else { home };
            return Some(Room {
                id: HQ.into(),
                name: "HQ".into(),
                original: "HQ".into(),
                path: path.display().to_string(),
                kind: "folder".into(),
                color: None,
                areas: vec![],
                selector: String::new(),
            });
        }
        let home_docs = dirs::home_dir().map(|h| h.join("Documents").display().to_string());
        let rooms = self.all_rooms();
        rooms
            .iter()
            .find(|r| Some(&r.path) == home_docs.as_ref())
            .or_else(|| rooms.iter().find(|r| r.kind == "folder"))
            .or_else(|| rooms.first())
            .cloned()
    }

    fn room_of_worktree(&self, worktree_id: &str) -> String {
        if worktree_id.starts_with("folder:") || worktree_id.starts_with("dir:") {
            return worktree_id.to_string();
        }
        worktree_id.split("::").next().unwrap_or_default().to_string()
    }

    /// The area an agent works in: the name you gave its Orca workspace, if you named it.
    fn area_of(&self, worktree_id: &str) -> Option<String> {
        self.live
            .worktrees
            .iter()
            .find(|w| w["id"] == worktree_id && w["displayNameMode"] == "fixed")
            .and_then(|w| w["displayName"].as_str())
            .map(str::to_string)
    }

    pub fn refresh_descriptions(&mut self) {
        for room in self.rooms() {
            if !self.live.descriptions.contains_key(&room.id) {
                let d = describe_folder(Path::new(&room.path));
                self.live.descriptions.insert(room.id, d);
            }
        }
    }

    // ---- agents ------------------------------------------------------------------------------

    pub fn pick_name(&self) -> String {
        let taken: HashSet<&str> = self
            .p
            .names
            .values()
            .map(String::as_str)
            .chain(self.live.pending.iter().map(|p| p.name.as_str()))
            .collect();
        let cos = self.cos_name();
        let free: Vec<&&str> = NAMES.iter().filter(|n| !taken.contains(**n) && **n != cos).collect();
        let seed = uuid::Uuid::new_v4().as_u128() as usize;
        match free.len() {
            0 => format!("{} {}", NAMES[seed % NAMES.len()], 2 + seed % 97),
            n => free[seed % n].to_string(),
        }
    }

    fn ensure_names(&mut self) -> bool {
        let mut changed = false;
        let terms: Vec<(String, String)> = self
            .live
            .terminals
            .iter()
            .filter(|t| self.is_agent(t))
            .filter_map(|t| Some((t["handle"].as_str()?.to_string(), clean_title(t["title"].as_str().unwrap_or_default()))))
            .collect();
        let cos = self.cos_name();
        // The chief of staff's own session, if the office lost track of it (its terminal changed).
        if self.cos_alive().is_none() {
            if let Some((h, _)) = terms.iter().find(|(_, t)| t.eq_ignore_ascii_case(&cos)) {
                self.p.names.remove(h);
                self.p.cos_handle = Some(h.clone());
                changed = true;
            }
        }
        for (h, title) in &terms {
            if self.p.cos_handle.as_deref() == Some(h) {
                continue;
            }
            let taken = |o: &Office, n: &str| terms.iter().any(|(x, _)| x != h && o.p.names.get(x).is_some_and(|m| m == n));
            if !self.p.names.contains_key(h) {
                // A session we knew, back in a new terminal: same name, desk and settings.
                let back = (!generic_title(title)).then(|| self.p.retired.iter().position(|r| &r.title == title)).flatten();
                if let Some(r) = back.map(|i| self.p.retired.remove(i)) {
                    if r.was_cos && self.cos_alive().is_none() {
                        self.p.cos_handle = Some(h.clone());
                    } else {
                        let name = if taken(self, &r.name) { self.pick_name() } else { r.name };
                        self.p.names.insert(h.clone(), name);
                        if let Some(m) = r.managed {
                            self.p.managed.insert(h.clone(), m);
                        }
                        if let Some(d) = r.desk {
                            self.p.desks.insert(h.clone(), d);
                        }
                        if let Some(n) = r.session_name {
                            self.p.session_names.insert(h.clone(), n);
                        }
                    }
                } else {
                    let n = self.pick_name();
                    self.p.names.insert(h.clone(), n);
                }
                changed = true;
            }
            // Sessions the office started are titled with the agent's name; that name wins.
            if NAMES.contains(&title.as_str()) && self.p.names.get(h) != Some(title) && !taken(self, title) {
                self.p.names.insert(h.clone(), title.clone());
                changed = true;
            }
        }
        changed
    }

    /// Drops records for terminals that no longer exist in Orca.
    fn prune(&mut self) -> bool {
        if self.live.terminals.is_empty() && self.live.error.is_some() {
            return false;
        }
        let live: HashSet<&str> =
            self.live.terminals.iter().filter_map(|t| t["handle"].as_str()).collect();
        // Agents in another terminal app keep their names for when you switch back.
        let host = self.live.host;
        let keep = |h: &str| host_of(h) != host || live.contains(h);
        // Remember whoever's leaving by their session's title, in case the session comes back.
        let now = now_ms();
        let mut leaving: Vec<String> = self.p.names.keys().filter(|h| !keep(h)).cloned().collect();
        if let Some(c) = self.p.cos_handle.clone().filter(|c| !keep(c)) {
            leaving.push(c);
        }
        for h in leaving {
            let Some(title) = self.p.titles.get(&h).cloned().filter(|t| !generic_title(t)) else { continue };
            self.p.retired.retain(|r| r.title != title);
            self.p.retired.push(Retired {
                title,
                name: self.p.names.get(&h).cloned().unwrap_or_default(),
                managed: self.p.managed.get(&h).cloned(),
                desk: self.p.desks.get(&h).copied(),
                session_name: self.p.session_names.get(&h).cloned(),
                was_cos: self.p.cos_handle.as_deref() == Some(h.as_str()),
                at: now,
            });
        }
        self.p.retired.retain(|r| now - r.at < 7 * 86_400_000);
        let extra = self.p.retired.len().saturating_sub(100);
        self.p.retired.drain(..extra);
        self.p.titles.retain(|h, _| keep(h));
        self.p.ignored.retain(|h| live.contains(h.as_str()));
        let before = self.p.names.len() + self.p.managed.len();
        self.p.names.retain(|h, _| keep(h));
        self.p.managed.retain(|h, _| keep(h));
        self.p.seen.retain(|h, _| keep(h));
        self.p.session_names.retain(|h, _| keep(h));
        self.p.desks.retain(|h, _| keep(h));
        if let Some(c) = &self.p.cos_handle {
            if !keep(c) {
                self.p.cos_handle = None;
            }
        }
        before != self.p.names.len() + self.p.managed.len()
    }

    /// Called after each terminal list refresh.
    pub fn reconcile(&mut self) {
        let listed: HashSet<String> = self.live.terminals.iter().filter_map(|t| t["handle"].as_str().map(str::to_string)).collect();
        // A hire whose terminal never turned up within two minutes isn't coming.
        let now = now_ms();
        self.live.pending.retain(|p| p.handle.as_ref().is_none_or(|h| !listed.contains(h)) && now - p.since < 120_000);
        let titles: Vec<(String, String)> = self
            .live
            .terminals
            .iter()
            .filter(|t| is_agent_terminal(t))
            .filter_map(|t| Some((t["handle"].as_str()?.to_string(), clean_title(t["title"].as_str().unwrap_or_default()))))
            .filter(|(_, t)| !generic_title(t))
            .collect();
        self.p.titles.extend(titles);
        let a = self.prune();
        let live: HashSet<String> = self.live.terminals.iter().filter_map(|t| t["handle"].as_str().map(str::to_string)).collect();
        let host = self.live.host;
        self.p.cleared_titles.retain(|h, _| host_of(h) != host || live.contains(h));
        let b = self.ensure_names();
        for t in &self.live.terminals {
            let (Some(h), Some(inc)) = (t["handle"].as_str(), t["incarnationId"].as_str()) else { continue };
            if let Some(m) = self.p.managed.get_mut(h).filter(|m| m.incarnation.is_none() && now - m.created_at < 180_000) {
                m.incarnation = Some(inc.to_string());
            }
        }
        // Sessions that were already running when we first saw them start out read, so the
        // "waiting on you" list only fills with things that happen while the office is open.
        let fresh: Vec<(String, i64)> = self
            .live
            .terminals
            .iter()
            .filter(|t| self.is_agent(t))
            .filter_map(|t| {
                let h = t["handle"].as_str()?.to_string();
                // No status yet counts as "now", so a late-loading status can't make it look new.
                (self.p.seen.get(&h).copied().unwrap_or(0) == 0 && !self.p.managed.contains_key(&h))
                    .then(|| (h, self.pane_status(t).map(|s| s.updated_at).unwrap_or_else(now_ms)))
            })
            .collect();
        let c = !fresh.is_empty();
        self.p.seen.extend(fresh);
        if a || b || c {
            self.save();
        }
    }

    /// Orca's status for the pane, unless our hook heard something clearly later that Orca missed.
    fn pane_status(&self, t: &Value) -> Option<&PaneStatus> {
        let key = format!(
            "{}:{}",
            t["tabId"].as_str().unwrap_or_default(),
            t["leafId"].as_str().unwrap_or_default()
        );
        let orca = self.live.status.panes.get(&key);
        let ours = t["handle"].as_str().and_then(|h| self.live.hook_status.get(h));
        match (orca, ours) {
            // Both see the same events a few ms apart; only a real gap means Orca missed one.
            (Some(o), Some(m)) => Some(if m.updated_at > o.updated_at + 1500 { m } else { o }),
            (o, m) => o.or(m),
        }
    }

    /// Re-reads our hook's files and works out each agent's status from them.
    pub fn refresh_hooks(&mut self) {
        self.live.hooks.refresh();
        let mut newest: HashMap<String, Session> = HashMap::new();
        for s in self.live.hooks.sessions() {
            let key = match (&s.orca, &s.surface, &s.pty) {
                (Some(h), _, _) => h.clone(),
                (None, Some(surface), _) => format!("cmux:{surface}"),
                (None, None, Some(pty)) => pty.clone(),
                _ => continue,
            };
            if newest.get(&key).is_none_or(|o| s.at_ms >= o.at_ms) {
                newest.insert(key, s.clone());
            }
        }
        let now = now_ms();
        let mut out = HashMap::new();
        let mut read = HashSet::new();
        for (key, s) in newest.into_iter().filter(|(_, s)| !s.ended()) {
            let mut st = s.status();
            if let Some(path) = st.transcript_path.clone().map(PathBuf::from) {
                let d = self.digest(&path, now);
                read.insert(path);
                if st.prompt.is_none() {
                    st.prompt = d.prompt;
                }
                // Esc doesn't send a stop event, but it does leave a note in the transcript.
                if st.state == "working" && d.interrupted {
                    st.state = "idle".into();
                    st.tool_name = None;
                    st.tool_input = None;
                }
            }
            out.insert(key, st);
        }
        self.live.digests.retain(|p, _| read.contains(p));
        self.live.hook_status = out;
    }

    /// What a transcript says now, re-read at most every 1.5s and only when it has grown.
    fn digest(&mut self, path: &Path, now: i64) -> transcript::Latest {
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if let Some((l, at, d)) = self.live.digests.get(path) {
            if *l == len || now - at < 1500 {
                return d.clone();
            }
        }
        let d = transcript::read_last(path, 256_000).map(|raw| transcript::latest(&raw)).unwrap_or_default();
        self.live.digests.insert(path.to_path_buf(), (len, now, d.clone()));
        d
    }

    /// With cmux, the agents are the Claude sessions our hook has heard from in cmux terminals.
    pub fn sync_cmux(&mut self) {
        let mut newest: HashMap<String, Session> = HashMap::new();
        for s in self.live.hooks.sessions().filter(|s| s.orca.is_none()) {
            let Some(surface) = &s.surface else { continue };
            if newest.get(surface).is_none_or(|o| s.at_ms >= o.at_ms) {
                newest.insert(surface.clone(), s.clone());
            }
        }
        let sessions: Vec<Session> = newest.into_values().filter(|s| !s.ended()).collect();
        let cos = self.p.cos_handle.clone().unwrap_or_default();
        let mut added = false;
        for s in &sessions {
            if cos != format!("cmux:{}", s.surface.as_deref().unwrap_or_default()) {
                added |= self.ensure_folder(&s.cwd);
            }
        }
        if added {
            self.save();
        }
        let rooms = self.folder_rooms();
        self.live.terminals = sessions
            .iter()
            .map(|s| {
                let (room, area) = Self::folder_place(&rooms, &s.cwd);
                json!({
                    "handle": format!("cmux:{}", s.surface.as_deref().unwrap_or_default()),
                    "agentIdentity": "claude",
                    "worktreeId": room,
                    "title": "",
                    "lastOutputAt": s.at_ms,
                    "workspace": s.workspace,
                    "cwd": s.cwd,
                    "area": area,
                })
            })
            .collect();
    }

    /// With the office's own terminals, the agents are the Claude sessions running in them.
    pub fn sync_builtin(&mut self) {
        let terms = crate::pty::list();
        let mut newest: HashMap<String, Session> = HashMap::new();
        for s in self.live.hooks.sessions() {
            let Some(id) = &s.pty else { continue };
            if newest.get(id).is_none_or(|o| s.at_ms >= o.at_ms) {
                newest.insert(id.clone(), s.clone());
            }
        }
        // A session moves on after /clear; the next launch resumes the one it's on now.
        let mut moved = false;
        for (id, s) in newest.iter().filter(|(_, s)| !s.ended()) {
            if let Some(m) = self.p.managed.get_mut(id).filter(|m| m.session_id != s.session_id) {
                m.session_id = s.session_id.clone();
                moved = true;
            }
        }
        if moved {
            self.save();
        }
        // Claude stuck on a screen only you can answer: the agent asks for you until it's past it.
        let now = now_ms();
        let was = std::mem::take(&mut self.live.blocked);
        for t in terms.iter().filter(|t| !t.utility && t.exit.is_none()) {
            if let Some(why) = crate::pty::waiting_on_you(&crate::pty::screen(&t.id)) {
                let since = was.get(&t.id).map(|(_, at)| *at).unwrap_or(now);
                self.live.blocked.insert(t.id.clone(), (why.to_string(), since));
            }
        }
        // A sign-in that failed stays on screen to be read, and doesn't count as signed in.
        for t in terms.iter().filter(|t| t.utility && t.exit.is_some_and(|c| c != 0)) {
            self.live.signins.remove(&t.id);
        }
        let rooms = self.folder_rooms();
        self.live.terminals = terms
            .iter()
            .map(|t| {
                let (room, area) = Self::folder_place(&rooms, &t.cwd);
                json!({
                    "handle": t.id,
                    "agentIdentity": if t.utility { "utility" } else { "claude" },
                    "worktreeId": room,
                    "title": t.title,
                    "lastOutputAt": t.last_output,
                    "cwd": t.cwd,
                    "area": area,
                    "connected": t.exit.is_none(),
                })
            })
            .collect();
    }

    /// The folder an agent's session runs in, which Claude files its transcript under.
    pub fn agent_cwd(&self, id: &str) -> Option<String> {
        if let Some(m) = self.p.managed.get(id).filter(|m| !m.cwd.is_empty()) {
            return Some(m.cwd.clone());
        }
        let t = self.live.terminals.iter().find(|t| t["handle"] == id)?;
        if let Some(c) = t["cwd"].as_str() {
            return Some(c.to_string());
        }
        let wt = t["worktreeId"].as_str()?;
        if let Some(f) = wt.strip_prefix("folder:") {
            return self.live.folders.iter().find(|x| x["id"] == f).and_then(|x| x["folderPath"].as_str()).map(str::to_string);
        }
        self.live.worktrees.iter().find(|w| w["id"] == wt).and_then(|w| w["path"].as_str()).map(str::to_string)
    }

    /// The cmux workspace an agent's terminal is in.
    pub fn cmux_workspace(&self, id: &str) -> Option<String> {
        self.live
            .terminals
            .iter()
            .find(|t| t["handle"] == id)
            .and_then(|t| t["workspace"].as_str())
            .map(str::to_string)
            .or_else(|| self.p.managed.get(id).and_then(|m| m.workspace.clone()))
    }

    /// Automatic: Orca when it's running, then cmux, then the office's own terminals. While
    /// agents are running in the office's own terminals, it stays on them.
    pub fn pick_host(&mut self) {
        self.live.host = match self.p.host.as_str() {
            "cmux" => Host::Cmux,
            "orca" => Host::Orca,
            "builtin" => Host::Builtin,
            _ if crate::pty::list().iter().any(|t| !t.utility) => Host::Builtin,
            _ if self.live.orca_up => Host::Orca,
            _ if self.live.cmux_up => Host::Cmux,
            _ => Host::Builtin,
        };
    }

    pub fn agents(&self) -> Vec<AgentView> {
        let mut out = vec![];
        for t in self.live.terminals.iter().filter(|t| self.is_agent(t)) {
            let handle = t["handle"].as_str().unwrap_or_default().to_string();
            let is_cos = self.p.cos_handle.as_deref() == Some(&handle);
            let managed = self.p.managed.get(&handle);
            let worktree_id = t["worktreeId"].as_str().unwrap_or_default();
            let status = self.pane_status(t);
            let title = clean_title(t["title"].as_str().unwrap_or_default());
            let quiet_ms = now_ms() - t["lastOutputAt"].as_i64().unwrap_or(0);
            let (mut state, mut brief, mut activity) = derive(status, t["title"].as_str().unwrap_or_default(), quiet_ms);
            // An interrupted turn sends no stop event, so a feed can be stuck on working or waiting.
            if matches!(state.as_str(), "working" | "waiting") && self.live.interrupted.contains(&handle) {
                state = "idle".into();
                brief = None;
                activity = None;
            }
            if self.live.starting.contains(&handle) {
                state = "starting".into();
            }
            if self.live.restarting.contains(&handle) {
                state = "starting".into();
                brief = None;
                activity = Some("Switching account".into());
            }
            // Claude is stuck on a screen only you can answer, like trusting a new folder.
            let blocked = self.live.blocked.get(&handle);
            if let Some((why, _)) = blocked {
                state = "needs_you".into();
                activity = None;
                brief = Some(Brief { status: "needs_input".into(), headline: why.clone(), ..Default::default() });
            }
            // Claude's question box shows as a permission wait; it's really a question for you.
            let ask = self.live.asks.get(&handle).filter(|_| state == "waiting").cloned();
            if let Some(qs) = &ask {
                state = "needs_you".into();
                brief = Some(Brief {
                    status: "needs_input".into(),
                    headline: if qs.len() == 1 { qs[0].question.clone() } else { format!("{} questions for you", qs.len()) },
                    ..Default::default()
                });
            }
            // A turn that ends while its helpers are still at work isn't finished; it's waiting on them.
            let running = self
                .live
                .helpers
                .get(&handle)
                .filter(|(_, h, changed)| !h.is_empty() && now_ms() - changed < 3_600_000)
                .map(|(_, h, _)| h.len())
                .unwrap_or(0);
            if state == "done" && running > 0 {
                state = "working".into();
                activity = Some(format!("Waiting on {running} {}", if running == 1 { "helper" } else { "helpers" }));
                brief = None;
            }
            // Status feeds re-stamp the time on every event, a helper's included; only a new state
            // or message counts as something new.
            let raw_at = blocked.map(|(_, at)| *at).unwrap_or_else(|| status.map(|s| s.updated_at).unwrap_or(0));
            let updated_at = match self.live.stable.get(&handle) {
                Some((sig, at)) if *sig == signature(&state, brief.as_ref()) => *at,
                _ => raw_at,
            };
            let seen = self.p.seen.get(&handle).copied().unwrap_or(0);
            // Orca's hook knows the current session (it moves on after a /clear); ours is the fallback.
            let transcript = status
                .and_then(|s| s.transcript_path.clone())
                .map(PathBuf::from)
                .or_else(|| managed.filter(|m| !m.session_id.is_empty()).map(|m| transcript::path_for(&m.cwd, &m.session_id)));
            let stale_title = self.p.cleared_titles.get(&handle).is_some_and(|t| t == &title);
            out.push(AgentView {
                name: if is_cos {
                    self.cos_name()
                } else {
                    self.p.names.get(&handle).cloned().unwrap_or_else(|| "New hire".into())
                },
                room_id: if is_cos { HQ.into() } else { self.room_of_worktree(worktree_id) },
                role: if is_cos { "cos".into() } else { "worker".into() },
                managed: managed.is_some(),
                unread: matches!(state.as_str(), "done" | "needs_you" | "waiting") && updated_at > seen,
                // Background helpers keep going after the turn ends, so these show in any state.
                helpers: self.live.helpers.get(&handle).map(|(_, h, _)| h.clone()).unwrap_or_default(),
                ask,
                desk: self.p.desks.get(&handle).copied(),
                work: self.live.work.get(&handle).copied().unwrap_or(0),
                blocked: blocked.is_some(),
                prompt: self
                    .live
                    .prompts
                    .get(&handle)
                    .cloned()
                    .or_else(|| status.and_then(|s| s.prompt.clone()))
                    .or_else(|| managed.and_then(|m| m.task.clone()))
                    .map(|p| clip(&p, 400)),
                task: if let Some(n) = self.p.session_names.get(&handle) {
                    n.clone()
                } else if stale_title {
                    String::new()
                } else {
                    // cmux doesn't share terminal titles, so its sessions go by what you last asked.
                    let asked = managed.and_then(|m| m.task.clone()).or_else(|| status.and_then(|s| s.prompt.clone()).filter(|_| title.is_empty()));
                    task_label(asked.as_deref(), &title, self.p.names.get(&handle))
                },
                area: if is_cos { None } else { self.area_of(worktree_id).or_else(|| t["area"].as_str().map(str::to_string)) },
                title,
                state,
                brief,
                activity,
                updated_at,
                transcript,
                id: handle,
            });
        }
        for p in &self.live.pending {
            out.push(AgentView {
                id: p.id.clone(),
                name: p.name.clone(),
                room_id: p.room_id.clone(),
                role: p.role.clone(),
                managed: true,
                title: String::new(),
                task: String::new(),
                state: "starting".into(),
                brief: None,
                activity: None,
                prompt: None,
                updated_at: now_ms(),
                unread: false,
                helpers: vec![],
                ask: None,
                desk: p.desk,
                work: 0,
                blocked: false,
                area: p.area.clone(),
                transcript: None,
            });
        }
        out
    }

    /// A Claude terminal that's an agent, not one the office opened to sign in to a server.
    fn is_agent(&self, t: &Value) -> bool {
        let h = t["handle"].as_str().unwrap_or_default();
        is_agent_terminal(t)
            && t["connected"] != false
            && !self.live.signins.contains_key(h)
            && !self.p.ignored.iter().any(|x| x == h)
    }

    /// Works out every MCP server's state across the agents, reading only new transcript lines.
    pub fn refresh_mcp(&mut self) {
        let agents: Vec<(String, String, String, PathBuf)> = self
            .agents()
            .into_iter()
            .filter(|a| a.role == "worker")
            .filter_map(|a| a.transcript.map(|t| (a.id, a.name, a.room_id, t)))
            .collect();
        let mut views = vec![];
        let mut trouble = HashMap::new();
        let mut work = HashMap::new();
        for (id, name, room, path) in &agents {
            if let Some(s) = self.live.mcp.read(path) {
                work.insert(id.clone(), s.tool_calls);
                let slugs: HashSet<String> = s.needs_sign_in.iter().chain(&s.failed).chain(s.last_error.keys()).cloned().collect();
                trouble.insert(id.clone(), slugs);
                views.push((name.clone(), s.clone(), room.clone()));
            }
        }
        self.live.mcp.keep(&agents.iter().map(|(_, _, _, p)| p.clone()).collect());
        let refs: Vec<(String, &crate::mcp::SessionMcp, String)> = views.iter().map(|(n, s, r)| (n.clone(), s, r.clone())).collect();
        self.live.mcp_servers = crate::mcp::summarise(&self.live.mcp_known, &refs);
        self.live.work = work;
        // `/mcp reconnect` takes the server's name, not the slug in its tool names.
        let names: HashMap<String, String> = self.live.mcp_servers.iter().map(|s| (crate::mcp::slug(&s.name), s.name.clone())).collect();
        self.live.mcp_trouble = trouble
            .into_iter()
            .map(|(id, slugs)| (id, slugs.iter().filter_map(|sl| names.get(sl).cloned()).collect()))
            .collect();
    }

    /// Re-reads running subagents and open questions, only for transcripts that have grown.
    pub fn refresh_helpers(&mut self) {
        let workers: Vec<(String, PathBuf)> = self
            .agents()
            .into_iter()
            .filter(|a| a.role == "worker")
            .filter_map(|a| a.transcript.map(|t| (a.id, t)))
            .collect();
        let ids: HashSet<&String> = workers.iter().map(|(id, _)| id).collect();
        self.live.helpers.retain(|id, _| ids.contains(id));
        self.live.prompts.retain(|id, _| ids.contains(id));
        self.live.subagents.keep(&workers.iter().map(|(_, p)| p.clone()).collect::<Vec<_>>());
        self.live.asks.retain(|id, _| ids.contains(id));
        self.live.interrupted.retain(|id| ids.contains(id));
        for (id, path) in workers {
            let Ok(len) = std::fs::metadata(&path).map(|m| m.len()) else { continue };
            if self.live.helpers.get(&id).is_some_and(|(l, _, _)| *l == len) {
                continue;
            }
            let changed = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or_else(now_ms);
            let raw = transcript::read_last(&path, 400_000).unwrap_or_default();
            match transcript::open_question(&raw) {
                Some(qs) => self.live.asks.insert(id.clone(), qs),
                None => self.live.asks.remove(&id),
            };
            let latest = transcript::latest(&raw);
            if latest.interrupted {
                self.live.interrupted.insert(id.clone());
            } else {
                self.live.interrupted.remove(&id);
            }
            // A long run of tool output can push your last message out of the tail; look further back once.
            let prompt = latest.prompt.or_else(|| {
                (!self.live.prompts.contains_key(&id))
                    .then(|| transcript::read_work(&path, 1).ok())
                    .flatten()
                    .and_then(|ev| ev.into_iter().rev().find_map(|e| match e {
                        transcript::WorkEvent::Prompt { text, .. } => Some(text),
                        _ => None,
                    }))
            });
            if let Some(p) = prompt {
                self.live.prompts.insert(id.clone(), p);
            }
            let running = match self.live.subagents.list(&path, &raw) {
                Some(all) => all.into_iter().filter(|h| h.running).collect(),
                None => transcript::running_helpers(&raw),
            };
            self.live.helpers.insert(id, (len, running, changed));
        }
    }

    /// Remembers when each agent's state or message last really changed.
    pub fn refresh_stable(&mut self) {
        let now: Vec<(String, u64, i64)> = self.agents().into_iter().map(|a| (a.id.clone(), signature(&a.state, a.brief.as_ref()), a.updated_at)).collect();
        let ids: HashSet<&String> = now.iter().map(|(id, _, _)| id).collect();
        self.live.stable.retain(|id, _| ids.contains(id));
        for (id, sig, at) in now {
            if self.live.stable.get(&id).is_none_or(|(s, _)| *s != sig) {
                self.live.stable.insert(id, (sig, at));
            }
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut requests: Vec<CosRequest> =
            self.p.requests.iter().filter(|r| !r.dismissed).cloned().collect();
        requests.reverse();
        requests.truncate(12);
        Snapshot {
            connected: match self.live.host {
                Host::Cmux => self.live.cmux_up,
                Host::Builtin => self.live.claude.is_some(),
                Host::Orca => self.live.error.is_none(),
            },
            error: self.live.error.clone(),
            rooms: self.rooms(),
            hidden_rooms: self.hidden_rooms(),
            agents: self.agents(),
            requests,
            cos_name: self.cos_name(),
            ask_cos_name: self.p.ask_cos_name,
            notifications: !self.p.notifications_off,
            usage: self.live.usage.clone(),
            mcp: self.live.mcp_servers.clone(),
            briefing: self.p.briefing.clone(),
            briefing_busy: self.p.briefing_pending,
            saved: {
                let open: HashSet<String> = self
                    .agents()
                    .into_iter()
                    .filter_map(|a| a.transcript.and_then(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string)))
                    .collect();
                let mut list: Vec<SavedSession> =
                    self.p.saved.iter().cloned().map(|mut s| { s.open = open.contains(&s.id); s }).collect();
                list.sort_by_key(|s| -s.saved_at);
                list
            },
            setup: Setup {
                host: match self.live.host {
                    Host::Cmux => "cmux",
                    Host::Builtin => "builtin",
                    Host::Orca => "orca",
                },
                preference: if self.p.host.is_empty() { "auto".into() } else { self.p.host.clone() },
                orca: self.live.orca_up,
                cmux: self.live.cmux_up,
                hooks: self.live.hooks_installed,
                cmux_access: self.live.cmux_access.clone(),
                cmux_password: !self.p.cmux_password.is_empty(),
                claude: self.live.claude.is_some(),
                claude_path: self.p.claude_path.clone(),
            },
        }
    }

    /// A chief of staff the office didn't start, or one Orca relaunched on its own: it has
    /// all of Claude's tools and none of its instructions, so it would do the work itself.
    pub fn untrusted_cos(&self) -> Option<String> {
        let h = self.cos_alive()?;
        if self.live.pending.iter().any(|p| p.handle.as_deref() == Some(h.as_str())) {
            return None;
        }
        let Some(t) = self.live.terminals.iter().find(|t| t["handle"] == h.as_str()) else { return None };
        let started_here = self.p.managed.get(&h).filter(|m| m.role == "cos");
        let relaunched = started_here
            .and_then(|m| m.incarnation.as_deref().zip(t["incarnationId"].as_str()))
            .is_some_and(|(was, now)| was != now);
        (started_here.is_none() || relaunched).then_some(h)
    }

    pub fn cos_alive(&self) -> Option<String> {
        let h = self.p.cos_handle.clone()?;
        let listed = self.live.terminals.iter().any(|t| t["handle"] == h.as_str());
        // A cmux session only shows up once it has started, so count one that's on its way.
        let arriving = self.live.pending.iter().any(|p| p.handle.as_deref() == Some(h.as_str()));
        (listed || arriving).then_some(h)
    }

    // ---- chief of staff ----------------------------------------------------------------------

    pub fn request_mut(&mut self, id: &str) -> Option<&mut CosRequest> {
        self.p.requests.iter_mut().find(|r| r.id == id)
    }

    pub fn cos_message(&self, request_id: &str) -> String {
        let Some(req) = self.p.requests.iter().find(|r| r.id == request_id) else {
            return String::new();
        };
        let agents = self.agents();
        let mut s = format!("Request {}:\n\"\"\"\n{}\n\"\"\"\n", req.id, req.text.trim());
        if !req.files.is_empty() {
            s += &format!("Attached files (Agent Office hands them to whoever takes it): {}\n", file_names(&req.files));
        }
        s += "\nRooms:\n";
        for room in self.rooms() {
            let d = self.live.descriptions.get(&room.id).cloned().unwrap_or_default();
            s += &format!("- {} ({}): {}\n", room.name, room.path, if d.is_empty() { "no description" } else { &d });
            if !room.areas.is_empty() {
                let names: Vec<&str> = room.areas.iter().map(|a| a.name.as_str()).collect();
                s += &format!("  areas: {}\n", names.join(", "));
            }
        }
        s += "\nAgents:\n";
        let workers: Vec<&AgentView> = agents.iter().filter(|a| a.role == "worker").collect();
        if workers.is_empty() {
            s += "- none yet\n";
        }
        let rooms = self.rooms();
        for a in workers {
            let room = rooms.iter().find(|r| r.id == a.room_id).map(|r| r.name.as_str()).unwrap_or("?");
            let topic = a.brief.as_ref().map(|b| b.headline.clone()).filter(|h| !h.is_empty()).unwrap_or(a.title.clone());
            let area = a.area.as_ref().map(|x| format!(" ({x})")).unwrap_or_default();
            s += &format!("- {}{} in {}: {} — {}\n", a.name, area, room, a.state, clip(&topic, 90));
        }
        s
    }

    /// Everything the chief of staff needs for a rundown, as plain lines, plus a key that only
    /// changes when something worth reporting does, and how many things need you.
    pub fn rundown(&self) -> (String, String, usize) {
        let rooms = self.rooms();
        let room_name = |id: &str| rooms.iter().find(|r| r.id == id).map(|r| r.name.clone()).unwrap_or_default();
        let mut key = String::new();
        let mut attention = 0;
        let mut s = String::from("Agents:\n");
        for a in self.agents().iter().filter(|a| a.role == "worker" && a.state != "starting") {
            let place = match &a.area {
                Some(area) => format!("{}, {area}", room_name(&a.room_id)),
                None => room_name(&a.room_id),
            };
            let on = if a.task.is_empty() { String::new() } else { format!(" on \"{}\"", clip(&a.task, 60)) };
            let headline = a.brief.as_ref().map(|b| b.headline.clone()).unwrap_or_default();
            let status = match a.state.as_str() {
                "needs_you" => {
                    attention += 1;
                    let mut l = format!("needs you: {headline}");
                    if let Some(qs) = &a.ask {
                        for q in qs.iter().take(3) {
                            let opts: Vec<&str> = q.options.iter().map(|o| o.label.as_str()).collect();
                            l += &format!(" | Q: {} [{}]", clip(&q.question, 140), opts.join(" / "));
                        }
                    } else if let Some(b) = &a.brief {
                        for q in b.questions.iter().take(3) {
                            l += &format!(" | Q: {}", clip(&q.q, 140));
                            if !q.options.is_empty() {
                                l += &format!(" [{}]", q.options.join(" / "));
                            }
                        }
                    }
                    l
                }
                "waiting" => {
                    attention += 1;
                    format!("waiting for permission: {headline} {}", a.brief.as_ref().map(|b| b.points.join(" ")).unwrap_or_default())
                }
                "done" if a.unread => {
                    attention += 1;
                    format!("finished, not read yet: {headline}")
                }
                "done" => format!("finished, already read: {headline}"),
                "working" => format!("working{}", a.activity.as_ref().map(|x| format!(": {x}")).unwrap_or_default()),
                _ => "free".into(),
            };
            let helpers = if a.helpers.is_empty() { String::new() } else { format!(" ({} helpers running)", a.helpers.len()) };
            s += &format!("- {} ({place}){on}: {}{helpers}\n", a.name, clip(status.trim(), 600));
            // Working agents change every second; only what they'd be reported for counts.
            let at = if matches!(a.state.as_str(), "needs_you" | "waiting" | "done") { a.updated_at } else { 0 };
            key += &format!("{}:{}:{}:{at}|", a.id, a.state, a.unread);
        }
        // Only servers that fail when used. One nobody has signed in to is left out: an agent
        // that needs it says so, and the Connections panel lists them.
        let failing: Vec<&crate::mcp::Server> = self.live.mcp_servers.iter().filter(|m| matches!(m.status.as_str(), "failed" | "error")).collect();
        if !failing.is_empty() {
            s += "\nConnections (MCP servers) failing when agents use them:\n";
            for m in failing {
                attention += 1;
                let who = if m.waiting.is_empty() { String::new() } else { format!(" (hit by {})", m.waiting.join(", ")) };
                s += &format!("- {}: {}{who}\n", m.name, m.detail.clone().unwrap_or_else(|| "couldn't connect".into()));
                key += &format!("mcp:{}:{}|", m.name, m.status);
            }
        }
        (key, s, attention)
    }

    /// Reads the chief of staff's latest reply and turns any `route` blocks into state changes.
    pub fn take_cos_reply(&mut self) -> Vec<Action> {
        let mut actions = vec![];
        let Some(h) = self.cos_alive() else { return actions };
        let Some(t) = self.live.terminals.iter().find(|t| t["handle"] == h.as_str()) else { return actions };
        let Some(st) = self.pane_status(t).cloned() else { return actions };
        if st.state != "done" || st.updated_at <= self.p.cos_processed_at {
            return actions;
        }
        let Some(msg) = st.last_message.clone() else { return actions };
        self.p.cos_processed_at = st.updated_at;

        let briefings: Vec<Briefing> = brief::all_fenced(&msg, "briefing").iter().filter_map(|b| serde_json::from_str(b).ok()).collect();
        if let Some(mut b) = briefings.into_iter().last() {
            b.at = now_ms();
            b.items.truncate(6);
            self.p.briefing = Some(b);
            self.p.briefing_pending = false;
            if brief::fenced(&msg, "route").is_none() {
                self.save();
                return actions;
            }
        }

        let routes: Vec<Route> = brief::all_fenced(&msg, "route")
            .iter()
            .filter_map(|b| serde_json::from_str(b).ok())
            .collect();
        let mut handled = false;
        for r in routes {
            let Some(req) = self.request_mut(&r.reference) else { continue };
            if req.state != "sent" {
                continue;
            }
            handled = true;
            match r.action.as_str() {
                "dispatch" => {
                    req.state = "routing".into();
                    actions.push(Action::Dispatch {
                        request: r.reference.clone(),
                        room: r.room,
                        area: r.area,
                        agent: r.agent,
                        task: if r.task.trim().is_empty() { req.text.clone() } else { r.task },
                    });
                }
                "ask" => {
                    req.state = "asking".into();
                    req.question = Some(r.question);
                    req.options = r.options;
                }
                _ => {
                    req.state = "replied".into();
                    req.result = Some(if r.text.is_empty() { clip(&msg, 300) } else { r.text });
                }
            }
        }
        if !handled && self.p.briefing_pending && !self.p.requests.iter().any(|r| r.state == "sent") {
            // A rundown that came back without its block; don't leave it hanging.
            self.p.briefing_pending = false;
        } else if !handled {
            // No usable block: show what the chief of staff said on the oldest waiting request.
            let sent_before = st.updated_at;
            if let Some(req) = self
                .p
                .requests
                .iter_mut()
                .find(|r| r.state == "sent" && r.sent_at <= sent_before)
            {
                req.state = "replied".into();
                req.result = Some(clip(&brief::strip_fenced(&msg, "route"), 300));
            }
        }
        self.save();
        actions
    }
}

fn signature(state: &str, brief: Option<&Brief>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    state.hash(&mut h);
    if let Some(b) = brief {
        b.headline.hash(&mut h);
        b.points.hash(&mut h);
        b.body.hash(&mut h);
    }
    h.finish()
}

/// Titles that say nothing about which session it is.
fn generic_title(t: &str) -> bool {
    t.is_empty() || t.eq_ignore_ascii_case("claude") || t.eq_ignore_ascii_case("claude code")
}

fn is_agent_terminal(t: &Value) -> bool {
    t["agentIdentity"] == "claude" && t["orphaned"] != true
}

/// Orca prefixes titles with a status glyph; the office shows status its own way.
fn clean_title(title: &str) -> String {
    let mut chars = title.chars();
    match chars.next() {
        Some(c) if !c.is_alphanumeric() && title[c.len_utf8()..].starts_with(' ') => {
            chars.as_str().trim().to_string()
        }
        _ => title.trim().to_string(),
    }
}

/// What an agent is doing right now, in plain words ("Editing sync.ts", "Running git").
fn activity(s: &PaneStatus) -> Option<String> {
    let tool = s.tool_name.as_deref()?;
    let input = s.tool_input.as_deref().unwrap_or("").trim();
    let file = || input.rsplit('/').next().unwrap_or(input).to_string();
    Some(match tool {
        "Read" => format!("Reading {}", file()),
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => format!("Editing {}", file()),
        "Grep" | "Glob" | "LS" => "Searching the code".into(),
        "WebFetch" | "WebSearch" => "Researching online".into(),
        "Agent" | "Task" => "Working with a helper agent".into(),
        "TodoWrite" => "Planning".into(),
        "Skill" => format!("Using the {} skill", clip(input, 30)),
        "Bash" => {
            let cmd = input
                .split("&&")
                .map(str::trim)
                .find(|c| !c.starts_with("cd ") && !c.is_empty())
                .and_then(|c| c.split_whitespace().find(|w| !w.contains('=')))
                .map(|w| w.rsplit('/').next().unwrap_or(w).to_string())
                .filter(|w| w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'));
            match cmd.as_deref() {
                Some("for" | "while" | "until") => "Running a loop".into(),
                Some("if" | "case" | "then" | "do" | "{" | "(") | None => "Running a command".into(),
                Some(c) => format!("Running {c}"),
            }
        }
        t if t.starts_with("mcp__") => {
            let server = t[5..].split("__").next().unwrap_or("").trim_start_matches("claude_ai_").replace(['_', '-'], " ");
            let server = server.split(' ').next().unwrap_or("a tool");
            let mut c = server.chars();
            format!("Using {}", c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default())
        }
        t => format!("Using {t}"),
    })
}

/// A short label for what an agent was asked to do.
fn task_label(task: Option<&str>, title: &str, name: Option<&String>) -> String {
    // Sessions we start are named after the agent, so their title says nothing about the work.
    let title_is_name = name.is_some_and(|n| n == title) || title.is_empty() || title == "Claude Code";
    match task {
        Some(t) if title_is_name => clip(t.lines().next().unwrap_or(t).trim(), 60),
        _ if !title_is_name => clip(title, 60),
        _ => String::new(),
    }
}

/// Claude Code sets the terminal title itself: a spinner while it works, ✳ when it's idle.
fn title_says(raw: &str) -> Option<bool> {
    let c = raw.chars().next()?;
    if c == '✳' {
        return Some(false);
    }
    let spinner = matches!(c, '◐' | '◓' | '◑' | '◒' | '✢' | '✶' | '✻' | '✽' | '·') || ('\u{2800}'..='\u{28FF}').contains(&c);
    spinner.then_some(true)
}

fn derive(status: Option<&PaneStatus>, raw_title: &str, quiet_ms: i64) -> (String, Option<Brief>, Option<String>) {
    let title = title_says(raw_title);
    let spinning = title == Some(true);
    let Some(s) = status else {
        return (if spinning { "working" } else { "idle" }.into(), None, None);
    };
    // Orca misses the stop event when a session is interrupted, leaving it "working" for good.
    // If Claude's own title says idle and the terminal has been quiet, believe the title.
    let stale = s.state == "working" && title == Some(false) && quiet_ms > 20_000;
    match s.state.as_str() {
        "working" if !stale => ("working".into(), None, activity(s)),
        "waiting" | "blocked" => {
            let tool = s.tool_name.as_deref().map(transcript::pretty_tool).unwrap_or_else(|| "a tool".into());
            let b = Brief {
                status: "needs_input".into(),
                headline: format!("Waiting for your OK to use {tool}"),
                points: s.tool_input.iter().map(|i| clip(&i.replace('\n', " "), 160)).collect(),
                ..Default::default()
            };
            ("waiting".into(), Some(b), activity(s))
        }
        _ if spinning => ("working".into(), None, activity(s)),
        _ => match &s.last_message {
            Some(m) => {
                let b = brief::from_message(m);
                let st = if matches!(b.status.as_str(), "needs_input" | "blocked") { "needs_you" } else { "done" };
                (st.into(), Some(b), None)
            }
            None => (if spinning { "working" } else { "idle" }.into(), None, None),
        },
    }
}

/// One line about what a folder is, from its CLAUDE.md / README, for the chief of staff.
fn describe_folder(path: &Path) -> String {
    for f in ["CLAUDE.md", "README.md", "AGENTS.md", "readme.md"] {
        let Ok(s) = std::fs::read_to_string(path.join(f)) else { continue };
        let mut para = String::new();
        for line in s.lines().map(str::trim) {
            let skip = line.starts_with(['#', '<', '!', '[', '|', '>', '-', '`', '*']) || line.starts_with("---");
            if line.is_empty() || skip {
                if !para.is_empty() && line.is_empty() {
                    break;
                }
                continue;
            }
            para.push_str(line);
            para.push(' ');
            if para.len() > 240 {
                break;
            }
        }
        if !para.is_empty() {
            return clip(para.trim(), 240);
        }
    }
    String::new()
}

pub fn file_names(files: &[String]) -> String {
    files.iter().map(|f| f.rsplit('/').next().unwrap_or(f)).collect::<Vec<_>>().join(", ")
}

/// A message with your attached files listed after it, the way agents receive them.
pub fn with_files(text: &str, files: &[String]) -> String {
    if files.is_empty() {
        return text.trim().to_string();
    }
    let lines: Vec<String> = files.iter().map(|f| format!("[Attached file: {f}]")).collect();
    format!("{}\n\n{}", text.trim(), lines.join("\n")).trim().to_string()
}

/// The branch a git HEAD file points at.
fn branch(head: &Path) -> Option<String> {
    let h = std::fs::read_to_string(head).ok()?;
    h.trim().strip_prefix("ref: refs/heads/").map(str::to_string)
}

/// The main checkout of a linked worktree, from the `gitdir:` line in its `.git` file.
fn main_checkout(dir: &Path) -> Option<PathBuf> {
    let link = std::fs::read_to_string(dir.join(".git")).ok()?;
    let gitdir = PathBuf::from(link.trim().strip_prefix("gitdir:")?.trim());
    let dotgit = gitdir.ancestors().find(|a| a.file_name().is_some_and(|n| n == ".git"))?;
    dotgit.parent().map(Path::to_path_buf)
}

/// A repo's git worktrees, as areas, its main checkout first. None when it has only the one.
fn worktree_areas(path: &str) -> Vec<Area> {
    let git = Path::new(path).join(".git");
    let Ok(dir) = std::fs::read_dir(git.join("worktrees")) else { return vec![] };
    let mut areas: Vec<Area> = dir
        .flatten()
        .filter_map(|e| {
            let gitdir = std::fs::read_to_string(e.path().join("gitdir")).ok()?;
            let wt = Path::new(gitdir.trim()).parent()?.to_path_buf();
            wt.is_dir().then(|| Area {
                id: format!("wt:{}", wt.display()),
                name: branch(&e.path().join("HEAD")).unwrap_or_else(|| e.file_name().to_string_lossy().into_owned()),
                path: wt.display().to_string(),
            })
        })
        .collect();
    if areas.is_empty() {
        return vec![];
    }
    areas.sort_by(|a, b| a.name.cmp(&b.name));
    let main = Area { id: format!("wt:{path}"), name: branch(&git.join("HEAD")).unwrap_or_else(|| "main".into()), path: path.to_string() };
    areas.insert(0, main);
    areas
}

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_a_name_on_first_start() {
        let dir = std::env::temp_dir().join(format!("ao-office-{}", uuid::Uuid::new_v4()));
        let o = Office::load(dir.clone());
        assert!(o.p.ask_cos_name);
        o.save();
        assert!(Office::load(dir.clone()).p.ask_cos_name, "asked again until a name is picked");
        std::fs::write(dir.join("office.json"), r#"{"cosName":""}"#).unwrap();
        assert!(!Office::load(dir.clone()).p.ask_cos_name, "an office saved before this question isn't asked");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn agents_keep_who_they_are_when_their_terminal_changes() {
        let dir = std::env::temp_dir().join(format!("ao-office-{}", uuid::Uuid::new_v4()));
        let mut o = Office::load(dir.clone());
        let term = |h: &str, title: &str| serde_json::json!({ "handle": h, "title": title, "agentIdentity": "claude", "connected": true });
        o.live.terminals = vec![term("t1", "✳ Shaper - Tier 2")];
        o.reconcile();
        let name = o.p.names["t1"].clone();
        o.p.desks.insert("t1".into(), 3);
        // Orca restarts: the same session comes back in a new terminal.
        o.live.terminals = vec![term("t2", "✳ Shaper - Tier 2")];
        o.reconcile();
        assert_eq!(o.p.names["t2"], name);
        assert_eq!(o.p.desks.get("t2"), Some(&3));
        // The chief of staff's session and an office-named session are recognised by title.
        o.live.terminals = vec![term("t2", "✳ Shaper - Tier 2"), term("t3", "✳ Raggy"), term("t4", "✳ Mae")];
        o.reconcile();
        assert_eq!(o.p.cos_handle.as_deref(), Some("t3"));
        assert_eq!(o.p.names["t4"], "Mae");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn only_a_chief_of_staff_the_office_started_is_trusted() {
        let dir = std::env::temp_dir().join(format!("ao-office-{}", uuid::Uuid::new_v4()));
        let mut o = Office::load(dir.clone());
        let term = |h: &str, inc: &str| serde_json::json!({ "handle": h, "title": "✳ Raggy", "agentIdentity": "claude", "connected": true, "incarnationId": inc });
        // A Raggy found by its title, which the office never started.
        o.live.terminals = vec![term("t1", "i1")];
        o.reconcile();
        assert_eq!(o.untrusted_cos().as_deref(), Some("t1"));
        // One the office started is fine, until Orca relaunches its process.
        o.p.managed.insert("t1".into(), Managed { role: "cos".into(), created_at: now_ms(), ..Default::default() });
        o.reconcile();
        assert_eq!(o.untrusted_cos(), None);
        o.live.terminals = vec![term("t1", "i2")];
        o.reconcile();
        assert_eq!(o.untrusted_cos().as_deref(), Some("t1"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reads_a_rundown() {
        let b: Briefing = serde_json::from_str(
            r#"{"ref":"b3","summary":"Two things need you","items":[{"kind":"unblock","text":"team-tools is failing for Edie and Theo","agents":["Edie","Theo"],"action":{"type":"mcp","server":"team-tools"}},{"kind":"review","text":"Pax finished the build"}]}"#,
        )
        .unwrap();
        assert_eq!(b.items[0].action.as_ref().unwrap().kind, "mcp");
        assert_eq!(b.items[0].agents, ["Edie", "Theo"]);
        assert!(b.items[1].action.is_none());
    }

    #[test]
    fn cleans_status_glyphs() {
        assert_eq!(clean_title("✳ ENG-396 Sync"), "ENG-396 Sync");
        assert_eq!(clean_title("◐ Outbound"), "Outbound");
        assert_eq!(clean_title("Plain title"), "Plain title");
    }

    #[test]
    fn quotes_for_shell() {
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn stale_working_trusts_the_title() {
        let s = PaneStatus { state: "working".into(), last_message: Some("All done.".into()), ..Default::default() };
        assert_eq!(derive(Some(&s), "✳ Fix the sync", 60_000).0, "done");
        assert_eq!(derive(Some(&s), "◐ Fix the sync", 60_000).0, "working");
        assert_eq!(derive(Some(&s), "✳ Fix the sync", 2_000).0, "working");
    }

    #[test]
    fn activity_reads_like_a_sentence() {
        let p = |t: &str, i: &str| PaneStatus { tool_name: Some(t.into()), tool_input: Some(i.into()), ..Default::default() };
        assert_eq!(activity(&p("Bash", "cd ~/x && git status")).unwrap(), "Running git");
        assert_eq!(activity(&p("Bash", "python3 - <<'EOF'\np=1")).unwrap(), "Running python3");
        assert_eq!(activity(&p("Edit", "/a/b/sync.ts")).unwrap(), "Editing sync.ts");
        assert_eq!(activity(&p("mcp__claude_ai_Notion__notion-update-page", "")).unwrap(), "Using Notion");
    }

    #[test]
    fn waiting_becomes_a_permission_card() {
        let s = PaneStatus { state: "waiting".into(), tool_name: Some("Bash".into()), tool_input: Some("make clean".into()), ..Default::default() };
        let (state, brief, _) = derive(Some(&s), "", 0);
        assert_eq!(state, "waiting");
        assert_eq!(brief.unwrap().headline, "Waiting for your OK to use Bash");
    }
}
