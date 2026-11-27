// Dev-only: renders the whole app against made-up data so the UI can be reviewed outside Tauri.
// Open http://localhost:1420/preview.html while `npm run dev` is running.
import React from "react";
import ReactDOM from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import "@fontsource-variable/archivo/wdth.css";
import "../styles.css";
import App from "../App";
import type { Agent, CosRequest, Room, Snapshot } from "../api";

const rooms: Room[] = ["api", "web-app", "data-tools", "docs", "prototypes", "infra", "mobile", "billing"].map((name, i) => ({
  id: `room-${i}`,
  name,
  original: name,
  path: `/tmp/${name}`,
  kind: "repo",
  color: null,
  areas: name === "api" ? [{ id: "w-main", name: "Building" }, { id: "w-gtm", name: "GTM/Ops" }, { id: "w-prod", name: "Product" }] : [],
}));

const helper = (id: string, task: string, kind: string, running = true) => ({ id, task, kind, running, startedAt: Date.now() - 9 * 60_000, updatedAt: Date.now() - (running ? 5_000 : 4 * 60_000) });

const agent = (id: string, name: string, roomId: string, state: Agent["state"], headline = "", unread = false, extra: Partial<Agent> = {}): Agent => ({
  id,
  name,
  roomId,
  role: "worker",
  managed: true,
  title: headline || "Refactor the sync job",
  task: "Refactor the sync job",
  state,
  brief: headline ? { status: state === "needs_you" ? "needs_input" : "done", headline, points: [], questions: [], links: [] } : null,
  activity: state === "working" ? "Edit: src/sync.ts" : null,
  prompt: null,
  updatedAt: Date.now() - 4 * 60_000,
  unread,
  area: roomId === "room-0" ? (name === "Wren" ? "GTM/Ops" : "Building") : null,
  helpers: name === "Otto" ? [helper("h1", "Search the billing code", "Explore"), helper("h2", "Check the failing tests", "general-purpose")] : [],
  ask: null,
  desk: null,
  work: { Otto: 140, Wren: 40, Iris: 75, Mabel: 12, Bea: 30, Cleo: 95 }[name] ?? 0,
  ...extra,
});

const agents: Agent[] = [
  agent("a1", "Otto", "room-0", "working"),
  agent("a2", "Wren", "room-0", "needs_you", "The retry job can run hourly or daily. Which do you want?", true, {
    brief: {
      status: "needs_input",
      headline: "The retry job can run hourly or daily. Which do you want?",
      points: ["Hourly costs about 2,400 extra API calls a day", "Daily means failed syncs wait up to 24 hours"],
      questions: [{ q: "How often should the retry job run?", options: ["Hourly", "Daily", "Every 6 hours"] }],
      links: [{ label: "PR #212 in api", url: "https://github.com/acme/api/pull/212" }],
    },
  }),
  agent("a3", "Iris", "room-1", "done", "Opened PR #318, which fixes the signup button on small phones.", true),
  agent("a4", "Hugo", "room-1", "idle"),
  agent("a5", "Mabel", "room-2", "working"),
  agent("a6", "Ada", "room-3", "waiting", "Waiting for your OK to use Bash"),
  agent("a7", "Bea", "room-4", "working"),
  agent("a8", "Cleo", "room-4", "needs_you", "2 questions for you", true, {
    helpers: [helper("h4", "Benchmark trade-business pricing", "general-purpose")],
    ask: [
      {
        question: "How should the base price work for a 1–5 person business?",
        header: "Pricing",
        multiSelect: false,
        options: [
          { label: "Flat price per business", description: "One price whatever the headcount." },
          { label: "Base plus per extra user", description: "The owner is included; each extra cardholder adds a little." },
          { label: "Not decided yet", description: "Model a couple of options and recommend one." },
        ],
      },
      {
        question: "What can count toward the revenue target?",
        header: "Revenue",
        multiSelect: true,
        options: [
          { label: "Subscriptions", description: "" },
          { label: "Interchange", description: "Card spend revenue." },
        ],
      },
    ],
  }),
  agent("a9", "Dex", "room-5", "idle"),
];
// `?crowd` fills one room, to check how rooms size themselves.
if (new URLSearchParams(location.search).has("crowd"))
  ["Nell", "Pip", "Rhys", "Sol", "Tess", "Uma", "Vic", "Wes"].forEach((n, i) =>
    agents.push(agent(`c${i}`, n, "room-0", i % 3 ? "working" : "idle", "", false, { work: i * 22 })),
  );

const requests: CosRequest[] = [
  {
    id: "r3",
    text: "Tidy up the onboarding emails copy",
    createdAt: Date.now(),
    sentAt: Date.now(),
    state: "asking",
    question: "Should this go to web-app or docs?",
    options: [
      { label: "web-app", room: "web-app" },
      { label: "docs", room: "docs" },
    ],
    result: null,
    agentId: null,
    files: ["/Users/you/Library/Application Support/agent-office/attachments/3f2a9c1d0b/2027 planning.md"],
  },
  {
    id: "r4",
    text: "Draft the Q1 roadmap one-pager from last week's planning notes",
    createdAt: Date.now(),
    sentAt: Date.now(),
    state: "sent",
    question: null,
    options: [],
    result: null,
    agentId: null,
    files: [],
  },
  {
    id: "r2",
    text: "Fix the flaky billing webhook test",
    createdAt: Date.now() - 600_000,
    sentAt: Date.now() - 600_000,
    state: "routed",
    question: null,
    options: [],
    result: "Sent to Otto in api",
    agentId: "a1",
    files: [],
  },
];

const params = new URLSearchParams(location.search);
const nowLimits = {
  session: { used: 47, resetsAt: Date.now() + 37 * 60_000 },
  week: { used: 36, resetsAt: Date.now() + 2 * 86_400_000 },
  fable: { used: 0, resetsAt: Date.now() + 2 * 86_400_000 },
  updatedAt: Date.now() - 90_000,
  error: null,
};

const snap: Snapshot = {
  connected: params.get("offline") === null,
  error: null,
  rooms,
  hiddenRooms: [{ id: "room-x", name: "e2e", original: "e2e", path: "/tmp/e2e", kind: "repo", color: null, areas: [] }],
  agents: params.get("empty") !== null ? [] : agents,
  requests: params.get("empty") !== null ? [] : requests,
  cosName: "Raggy",
  askCosName: params.get("first") !== null,
  notifications: true,
  briefing: {
    summary: "Three things need you; everyone else is moving.",
    at: Date.now() - 120_000,
    items: [
      { kind: "decide", text: "Wren needs to know whether the retry job runs hourly or daily before finishing.", agents: ["Wren"], action: { type: "open", agent: "Wren", text: "", server: "" } },
      { kind: "unblock", text: "atlassian needs signing in for Otto and Hugo; one sign-in unblocks both.", agents: ["Otto", "Hugo"], action: { type: "mcp", agent: "", text: "", server: "atlassian" } },
      {
        kind: "connect",
        text: "Iris's PR #318 changes the signup flow Bea is testing.",
        agents: ["Iris", "Bea"],
        action: { type: "tell", agent: "Bea", text: "Heads-up: Iris just opened PR #318, which changes the signup button on small phones. Check your tests against it.", server: "" },
      },
    ],
  },
  briefingBusy: false,
  saved: [
    { id: "0b86e6b1-4542-4adc-a368-bcbfed609610", name: "Ring article edit", roomId: "room-0", savedAt: Date.now() - 3 * 3_600_000, open: false },
  ],
  mcp: [
    { name: "atlassian", label: "atlassian", scope: "folder", status: "needs_sign_in", detail: null, waiting: ["Otto", "Hugo"], room: "room-0" },
    { name: "team-tools", label: "team-tools", scope: "you", status: "error", detail: "Cloudflare blocked the last request", waiting: [], room: null },
    { name: "linear-server", label: "linear-server", scope: "you", status: "connected", detail: null, waiting: [], room: null },
    { name: "claude.ai Notion", label: "Notion", scope: "claude.ai", status: "connected", detail: null, waiting: [], room: null },
    { name: "claude.ai Figma", label: "Figma", scope: "claude.ai", status: "idle", detail: null, waiting: [], room: null },
    { name: "claude.ai Vercel", label: "Vercel", scope: "claude.ai", status: "idle", detail: null, waiting: [], room: null },
  ],
  usage: {
    limits: nowLimits,
    accounts: [
      { id: "acc-1", name: "Acme", email: "sam@acme.example", active: true, limits: nowLimits },
      {
        id: "acc-2",
        name: "Personal",
        email: "sam@acme.example",
        active: false,
        limits: {
          session: { used: 8, resetsAt: Date.now() + 3 * 3_600_000 },
          week: { used: 81, resetsAt: Date.now() + 4 * 86_400_000 },
          fable: null,
          updatedAt: Date.now() - 300_000,
          error: null,
        },
      },
    ],
  },
  setup: {
    host: params.get("builtin") !== null ? "builtin" : "orca",
    preference: "auto",
    orca: true,
    cmux: true,
    hooks: false,
    cmuxAccess: "cmux closed the connection. Its socket only lets in cmux's own terminals until you allow other apps.",
    cmuxPassword: false,
    claude: true,
    claudePath: "",
  },
};

// A made-up Claude screen for the terminal panel (?builtin, then open someone's terminal).
const screen = [
  "\x1b[1m ▐▛███▜▌\x1b[0m   Claude Code",
  "▝▜█████▛▘  Opus · Claude Max",
  "  ▘▘ ▝▝    ~/code/api",
  "",
  "\x1b[38;5;245m❯\x1b[0m Make the retry job configurable",
  "",
  "\x1b[38;5;114m⏺\x1b[0m I made the schedule a setting. Which default do you want, hourly or daily?",
  "",
  "\x1b[38;5;240m" + "─".repeat(100) + "\x1b[0m",
  "❯ ",
  "\x1b[38;5;240m" + "─".repeat(100) + "\x1b[0m",
  "  \x1b[38;5;245m⏵⏵ auto mode on\x1b[0m",
].join("\r\n");
const toB64 = (t: string) => btoa(String.fromCharCode(...new TextEncoder().encode(t)));

mockIPC(
  (cmd, args) => {
  if (cmd === "snapshot") return snap;
  if (cmd === "open_terminal" && snap.setup.host === "builtin") {
    emit("open-terminal", (args as { agentId: string }).agentId);
    return null;
  }
  if (cmd === "terminal_attach") return [1, toB64(screen)];
  if (cmd === "links")
    return {
      top: [
        { kind: "PR", label: "The retry schedule PR", url: "https://github.com/acme/api/pull/212" },
        { kind: "Notion", label: "Retry job PRD", url: "https://www.notion.so/acme/Retry-job-PRD-0123456789abcdef0123456789abcdef" },
      ],
      more: [{ kind: "Linear", label: "ENG-401 Configurable retries", url: "https://linear.app/acme/issue/ENG-401/configurable-retries" }],
    };
  if (cmd === "skills")
    return [
      { name: "draft-prd", description: "Draft a PRD from notes, tickets and calls", source: "project" },
      { name: "release", description: "Cut a release and post the notes to #product-updates", source: "project" },
      { name: "review-pr", description: "Review the open pull request for bugs and style", source: "you" },
      { name: "frontend-design:frontend-design", description: "Design distinctive, production-grade interfaces", source: "plugin" },
      { name: "vercel:deploy", description: "Deploy the current project to Vercel", source: "plugin" },
      { name: "clear", description: "Start fresh: clear the conversation", source: "built-in" },
      { name: "compact", description: "Shrink the conversation to free up room", source: "built-in" },
      { name: "mcp", description: "Manage MCP server connections", source: "built-in" },
    ];
  if (cmd === "helpers")
    return [
      helper("h1", "Search the billing code", "Explore"),
      helper("h2", "Check the failing tests", "general-purpose"),
      helper("h3", "Draft the migration for the jobs table", "general-purpose", false),
    ];
  if (cmd === "helper_work")
    return [
      { kind: "prompt", text: "Find every place the billing code retries a failed charge, and list them with file and line.", ts: "" },
      { kind: "tool", name: "Search", detail: "retry in src/billing", ts: "" },
      { kind: "tool", name: "Read", detail: "src/billing/charge.ts", ts: "" },
      { kind: "said", text: "Three places retry: `charge.ts:88`, `invoice.ts:140` and the nightly sweep in `jobs/retry.ts:12`.", ts: "" },
    ];
  if (cmd === "stop_agent") return "unqueued";
  if (cmd === "work" && new URLSearchParams(location.search).has("queued"))
    return [
      { kind: "prompt", text: "Make the retry job configurable", ts: "" },
      { kind: "tool", name: "Edit", detail: "src/jobs/retry.ts", ts: "" },
      { kind: "prompt", text: "Actually, keep the default at hourly", ts: "", queued: true },
    ];
  if (cmd === "work")
    return [
      { kind: "prompt", text: "Make the retry job configurable", ts: "" },
      { kind: "tool", name: "Read", detail: "src/jobs/retry.ts", ts: "" },
      { kind: "tool", name: "Edit", detail: "src/jobs/retry.ts", ts: "" },
      { kind: "said", text: "I made the schedule a setting. It needs a default, so I've asked which one you want before I open the PR. Everything else is ready: tests pass, and the migration only touches the jobs table.", ts: "" },
    ];
  return null;
  },
  { shouldMockEvents: true },
);

ReactDOM.createRoot(document.getElementById("host")!).render(React.createElement(App));
