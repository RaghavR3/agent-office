import { useEffect, useRef, useState } from "react";
import { Agent, BriefItem, CosRequest, Snapshot, ago, api } from "../api";
import { Dialog, Portrait } from "./Dialog";
import { Attachment, Composer } from "./Composer";

function Name({ name }: { name: string }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(name);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setValue(name), [name]);
  if (!editing)
    return (
      <button className="rename" onClick={() => setEditing(true)} title="Rename">
        {name}
      </button>
    );
  const save = async () => {
    try {
      await api.renameCos(value);
      setEditing(false);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };
  return (
    <form
      className="rename-form"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <input
        autoFocus
        value={value}
        maxLength={20}
        aria-label="Chief of staff's name"
        onChange={(e) => setValue(e.target.value)}
        onBlur={() => setEditing(false)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            setEditing(false);
          }
        }}
      />
      {error && <span className="error">{error}</span>}
    </form>
  );
}

const fileNames = (files: string[]) => files.map((f) => f.split("/").pop()).join(", ");

/** The hand-off, folded to a few lines when it's long. */
function Quote({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const long = text.length > 280 || text.split("\n").length > 4;
  return (
    <blockquote className={long && !open ? "folded" : ""}>
      {text}
      {long && (
        <button type="button" className="link-more" onClick={() => setOpen(!open)}>
          {open ? "Show less" : "Show all"}
        </button>
      )}
    </blockquote>
  );
}

/** A question from the chief of staff: pick an option, answer in your own words, or call it off. */
function Ask({ r, name }: { r: CosRequest; name: string }) {
  const [text, setText] = useState("");
  const [files, setFiles] = useState<Attachment[]>([]);
  const [error, setError] = useState<string | null>(null);
  const busy = useRef(false);
  const run = async (f: () => Promise<unknown>) => {
    if (busy.current) return;
    busy.current = true;
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(String(e));
    } finally {
      busy.current = false;
    }
  };
  const answer = () => {
    if (text.trim())
      run(() =>
        api.answerRequest(
          r.id,
          text,
          files.map((f) => f.path),
        ),
      );
  };
  return (
    <div className="ask">
      <Quote text={r.text} />
      {r.files.length > 0 && <p className="ask-files">Attached: {fileNames(r.files)}</p>}
      <p className="headline">{r.question || "Where should this go?"}</p>
      {r.options.length > 0 && (
        <div className="options">
          {r.options.map((o) => (
            <button key={o.label} type="button" className="opt" onClick={() => run(() => api.chooseRoute(r.id, o.label, o.room))}>
              {o.label}
            </button>
          ))}
        </div>
      )}
      <form
        className="ask-answer"
        onSubmit={(e) => {
          e.preventDefault();
          answer();
        }}
      >
        <Composer
          value={text}
          onChange={setText}
          files={files}
          onFiles={setFiles}
          onSubmit={answer}
          placeholder={r.options.length ? "Or answer in your own words" : `Answer ${name}`}
          rows={1}
          primary={false}
        />
        <div className="handoff-row">
          <button type="button" className="btn ghost" onClick={() => run(() => api.cancelRequest(r.id))}>
            Cancel this task
          </button>
          <button type="submit" className="btn primary" disabled={!text.trim()}>
            Answer
          </button>
        </div>
      </form>
      {error && <p className="error">{error}</p>}
    </div>
  );
}

/** Small line icons for each kind of rundown item. */
const ICONS: Record<string, string> = {
  decide: "M8 14.2A6.2 6.2 0 1 0 8 1.8a6.2 6.2 0 0 0 0 12.4zM6.3 6.3a1.8 1.8 0 1 1 2.4 1.7c-.5.2-.7.5-.7 1v.4M8 11.2v.1",
  unblock: "M8 2.6l5.6 9.8H2.4L8 2.6zM8 6.4v2.6M8 10.9v.1",
  review: "M1.8 8s2.3-4.2 6.2-4.2S14.2 8 14.2 8s-2.3 4.2-6.2 4.2S1.8 8 1.8 8zM8 9.6a1.6 1.6 0 1 0 0-3.2 1.6 1.6 0 0 0 0 3.2z",
  connect: "M6.6 9.4l2.8-2.8M5.4 7.6l-1.2 1.2a2.2 2.2 0 0 0 3.1 3.1l1.2-1.2M10.6 8.4l1.2-1.2a2.2 2.2 0 0 0-3.1-3.1L7.5 5.3",
  next: "M3 8h9.4M9 4.6L12.4 8 9 11.4",
};

const KINDS: Record<string, { label: string; tone: string }> = {
  unblock: { label: "Unblock", tone: "red" },
  decide: { label: "Decide", tone: "orange" },
  connect: { label: "Connect", tone: "orange" },
  review: { label: "Review", tone: "green" },
  next: { label: "Next", tone: "green" },
};

/** One thing from the rundown, with the one click that deals with it. Nothing reaches an agent
 * unless you press its button. */
function RundownItem({ item, index, agents, onGo }: { item: BriefItem; index: number; agents: Agent[]; onGo: (id: string) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const kind = KINDS[item.kind] ?? { label: "Note", tone: "orange" };
  const a = item.action;
  const target = a?.agent ? agents.find((x) => x.name.toLowerCase() === a.agent.toLowerCase()) : undefined;
  const run = async (f: () => Promise<unknown>, done = true) => {
    setBusy(true);
    setError(null);
    try {
      await f();
      if (done) await api.dismissBriefItem(index);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  let button = null;
  if (a?.type === "open" && target) button = <button className="btn3d" onClick={() => onGo(target.id)}>Go to {target.name}</button>;
  if (a?.type === "mcp" && a.server)
    button = (
      <button className="btn3d" disabled={busy} onClick={() => run(() => api.mcpSignIn(a.server), false)}>
        Sign in to {a.server}
      </button>
    );
  if (a?.type === "reconnect")
    button = (
      <button className="btn3d" disabled={busy} onClick={() => run(api.mcpReconnect)}>
        Reconnect agents
      </button>
    );
  if (a?.type === "tell" && target && a.text)
    button = (
      <button className="btn3d" disabled={busy} onClick={() => run(() => api.messageAgent(target.id, a.text))}>
        {busy ? "Sending…" : `Send to ${target.name}`}
      </button>
    );
  if (a?.type === "handoff" && a.text)
    button = (
      <button className="btn3d" disabled={busy} onClick={() => run(() => api.delegate(a.text, []))}>
        Hand it over
      </button>
    );

  return (
    <li className={`ri t-${kind.tone}`}>
      <span className="ri-icon" aria-hidden="true">
        <svg viewBox="0 0 16 16" width="15" height="15">
          <path d={ICONS[item.kind] ?? ICONS.review} />
        </svg>
      </span>
      <div className="ri-body">
        <span className="ri-kind">{kind.label}</span>
        <p>{item.text}</p>
        {(a?.type === "tell" || a?.type === "handoff") && a.text && <blockquote className="ri-msg">{a.text}</blockquote>}
        {error && <p className="error">{error}</p>}
      </div>
      <div className="ri-actions">{button}</div>
      <button className="ri-dismiss" onClick={() => api.dismissBriefItem(index)} aria-label="Dismiss" title="Dismiss">
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
          <path d="M2 2l8 8M10 2l-8 8" />
        </svg>
      </button>
    </li>
  );
}

/** The chief of staff's rundown: what needs you across every agent, most urgent first. */
function Rundown({ snap, onGo }: { snap: Snapshot; onGo: (id: string) => void }) {
  const b = snap.briefing;
  const name = snap.cosName;
  return (
    <section className="rundown" aria-label={`${name}'s rundown`}>
      <header>
        <h3>Where things stand</h3>
        <span className="quiet">{snap.briefingBusy ? `${name} is catching up…` : b?.at ? `Updated ${ago(b.at, Date.now())}` : ""}</span>
        <button className="link-btn" disabled={snap.briefingBusy} onClick={() => api.briefNow()}>
          Refresh
        </button>
      </header>
      {b ? (
        <>
          {b.summary && <p className="rundown-summary">{b.summary}</p>}
          {b.items.length > 0 && (
            <ul>
              {b.items.map((it, i) => (
                <RundownItem key={`${i}:${it.text}`} item={it} index={i} agents={snap.agents} onGo={onGo} />
              ))}
            </ul>
          )}
        </>
      ) : (
        <p className="rundown-summary">{name} gives you a rundown here as soon as something needs you.</p>
      )}
    </section>
  );
}

export function CosDialog({
  name,
  requests,
  snap,
  onGo,
  onClose,
}: {
  name: string;
  requests: CosRequest[];
  snap: Snapshot;
  onGo: (id: string) => void;
  onClose: () => void;
}) {
  const [text, setText] = useState("");
  const [files, setFiles] = useState<Attachment[]>([]);
  const [error, setError] = useState<string | null>(null);
  const asks = requests.filter((r) => r.state === "asking");
  const inFlight = requests.filter((r) => r.state === "queued" || r.state === "sent" || r.state === "routing");
  const sending = useRef(false);

  const send = async () => {
    if ((!text.trim() && !files.length) || sending.current) return;
    sending.current = true;
    setError(null);
    try {
      // The files travel with the request to whoever takes it; the chief of staff only sees their names.
      await api.delegate(
        text,
        files.map((f) => f.path),
      );
      setText("");
      setFiles([]);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      sending.current = false;
    }
  };

  return (
    <Dialog
      label={`${name}, chief of staff`}
      onClose={onClose}
      head={<Portrait who={{ name, role: "cos" }} size={52} tone="green" />}
      title={<Name name={name} />}
      sub={<span>Chief of staff. {name} keeps track of what needs you and finds the right agent for anything you hand over.</span>}
    >
      {asks.map((r) => (
        <Ask key={r.id} r={r} name={name} />
      ))}
      <Rundown snap={snap} onGo={onGo} />
      {inFlight.length > 0 && (
        <ul className="inflight" aria-label={`With ${name}`}>
          {inFlight.map((r) => (
            <li key={r.id}>
              <i className="pulse" />
              <span className="inflight-text">
                {r.text}
                {r.files.length > 0 && <em> + {fileNames(r.files)}</em>}
              </span>
              {r.state === "routing" ? (
                <span className="quiet">Sending to an agent</span>
              ) : (
                <button type="button" className="btn ghost" onClick={() => api.cancelRequest(r.id).catch((e) => setError(String(e)))}>
                  Cancel
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      <form
        className="handoff"
        onSubmit={(e) => {
          e.preventDefault();
          send();
        }}
      >
        <Composer value={text} onChange={setText} files={files} onFiles={setFiles} onSubmit={send} placeholder="What needs doing?" rows={3} autoFocus skills={{}} />
        <div className="handoff-row">
          <span className="quiet">Enter hands it over. Shift+Enter adds a line.</span>
          <button type="submit" className="btn primary" disabled={!text.trim()}>
            Hand over
          </button>
        </div>
        {error && <p className="error">{error}</p>}
      </form>
    </Dialog>
  );
}
