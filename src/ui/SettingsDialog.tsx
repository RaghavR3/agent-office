import { useState } from "react";
import { Setup, api } from "../api";
import { isSummer } from "../office/art";
import { Dialog } from "./Dialog";

const HOSTS: { id: Setup["preference"]; label: string }[] = [
  { id: "auto", label: "Automatic" },
  { id: "orca", label: "Orca" },
  { id: "cmux", label: "cmux" },
  { id: "builtin", label: "Built-in" },
];

const names = (list: string[]) => (list.length === 1 ? list[0] : `${list.slice(0, -1).join(", ")} and ${list[list.length - 1]}`);

function hostLine(s: Setup): string {
  if (s.host === "builtin") {
    if (s.preference !== "auto") return "Agents run in the office's own terminals, inside Agent Office.";
    return s.orca || s.cmux
      ? "Agents run in the office's own terminals while any are at work there."
      : "Agents run in the office's own terminals, since neither Orca nor cmux is running.";
  }
  const name = s.host === "orca" ? "Orca" : "cmux";
  if (!s.orca && !s.cmux) return "Neither Orca nor cmux is running.";
  if (s.preference === "auto") {
    const other = s.host === "orca" ? (s.cmux ? " cmux is running too." : "") : s.orca ? " Orca is running too." : "";
    return `Showing ${name}, since it's running.${other}`;
  }
  return (s.host === "orca" ? s.orca : s.cmux) ? `Showing ${name}.` : `Showing ${name}, which isn't running right now.`;
}

export function SettingsDialog({ setup, cosName, dreamHouse, onClose }: { setup: Setup; cosName: string; dreamHouse: boolean; onClose: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [password, setPassword] = useState("");
  const [saved, setSaved] = useState(false);
  const [claudePath, setClaudePath] = useState(setup.claudePath);

  const run = async (f: () => Promise<unknown>) => {
    setError(null);
    setBusy(true);
    try {
      await f();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const showCmux = setup.host === "cmux" || setup.cmux;

  return (
    <Dialog
      label="Settings"
      onClose={onClose}
      head={
        <span className="portrait t-empty" style={{ width: 52, height: 52 }} aria-hidden="true">
          <svg viewBox="0 0 16 16" width="22" height="22">
            <path d="M3 4.5h10M3 8h10M3 11.5h10" />
            <circle cx="6" cy="4.5" r="1.4" />
            <circle cx="10.5" cy="8" r="1.4" />
            <circle cx="5" cy="11.5" r="1.4" />
          </svg>
        </span>
      }
      title="Settings"
      sub={<span>How the office connects to your terminals.</span>}
    >
      <section className="setting">
        <h3>Terminal app</h3>
        <div className="seg" role="group" aria-label="Terminal app">
          {HOSTS.map((h) => (
            <button
              key={h.id}
              aria-pressed={setup.preference === h.id}
              onClick={() => run(() => api.setHost(h.id))}
              disabled={busy}
            >
              {h.label}
            </button>
          ))}
        </div>
        <p className="quiet">{hostLine(setup)}</p>
        {setup.elsewhere.length > 0 && (
          <p className="warn">
            {names(setup.elsewhere)} {setup.elsewhere.length === 1 ? "is" : "are"} still running in the office's own terminals, out of
            sight here. Switch to Built-in to see them; quitting stops them.
          </p>
        )}
      </section>

      {(setup.host === "builtin" || setup.preference === "builtin") && (
        <section className="setting">
          <h3>Claude Code</h3>
          {setup.claude ? (
            <p className="quiet">Found. Agents run in it on your own Claude plan.</p>
          ) : (
            <p className="warn">Couldn't find Claude Code. Install it from claude.com/claude-code, or give its path here.</p>
          )}
          <form
            className="password-row"
            onSubmit={(e) => {
              e.preventDefault();
              run(() => api.setClaudePath(claudePath));
            }}
          >
            <input
              value={claudePath}
              placeholder="Found on your login PATH. Paste a path to use another."
              onChange={(e) => setClaudePath(e.target.value)}
              aria-label="Where Claude Code is"
              spellCheck={false}
            />
            <button className="btn ghost" type="submit" disabled={busy || claudePath === setup.claudePath}>
              Save
            </button>
          </form>
          <div className="handoff-row">
            <span className="quiet">Signed out, or switching plans? Claude Code's own sign-in opens in a terminal.</span>
            <button className="btn ghost" onClick={() => run(api.claudeSignIn)} disabled={busy || !setup.claude}>
              Sign in
            </button>
          </div>
        </section>
      )}

      {isSummer(cosName) && (
        <section className="setting">
          <h3>Dream house</h3>
          <p>With {cosName.trim()} as chief of staff, the office is a pink dream house full of golden retrievers.</p>
          <div className="handoff-row">
            <span className="quiet">{dreamHouse ? "On." : `Off: the normal office, with ${cosName.trim()} at reception.`}</span>
            <button className="btn ghost" onClick={() => run(() => api.setDreamHouse(!dreamHouse))} disabled={busy}>
              {dreamHouse ? "Turn off" : "Turn on"}
            </button>
          </div>
        </section>
      )}

      <section className="setting">
        <h3>Status hook</h3>
        <p>
          Lets the office follow Claude Code directly instead of relying on {setup.host === "orca" ? "Orca's" : "the terminal app's"} status. It
          adds an entry to <code>~/.claude/settings.json</code> that saves each session's latest step to{" "}
          <code>~/.agent-office</code> on this Mac. Your other hooks stay as they are, and the file is backed up first.
        </p>
        {setup.host === "cmux" && !setup.hooks && <p className="warn">cmux needs this to show your agents.</p>}
        {setup.host === "builtin" && <p className="quiet">The office's own terminals don't need it: they get the hook for their sessions only.</p>}
        <div className="handoff-row">
          <span className="quiet">
            {setup.hooks
              ? "Installed. Sessions started from now on report in; restart running ones to include them."
              : "Not installed."}
          </span>
          {setup.hooks ? (
            <button className="btn ghost" onClick={() => run(api.uninstallHooks)} disabled={busy}>
              Remove
            </button>
          ) : (
            <button className="btn primary" onClick={() => run(api.installHooks)} disabled={busy}>
              Install
            </button>
          )}
        </div>
      </section>

      {showCmux && (
        <section className="setting">
          <h3>cmux access</h3>
          {setup.cmuxAccess ? (
            <p>
              cmux only takes commands from its own terminals. To let the office start agents and message them, open cmux
              Settings and set Socket Control Mode to the automation option, or to password mode and paste the password
              here.
            </p>
          ) : (
            <p className="quiet">{setup.cmux ? "Connected. The office can start agents in cmux and message them." : "cmux isn't running."}</p>
          )}
          <form
            className="password-row"
            onSubmit={(e) => {
              e.preventDefault();
              run(async () => {
                await api.setCmuxPassword(password);
                setPassword("");
                setSaved(true);
              });
            }}
          >
            <input
              type="password"
              value={password}
              placeholder={setup.cmuxPassword ? "Password saved. Type a new one to replace it" : "cmux socket password"}
              onChange={(e) => {
                setPassword(e.target.value);
                setSaved(false);
              }}
              aria-label="cmux socket password"
            />
            <button className="btn ghost" type="submit" disabled={busy || (!password && !setup.cmuxPassword)}>
              {password || !setup.cmuxPassword ? "Save" : "Clear"}
            </button>
          </form>
          {saved && <p className="quiet">Saved.</p>}
        </section>
      )}

      {error && <p className="error">{error}</p>}
    </Dialog>
  );
}
