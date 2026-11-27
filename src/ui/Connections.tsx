import { useEffect, useState } from "react";
import { McpServer, api } from "../api";

/** Remote servers people commonly add. All are https and sign in with OAuth. */
const PRESETS: { name: string; label: string; url: string }[] = [
  { name: "linear", label: "Linear", url: "https://mcp.linear.app/mcp" },
  { name: "notion", label: "Notion", url: "https://mcp.notion.com/mcp" },
  { name: "atlassian", label: "Atlassian", url: "https://mcp.atlassian.com/v2/mcp" },
  { name: "slack", label: "Slack", url: "https://mcp.slack.com/mcp" },
  { name: "figma", label: "Figma", url: "https://mcp.figma.com/mcp" },
  { name: "github", label: "GitHub", url: "https://api.githubcopilot.com/mcp/" },
  { name: "sentry", label: "Sentry", url: "https://mcp.sentry.dev/mcp" },
  { name: "posthog", label: "PostHog", url: "https://mcp.posthog.com/mcp" },
  { name: "vercel", label: "Vercel", url: "https://mcp.vercel.com" },
  { name: "webflow", label: "Webflow", url: "https://mcp.webflow.com/mcp" },
];

const SCOPE: Record<string, string> = {
  you: "All sessions",
  "claude.ai": "claude.ai",
  folder: "One folder",
  plugin: "Plugin",
};

const trouble = (s: McpServer) => s.status === "needs_sign_in" || s.status === "failed" || s.status === "error";

function names(list: string[]) {
  if (list.length <= 2) return list.join(" and ");
  return `${list.slice(0, 2).join(", ")} and ${list.length - 2} more`;
}

function line(s: McpServer): string {
  switch (s.status) {
    case "needs_sign_in":
      return `Needs signing in for ${names(s.waiting)}`;
    case "failed":
      return "Couldn't connect";
    case "error":
      return s.detail ?? "The last call failed";
    case "connected":
      return "Connected";
    default:
      return "Not in use right now";
  }
}

/** The top-bar button: quiet when every server is fine, red when one needs you. */
export function ConnectionsButton({ servers, open, onToggle }: { servers: McpServer[]; open: boolean; onToggle: () => void }) {
  const signIn = servers.filter((s) => s.status === "needs_sign_in" || s.status === "failed").length;
  const errors = servers.filter((s) => s.status === "error").length;
  return (
    <button className={`bar-btn conn-btn${open ? " on" : ""}`} onClick={onToggle} aria-expanded={open} title="MCP connections">
      <i className={`dot ${signIn ? "red" : errors ? "amber" : "green"}`} />
      {signIn ? `${signIn} to sign in` : errors ? `${errors} ${errors === 1 ? "server" : "servers"} failing` : "Connections"}
    </button>
  );
}

function AddServer({ servers, onDone }: { servers: McpServer[]; onDone: () => void }) {
  const [name, setName] = useState("");
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mine = new Set(servers.filter((s) => s.scope === "you").map((s) => s.name));

  const add = async (n: string, u: string) => {
    setBusy(true);
    setError(null);
    try {
      await api.mcpAdd(n, u);
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="conn-add">
      <p className="dim">Added for all your Claude sessions. Your browser opens to sign in.</p>
      <div className="conn-presets">
        {PRESETS.map((p) => (
          <button key={p.name} disabled={busy || mine.has(p.name)} onClick={() => add(p.name, p.url)} title={p.url}>
            {p.label}
            {mine.has(p.name) && <span> added</span>}
          </button>
        ))}
      </div>
      <form
        className="conn-custom"
        onSubmit={(e) => {
          e.preventDefault();
          add(name, url);
        }}
      >
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Server name" />
        <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://…" aria-label="Server address" />
        <button type="submit" disabled={busy || !name.trim() || !url.trim()}>
          Add
        </button>
      </form>
      {error && <p className="usage-error">{error}</p>}
    </div>
  );
}

/** Every MCP server your agents use: which ones need signing in, and a way to fix it from here. */
export function ConnectionsPanel({ servers, onClose }: { servers: McpServer[]; onClose: () => void }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const run = async (id: string, f: () => Promise<void>, done: string) => {
    setBusy(id);
    setError(null);
    setNotice(null);
    try {
      await f();
      setNotice(done);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const active = servers.filter((s) => s.status !== "idle");
  const idle = servers.filter((s) => s.status === "idle");

  return (
    <aside className="usage-panel conn-panel" aria-label="MCP connections">
      <header>
        <h2>Connections</h2>
        <button
          className="conn-reconnect"
          disabled={busy !== null}
          onClick={() => run("reconnect", api.mcpReconnect, "Agents with a failing server reconnect it as soon as they're free.")}
          title="Types /mcp reconnect <server> into each agent whose connection is failing, once it's free"
        >
          Reconnect agents
        </button>
      </header>
      <ul className="conn-list">
        {active.map((s) => (
          <li key={s.name} className={trouble(s) ? "bad" : ""}>
            <i className={`dot ${s.status === "connected" ? "green" : s.status === "error" ? "amber" : "red"}`} />
            <div className="conn-text">
              <b>{s.label}</b>
              <span>
                {SCOPE[s.scope] ? `${SCOPE[s.scope]} · ` : ""}
                {line(s)}
              </span>
            </div>
            {trouble(s) && (
              <button
                className="switch"
                disabled={busy !== null}
                onClick={() =>
                  run(s.name, () => api.mcpSignIn(s.name), `Signing in to ${s.label} in a terminal. Agents waiting on it reconnect once it's done.`)
                }
              >
                {busy === s.name ? "Opening…" : "Sign in"}
              </button>
            )}
          </li>
        ))}
        {active.length === 0 && <li className="dim">No agent is using an MCP server right now.</li>}
      </ul>
      {idle.length > 0 && <p className="conn-idle">Not in use right now: {idle.map((s) => s.label).join(", ")}</p>}
      {notice && <p className="conn-notice">{notice}</p>}
      {error && <p className="usage-error">{error}</p>}
      {adding ? (
        <AddServer
          servers={servers}
          onDone={() => {
            setAdding(false);
            setNotice("Adding it in a terminal. Your browser opens to sign in.");
          }}
        />
      ) : (
        <button className="conn-add-btn" onClick={() => setAdding(true)}>
          Add a server
        </button>
      )}
    </aside>
  );
}
