import { useEffect, useState } from "react";
import { ClaudeAccount, Limits, Usage, UsageWindow, ago, api, until } from "../api";

function useNow(ms: number) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

const level = (used: number) => (used >= 90 ? "high" : used >= 75 ? "mid" : "low");

function Meter({ w }: { w: UsageWindow }) {
  return (
    <span className={`meter ${level(w.used)}`} aria-hidden="true">
      <i style={{ width: `${Math.max(2, w.used)}%` }} />
    </span>
  );
}

/** The bar in the top right: how much of your Claude plan you've used, like Orca's. */
export function UsageButton({ usage, open, onToggle }: { usage: Usage; open: boolean; onToggle: () => void }) {
  const now = useNow(30_000);
  const l = usage.limits;
  const active = usage.accounts.find((a) => a.active);
  if (!l) return null;
  return (
    <button
      className={`bar-btn usage-btn${open ? " on" : ""}`}
      onClick={onToggle}
      aria-expanded={open}
      title="Claude usage and accounts"
    >
      {usage.accounts.length > 1 && active && <span className="usage-acct">{active.name}</span>}
      {l.session ? (
        <>
          <Meter w={l.session} />
          <span>
            {Math.round(l.session.used)}% <span className="dim">· {until(l.session.resetsAt, now)}</span>
          </span>
        </>
      ) : (
        <span className="dim">{l.error ? "Usage unavailable" : "Usage"}</span>
      )}
      {l.week && <span className="dim">{Math.round(l.week.used)}% week</span>}
      {l.fable && <span className="dim">{Math.round(l.fable.used)}% Fable</span>}
    </button>
  );
}

function Rows({ limits, now }: { limits: Limits; now: number }) {
  const rows: [string, UsageWindow | null][] = [
    ["Session (5 hours)", limits.session],
    ["This week", limits.week],
    ["Fable this week", limits.fable],
  ];
  return (
    <ul className="usage-rows">
      {rows.map(([label, w]) =>
        w ? (
          <li key={label}>
            <span className="row-label">{label}</span>
            <Meter w={w} />
            <span className="row-val">
              {Math.round(w.used)}% <span className="dim">· resets in {until(w.resetsAt, now)}</span>
            </span>
          </li>
        ) : null,
      )}
      {limits.error && <li className="usage-error">{limits.error}</li>}
    </ul>
  );
}

/** Usage for each Claude subscription saved in Orca, and a one-click switch between them. */
export function UsagePanel({
  usage,
  agents,
  midTask,
  onClose,
}: {
  usage: Usage;
  /** How many agents a switch restarts. */
  agents: number;
  /** Who's mid-task right now; switching waits for them. */
  midTask: string[];
  onClose: () => void;
}) {
  const now = useNow(30_000);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [switched, setSwitched] = useState(false);
  const waitingOn =
    midTask.length === 0
      ? ""
      : midTask.length <= 2
        ? midTask.join(" and ")
        : `${midTask.slice(0, 2).join(", ")} and ${midTask.length - 2} more`;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const run = async (id: string, f: () => Promise<void>) => {
    setBusy(id);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  // With no saved accounts, Orca still reports the one Claude Code is signed in with.
  const accounts: ClaudeAccount[] = usage.accounts.length
    ? usage.accounts
    : [{ id: "", name: "Your Claude account", email: "", active: true, limits: usage.limits }];
  const updated = usage.limits?.updatedAt ?? 0;

  return (
    <aside className="usage-panel" aria-label="Claude usage">
      <header>
        <h2>Claude usage</h2>
        <button
          className="refresh"
          onClick={() => run("refresh", api.refreshUsage)}
          disabled={busy !== null}
          title="Check usage now"
          aria-label="Check usage now"
        >
          <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true" className={busy === "refresh" ? "spin" : ""}>
            <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5v2.8h-2.8" />
          </svg>
        </button>
      </header>
      {accounts.map((a) => (
        <section key={a.id || "me"} className={a.active ? "active" : ""}>
          <div className="acct">
            <div className="acct-name">
              <b>{a.name}</b>
              {a.email && a.email !== a.name && <span>{a.email}</span>}
            </div>
            {a.active ? (
              <span className="in-use">In use</span>
            ) : (
              <button
                className="switch"
                onClick={() => setConfirm(confirm === a.id ? null : a.id)}
                disabled={busy !== null || midTask.length > 0}
                title={midTask.length ? `Switch once ${waitingOn} ${midTask.length === 1 ? "is" : "are"} done` : undefined}
              >
                {busy === a.id ? "Switching…" : "Switch"}
              </button>
            )}
          </div>
          {confirm === a.id && (
            <div className="switch-confirm">
              <p>
                {agents === 0
                  ? `Switch to ${a.name}?`
                  : `Switch to ${a.name} and restart ${agents === 1 ? "your agent" : `all ${agents} agents`}? Each picks up its conversation where it left off.`}
              </p>
              <div>
                <button className="conn-reconnect" onClick={() => setConfirm(null)}>
                  Cancel
                </button>
                <button
                  className="switch"
                  disabled={busy !== null}
                  onClick={() =>
                    run(a.id, async () => {
                      await api.switchAccount(a.id);
                      setConfirm(null);
                      setSwitched(true);
                    })
                  }
                >
                  {agents === 0 ? "Switch" : "Switch and restart"}
                </button>
              </div>
            </div>
          )}
          {a.limits ? (
            <Rows limits={a.limits} now={now} />
          ) : (
            <p className="dim">Usage shows here after Orca next checks this account.</p>
          )}
        </section>
      ))}
      {midTask.length > 0 && usage.accounts.length > 1 && (
        <p className="usage-wait">
          Switching restarts every agent, so it waits until {waitingOn} {midTask.length === 1 ? "is" : "are"} done.
        </p>
      )}
      {switched && <p className="conn-notice">Switched. Your agents are restarting on the new account.</p>}
      <p className="usage-foot">
        {usage.accounts.length <= 1
          ? "Add another Claude account in Orca's settings to switch between subscriptions here. "
          : midTask.length === 0
            ? "Switching restarts your agents so they use the new account straight away, each in the conversation it was in. "
            : ""}
        {updated > 0 && `Checked ${ago(updated, now)}.`}
      </p>
      {error && <p className="usage-error">{error}</p>}
    </aside>
  );
}
