import { Channel, invoke } from "@tauri-apps/api/core";

export type AgentState = "starting" | "working" | "waiting" | "needs_you" | "done" | "idle";

export interface Brief {
  status: string;
  headline: string;
  points: string[];
  questions: { q: string; options: string[] }[];
  links: { label: string; url: string }[];
  /** The whole message, when the agent didn't write a short brief. */
  body?: string | null;
}

export interface Room {
  id: string;
  name: string;
  /** Orca's name for it, when you've renamed it in the office. */
  original: string;
  path: string;
  kind: "repo" | "folder";
  color: string | null;
  /** The Orca workspaces in this room that you've named, main one first. */
  areas: { id: string; name: string }[];
}

export interface Skill {
  name: string;
  description: string;
  source: "project" | "you" | "plugin" | "built-in";
}

export interface AskQuestion {
  question: string;
  header: string;
  multiSelect: boolean;
  options: { label: string; description: string }[];
}

export interface Agent {
  id: string;
  name: string;
  roomId: string;
  role: "cos" | "worker";
  managed: boolean;
  title: string;
  task: string;
  state: AgentState;
  brief: Brief | null;
  activity: string | null;
  prompt: string | null;
  updatedAt: number;
  unread: boolean;
  /** The named Orca workspace this agent works in, like "GTM/Ops". */
  area: string | null;
  /** Subagents running for this agent right now. */
  helpers: Helper[];
  /** Questions from Claude's own question box, waiting for you. */
  ask: AskQuestion[] | null;
  /** The desk you picked for them when hiring. */
  desk: number | null;
  /** Tool calls in their current session: the papers on their desk. */
  work: number;
  /** Claude is on a screen only you can answer in its terminal, like trusting a folder. */
  blocked: boolean;
}

export interface CosRequest {
  id: string;
  text: string;
  createdAt: number;
  sentAt: number;
  state: "queued" | "sent" | "asking" | "routing" | "routed" | "replied" | "failed" | "cancelled";
  question: string | null;
  options: { label: string; room: string }[];
  result: string | null;
  agentId: string | null;
  /** Attached files, handed to whoever takes the task. */
  files: string[];
}

export interface Setup {
  /** The terminal app the office is showing: Orca, cmux, or its own terminals. */
  host: "orca" | "cmux" | "builtin";
  /** "auto" shows Orca when it's running, then cmux, then the office's own terminals. */
  preference: "auto" | "orca" | "cmux" | "builtin";
  orca: boolean;
  cmux: boolean;
  /** Whether Agent Office's own status hook is installed. */
  hooks: boolean;
  /** Why the office can't send commands to cmux, when it can't. */
  cmuxAccess: string | null;
  cmuxPassword: boolean;
  /** Whether Claude Code was found, for the office's own terminals. */
  claude: boolean;
  /** Where you told the office Claude Code is; empty means your login PATH. */
  claudePath: string;
  /** Agents still running in the office's own terminals while it shows another app, out of sight. */
  elsewhere: string[];
}

/** What a terminal panel is sent: output to draw (base64), or the exit code once it ends. */
export type TerminalEvent = { output: string } | { exit: number };

export interface UsageWindow {
  /** Percent of the limit used, 0 to 100. */
  used: number;
  resetsAt: number;
}

export interface Limits {
  /** The rolling five-hour limit. */
  session: UsageWindow | null;
  week: UsageWindow | null;
  fable: UsageWindow | null;
  updatedAt: number;
  error: string | null;
}

export interface ClaudeAccount {
  id: string;
  name: string;
  email: string;
  active: boolean;
  limits: Limits | null;
}

export interface Usage {
  accounts: ClaudeAccount[];
  limits: Limits | null;
}

export interface McpServer {
  /** The name Claude Code knows it by. */
  name: string;
  label: string;
  /** "you" (all sessions), "claude.ai", "folder", "plugin" or "other". */
  scope: string;
  status: "connected" | "needs_sign_in" | "failed" | "error" | "idle";
  detail: string | null;
  /** Agents whose session needs it signed in. */
  waiting: string[];
  room: string | null;
}

export interface SavedSession {
  /** Claude's session id. */
  id: string;
  name: string;
  roomId: string;
  savedAt: number;
  /** An agent has it open right now. */
  open: boolean;
}

export interface BriefItem {
  kind: "decide" | "unblock" | "review" | "connect" | "next" | string;
  text: string;
  agents: string[];
  action: { type: "open" | "mcp" | "reconnect" | "tell" | "handoff" | string; agent: string; text: string; server: string } | null;
}

export interface Briefing {
  summary: string;
  items: BriefItem[];
  at: number;
}

export interface Snapshot {
  connected: boolean;
  error: string | null;
  rooms: Room[];
  hiddenRooms: Room[];
  agents: Agent[];
  requests: CosRequest[];
  cosName: string;
  /** First start: the chief of staff hasn't been named yet. */
  askCosName: boolean;
  notifications: boolean;
  setup: Setup;
  /** Claude plan usage and saved accounts, from Orca. */
  usage: Usage | null;
  /** MCP servers across all agents. */
  mcp: McpServer[];
  /** Sessions you named, newest first. */
  saved: SavedSession[];
  /** The chief of staff's latest rundown of what needs you. */
  briefing: Briefing | null;
  briefingBusy: boolean;
}

export interface LinkItem {
  kind: string;
  label: string;
  url: string;
}

export interface Links {
  top: LinkItem[];
  more: LinkItem[];
}

/** A subagent the agent started. Its thread can be read with `api.helperWork`. */
export interface Helper {
  /** Empty when Claude Code didn't keep the helper's thread. */
  id: string;
  task: string;
  kind: string;
  running: boolean;
  startedAt: number;
  updatedAt: number;
}

export type WorkEvent =
  /** `queued`: sent while the agent was busy and not read yet. */
  | { kind: "prompt"; text: string; ts: string; queued?: boolean }
  | { kind: "said"; text: string; ts: string }
  | { kind: "tool"; name: string; detail: string; ts: string };

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  delegate: (text: string, files: string[]) => invoke<string>("delegate", { text, files }),
  chooseRoute: (requestId: string, label: string, room: string) =>
    invoke<void>("choose_route", { requestId, label, room }),
  dismissRequest: (requestId: string) => invoke<void>("dismiss_request", { requestId }),
  briefNow: () => invoke<void>("brief_now"),
  dismissBriefItem: (index: number) => invoke<void>("dismiss_brief_item", { index }),
  cancelRequest: (requestId: string) => invoke<void>("cancel_request", { requestId }),
  answerRequest: (requestId: string, text: string, files: string[]) =>
    invoke<void>("answer_request", { requestId, text, files }),
  spawnAgent: (roomId: string, area: string | null, task: string | null, desk: number | null = null) =>
    invoke<void>("spawn_agent", { roomId, area, task, desk }),
  messageAgent: (agentId: string, text: string) => invoke<void>("message_agent", { agentId, text }),
  removeAgent: (agentId: string) => invoke<void>("remove_agent", { agentId }),
  resetAgent: (agentId: string) => invoke<void>("reset_agent", { agentId }),
  renameSession: (agentId: string, name: string) => invoke<void>("rename_session", { agentId, name }),
  resumeSession: (sessionId: string) => invoke<void>("resume_session", { sessionId }),
  forgetSession: (sessionId: string) => invoke<void>("forget_session", { sessionId }),
  answerAsk: (agentId: string, answers: string[]) => invoke<void>("answer_ask", { agentId, answers }),
  setNotifications: (on: boolean) => invoke<void>("set_notifications", { on }),
  openTerminal: (agentId: string) => invoke<void>("open_terminal", { agentId }),
  refreshUsage: () => invoke<void>("refresh_usage"),
  mcpSignIn: (name: string) => invoke<void>("mcp_sign_in", { name }),
  mcpAdd: (name: string, url: string) => invoke<void>("mcp_add", { name, url }),
  mcpReconnect: () => invoke<void>("mcp_reconnect"),
  switchAccount: (accountId: string) => invoke<void>("switch_account", { accountId }),
  installHooks: () => invoke<void>("install_hooks"),
  uninstallHooks: () => invoke<void>("uninstall_hooks"),
  setHost: (host: Setup["preference"]) => invoke<void>("set_host", { host }),
  setCmuxPassword: (password: string) => invoke<void>("set_cmux_password", { password }),
  markSeen: (agentId: string) => invoke<void>("mark_seen", { agentId }),
  markUnread: (agentId: string) => invoke<void>("mark_unread", { agentId }),
  renameCos: (name: string) => invoke<string>("rename_cos", { name }),
  addRoom: (path: string) => invoke<void>("add_room", { path }),
  renameRoom: (roomId: string, name: string) => invoke<void>("rename_room", { roomId, name }),
  setRoomHidden: (roomId: string, hidden: boolean) => invoke<void>("set_room_hidden", { roomId, hidden }),
  moveRoom: (roomId: string, delta: number) => invoke<void>("move_room", { roomId, delta }),
  work: (agentId: string) => invoke<WorkEvent[]>("work", { agentId }),
  helpers: (agentId: string) => invoke<Helper[]>("helpers", { agentId }),
  helperWork: (agentId: string, helperId: string) => invoke<WorkEvent[]>("helper_work", { agentId, helperId }),
  /** "unqueued" when a message still waiting its turn was taken back, "stopped" when the turn was interrupted. */
  stopAgent: (agentId: string) => invoke<"stopped" | "unqueued">("stop_agent", { agentId }),
  links: (agentId: string) => invoke<Links>("links", { agentId }),
  saveAttachment: (name: string, data: string) => invoke<string>("save_attachment", { name, data }),
  importAttachment: (path: string) => invoke<string>("import_attachment", { path }),
  openLink: (url: string) => invoke<void>("open_link", { url }),
  skills: (agentId: string | null, roomId: string | null) => invoke<Skill[]>("skills", { agentId, roomId }),
  /** Starts streaming one of the office's own terminals. Resolves to the panel's key and the screen as it is now. */
  terminalAttach: (id: string, onEvent: Channel<TerminalEvent>) => invoke<[number, string]>("terminal_attach", { id, onEvent }),
  terminalDetach: (id: string, key: number) => invoke<void>("terminal_detach", { id, key }),
  terminalWrite: (id: string, data: string) => invoke<void>("terminal_write", { id, data }),
  terminalResize: (id: string, cols: number, rows: number) => invoke<void>("terminal_resize", { id, cols, rows }),
  terminalDismiss: (id: string) => invoke<void>("terminal_dismiss", { id }),
  claudeSignIn: () => invoke<void>("claude_sign_in"),
  setClaudePath: (path: string) => invoke<void>("set_claude_path", { path }),
  quitApp: () => invoke<void>("quit_app"),
};

/** Time until a limit resets, short: "37m", "3h", "2d". */
export function until(ms: number, now: number): string {
  const m = Math.max(0, Math.round((ms - now) / 60_000));
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.round(h / 24)}d`;
}

export function ago(ms: number, now: number): string {
  if (!ms) return "";
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

export const hostName = (s: Setup) => (s.host === "cmux" ? "cmux" : s.host === "builtin" ? "the terminal" : "Orca");

/** cmux needs the hook to see anyone, and socket access to start or message them. */
export const needsSetup = (s: Setup) => s.host === "cmux" && s.cmux && (!s.hooks || !!s.cmuxAccess);

/** A steady colour per area, dark enough to carry white text. */
const AREA_COLORS = ["#2f6f8f", "#6a4c93", "#2f7a55", "#94582f", "#8a3f63", "#56703a", "#4a5a9a", "#8a6a2a"];
export function areaColor(area: string): string {
  let h = 0;
  for (let i = 0; i < area.length; i++) h = (h * 31 + area.charCodeAt(i)) >>> 0;
  return AREA_COLORS[h % AREA_COLORS.length];
}

export function needsYou(a: Agent): boolean {
  return a.state === "needs_you" || a.state === "waiting";
}

export const isBusy = (r: CosRequest) => r.state === "queued" || r.state === "sent" || r.state === "routing";

export function stateLine(a: Agent, now: number): string {
  switch (a.state) {
    case "starting":
      return "Arriving";
    case "working":
      return a.activity ? `Working: ${a.activity}` : "Working";
    case "waiting":
      return "Waiting for your OK";
    case "needs_you":
      return `Needs you, ${ago(a.updatedAt, now)}`;
    case "done":
      return `Finished ${ago(a.updatedAt, now)}`;
    default:
      return "Free";
  }
}
