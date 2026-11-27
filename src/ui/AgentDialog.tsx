import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Agent, Helper, Links as LinkSet, Room, WorkEvent, ago, api, needsYou, stateLine } from "../api";
import { BriefView, Reply } from "../BriefView";
import { Markdown } from "../Markdown";
import { Dialog, Portrait } from "./Dialog";

type Block =
  | { kind: "prompt"; text: string; queued?: boolean }
  | { kind: "said"; text: string }
  | { kind: "steps"; steps: { name: string; detail: string }[] };

function group(events: WorkEvent[]): Block[] {
  const out: Block[] = [];
  for (const e of events) {
    if (e.kind === "tool") {
      const last = out[out.length - 1];
      if (last?.kind === "steps") last.steps.push({ name: e.name, detail: e.detail });
      else out.push({ kind: "steps", steps: [{ name: e.name, detail: e.detail }] });
    } else if (e.kind === "prompt") out.push({ kind: "prompt", text: e.text, queued: e.queued });
    else out.push({ kind: e.kind, text: e.text });
  }
  return out;
}

function Steps({ steps }: { steps: { name: string; detail: string }[] }) {
  const [open, setOpen] = useState(false);
  const last = steps[steps.length - 1];
  return (
    <div className="steps">
      <button className="steps-toggle" onClick={() => setOpen(!open)} aria-expanded={open}>
        <b>{steps.length === 1 ? "1 step" : `${steps.length} steps`}</b>
        {!open && (
          <span>
            {last.name}
            {last.detail ? `: ${last.detail}` : ""}
          </span>
        )}
      </button>
      {open && (
        <ol>
          {steps.map((s, i) => (
            <li key={i}>
              <b>{s.name}</b> {s.detail}
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

/** What the agent reported or made: its PRs, pages, docs and previews. Passing mentions are left out. */
function Links({ agent }: { agent: Agent }) {
  const [links, setLinks] = useState<LinkSet>({ top: [], more: [] });
  const [all, setAll] = useState(false);
  useEffect(() => {
    let live = true;
    api
      .links(agent.id)
      .then((l) => live && setLinks(l))
      .catch(() => live && setLinks({ top: [], more: [] }));
    return () => {
      live = false;
    };
  }, [agent.id, agent.updatedAt]);
  const shown = all ? [...links.top, ...links.more] : links.top;
  if (!shown.length && !links.more.length) return null;
  return (
    <section className="links-box" aria-label="Links">
      <h3>Links</h3>
      <ul>
        {shown.map((l) => (
          <li key={l.url}>
            <button onClick={() => api.openLink(l.url)} title={l.url}>
              <span className={`kind k-${l.kind.toLowerCase()}`}>{l.kind}</span>
              <span className="label">{l.label}</span>
            </button>
          </li>
        ))}
      </ul>
      {links.more.length > 0 && (
        <button className="link-more" onClick={() => setAll(!all)}>
          {all ? "Fewer" : `${links.more.length} more mentioned along the way`}
        </button>
      )}
    </section>
  );
}

/** The agent's transcript, fetched once for the card and kept fresh while they work. */
function useWork(agent: Agent) {
  const [events, setEvents] = useState<WorkEvent[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [nudge, setNudge] = useState(0);
  useEffect(() => {
    let live = true;
    const load = () =>
      api
        .work(agent.id)
        .then((ev) => live && (setEvents(ev), setError(null)))
        .catch((e) => live && setError(String(e)));
    load();
    // A message you just sent lands in the transcript a moment later, so look again soon after.
    const soon = nudge ? setTimeout(load, 700) : undefined;
    const t = agent.state === "working" || agent.state === "starting" ? setInterval(load, 2500) : undefined;
    return () => {
      live = false;
      if (t) clearInterval(t);
      if (soon) clearTimeout(soon);
    };
  }, [agent.id, agent.updatedAt, agent.state, nudge]);
  return { events, error, reload: () => setNudge((n) => n + 1) };
}

/** The ⋯ menu for actions you need now and then, so the footer stays short as more are added. */
function MoreMenu({ items }: { items: { label: string; onClick: () => void; danger?: boolean; confirm?: string }[] }) {
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return setConfirming(null);
    const onDown = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);
  return (
    <div className="more" ref={ref}>
      <button className="btn ghost more-btn" aria-label="More actions" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
          <circle cx="3.5" cy="8" r="1.4" />
          <circle cx="8" cy="8" r="1.4" />
          <circle cx="12.5" cy="8" r="1.4" />
        </svg>
      </button>
      {open && (
        <ul className="more-menu" role="menu">
          {items.map((it) => (
            <li key={it.label} role="none">
              <button
                role="menuitem"
                className={it.danger ? "danger" : ""}
                onClick={() => {
                  if (it.confirm && confirming !== it.label) return setConfirming(it.label);
                  setOpen(false);
                  it.onClick();
                }}
              >
                {confirming === it.label ? it.confirm : it.label}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Clamp({ text, lines }: { text: string; lines: number }) {
  const [open, setOpen] = useState(false);
  const long = text.length > lines * 90 || text.split("\n").length > lines;
  return (
    <>
      <div className={open || !long ? "" : "clamp"} style={{ ["--lines" as string]: lines }}>
        <Markdown text={text} />
      </div>
      {long && (
        <button className="link-more" onClick={() => setOpen(!open)}>
          {open ? "Show less" : "Show all"}
        </button>
      )}
    </>
  );
}

/** What the agent is on, renamable: the name goes to Claude (`/rename`) and the session is saved,
 * so you can bring it back from the room later. */
function SessionName({ agent, room }: { agent: Agent; room: string }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(agent.task);
  const [note, setNote] = useState<string | null>(null);
  useEffect(() => setValue(agent.task), [agent.task]);
  if (agent.role === "cos") return null;
  if (editing)
    return (
      <form
        className="session-rename"
        onSubmit={async (e) => {
          e.preventDefault();
          try {
            await api.renameSession(agent.id, value);
            setEditing(false);
            setNote(`Saved. Bring it back any time from a free desk in ${room}.`);
          } catch (err) {
            setNote(String(err));
          }
        }}
      >
        <input autoFocus value={value} maxLength={60} placeholder="Name this session" aria-label="Session name" onChange={(e) => setValue(e.target.value)} onBlur={() => setEditing(false)} />
      </form>
    );
  return (
    <span className="task">
      {agent.task ? `On: ${agent.task}` : "Unnamed session"}
      <button className="link-btn" onClick={() => setEditing(true)} title="Name this session so you can come back to it">
        Rename
      </button>
      {note && <em className="session-note">{note}</em>}
    </span>
  );
}

/** Claude's own question box, answered here: pick options, write your own, or both. */
function AskForm({ agent, host }: { agent: Agent; host: string }) {
  const qs = agent.ask ?? [];
  const [picked, setPicked] = useState<string[][]>(() => qs.map(() => []));
  const [own, setOwn] = useState<string[]>(() => qs.map(() => ""));
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const answer = (i: number) => [...(picked[i] ?? []), (own[i] ?? "").trim()].filter(Boolean).join(", ");
  const ready = qs.some((_, i) => answer(i));

  const toggle = (i: number, label: string) =>
    setPicked((p) =>
      p.map((cur, j) => {
        if (j !== i) return cur;
        if (qs[i].multiSelect) return cur.includes(label) ? cur.filter((x) => x !== label) : [...cur, label];
        return cur[0] === label ? [] : [label];
      }),
    );

  const send = async () => {
    setSending(true);
    setError(null);
    try {
      await api.answerAsk(
        agent.id,
        qs.map((_, i) => answer(i)),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  };

  return (
    <section className="ask-form" aria-label={`Questions from ${agent.name}`}>
      <p className="ask-title">
        {agent.name} has {qs.length === 1 ? "a question" : `${qs.length} questions`} for you
      </p>
      {qs.map((q, i) => (
        <fieldset key={i} className="ask-q">
          <legend>
            {q.header && <span className="ask-header">{q.header}</span>}
            {q.question}
          </legend>
          {q.options.length > 0 && (
            <div className="ask-options">
              {q.options.map((o) => {
                const on = picked[i]?.includes(o.label) ?? false;
                return (
                  <button type="button" key={o.label} className={`ask-opt${on ? " on" : ""}`} aria-pressed={on} onClick={() => toggle(i, o.label)}>
                    <b>{o.label}</b>
                    {o.description && <span>{o.description}</span>}
                  </button>
                );
              })}
            </div>
          )}
          <input
            className="ask-own"
            value={own[i] ?? ""}
            placeholder={q.options.length ? (q.multiSelect ? "Pick any that apply, or add your own" : "Or write your own answer") : "Your answer"}
            aria-label={`Your own answer to: ${q.question}`}
            onChange={(e) => setOwn((o) => o.map((v, j) => (j === i ? e.target.value : v)))}
          />
        </fieldset>
      ))}
      <div className="handoff-row">
        <span className="quiet">Sending closes the question in {host} and passes your answers to {agent.name}.</span>
        <button className="btn primary" disabled={!ready || sending} onClick={send}>
          {sending ? "Sending…" : qs.length > 1 ? "Send answers" : "Send answer"}
        </button>
      </div>
      {error && <p className="error">{error}</p>}
    </section>
  );
}

const same = (a: string, b: string) => a.trim().slice(0, 120) === b.trim().slice(0, 120);

/** Your last message and their latest update, so you know where things stand. */
function Latest({ agent, events, busy, sent }: { agent: Agent; events: WorkEvent[]; busy: boolean; sent: { text: string; at: number } | null }) {
  const found = [...events].reverse().find((e) => e.kind === "prompt");
  // What you just sent shows straight away, until the transcript catches up.
  const fresh = sent && Date.now() - sent.at < 30_000 && !(found?.kind === "prompt" && same(found.text, sent.text));
  const prompt = fresh ? { kind: "prompt" as const, text: sent.text, ts: "", queued: busy } : found;
  const said = [...events].reverse().find((e) => e.kind === "said");
  const showSaid = said && (busy || !agent.brief || !!agent.ask);
  if (!prompt && !showSaid) return null;
  return (
    <div className="latest">
      {prompt && prompt.kind === "prompt" && (
        <div className="latest-you">
          <span>{prompt.queued ? "You, waiting its turn" : "You"}</span>
          <Clamp text={prompt.text} lines={3} />
        </div>
      )}
      {showSaid && said.kind === "said" && (
        <div className="latest-said">
          <span>{busy ? `${agent.name}, latest` : agent.name}</span>
          <Clamp text={said.text} lines={5} />
        </div>
      )}
    </div>
  );
}

/** A thread, like a chat: opens at the latest message and follows it unless you've scrolled up. */
function Thread({ events, asker, className }: { events: WorkEvent[]; asker: string; className: string }) {
  const blocks = group(events);
  const box = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  useLayoutEffect(() => {
    const el = box.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  }, [events]);
  return (
    <div
      className={className}
      ref={box}
      onScroll={(e) => {
        const el = e.currentTarget;
        pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
      }}
    >
      {blocks.map((b, i) =>
        b.kind === "prompt" ? (
          <div key={i} className="you">
            <span>{b.queued ? `${asker}, waiting its turn` : asker}</span>
            <Markdown text={b.text} />
          </div>
        ) : b.kind === "said" ? (
          <div key={i} className="said">
            <Markdown text={b.text} />
          </div>
        ) : (
          <Steps key={i} steps={b.steps} />
        ),
      )}
    </div>
  );
}

/** One helper's own thread, kept fresh while it runs. */
function HelperThread({ agentId, helper }: { agentId: string; helper: Helper }) {
  const [events, setEvents] = useState<WorkEvent[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    const load = () =>
      api
        .helperWork(agentId, helper.id)
        .then((ev) => live && (setEvents(ev), setError(null)))
        .catch((e) => live && setError(String(e)));
    load();
    const t = helper.running ? setInterval(load, 3000) : undefined;
    return () => {
      live = false;
      if (t) clearInterval(t);
    };
  }, [agentId, helper.id, helper.running]);
  if (error) return <p className="quiet thread-note">{error}</p>;
  if (!events) return <p className="quiet thread-note">Reading its thread…</p>;
  if (!events.length) return <p className="quiet thread-note">Nothing written yet.</p>;
  return <Thread events={events} asker="Asked" className="work thread" />;
}

const plainKind = (k: string) => !k || k === "helper" || k === "general-purpose";

/** Every helper the agent started, each opening onto its own thread. */
function Helpers({ agent, open, onOpen, now }: { agent: Agent; open: string | null; onOpen: (id: string | null) => void; now: number }) {
  const [list, setList] = useState<Helper[] | null>(null);
  const [all, setAll] = useState(false);
  useEffect(() => {
    let live = true;
    const load = () =>
      api
        .helpers(agent.id)
        .then((l) => live && setList(l))
        .catch(() => live && setList([]));
    load();
    const t = setInterval(load, 4000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [agent.id]);
  if (!list?.length) return null;
  const shown = all ? list : list.filter((h, i) => h.running || i < 4 || h.id === open);
  return (
    <section className="threads" aria-label={`${agent.name}'s helpers`}>
      <h3>Helpers</h3>
      <ul>
        {shown.map((h) => (
          <li key={h.id || h.task}>
            <button className="thread-row" disabled={!h.id} aria-expanded={open === h.id} onClick={() => onOpen(open === h.id ? null : h.id)}>
              <i className={h.running ? "pulse" : "tick"} aria-hidden="true" />
              <span className="thread-task">{h.task}</span>
              {!plainKind(h.kind) && <em>{h.kind}</em>}
              <span className="thread-time">{h.running ? "running" : h.updatedAt ? ago(h.updatedAt, now) : "done"}</span>
            </button>
            {open === h.id && <HelperThread agentId={agent.id} helper={h} />}
          </li>
        ))}
      </ul>
      {shown.length < list.length && (
        <button className="link-more" onClick={() => setAll(true)}>
          {list.length - shown.length} earlier
        </button>
      )}
    </section>
  );
}

function Work({
  agent,
  events,
  error,
  openHelper,
  onOpenHelper,
  now,
}: {
  agent: Agent;
  events: WorkEvent[] | null;
  error: string | null;
  openHelper: string | null;
  onOpenHelper: (id: string | null) => void;
  now: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.scrollIntoView({ block: "nearest" });
  }, []);
  return (
    <div className="work-panel" ref={ref}>
      <Helpers agent={agent} open={openHelper} onOpen={onOpenHelper} now={now} />
      {error ? (
        <p className="quiet">{error}</p>
      ) : !events ? (
        <p className="quiet">Reading the transcript…</p>
      ) : events.length === 0 ? (
        <p className="quiet">Nothing written yet.</p>
      ) : (
        <Thread events={events} asker="You" className="work" />
      )}
    </div>
  );
}

const tone = (a: Agent) => (needsYou(a) ? "red" : a.state === "working" ? "blue" : a.state === "done" ? "green" : undefined);

export function AgentDialog({
  agent,
  rooms,
  now,
  host,
  onClose,
}: {
  agent: Agent;
  rooms: Room[];
  now: number;
  host: string;
  onClose: () => void;
}) {
  const [showWork, setShowWork] = useState(false);
  const [openHelper, setOpenHelper] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [resetting, setResetting] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [sent, setSent] = useState<{ text: string; at: number } | null>(null);
  const [restore, setRestore] = useState<{ text: string; n: number } | null>(null);
  const room = rooms.find((r) => r.id === agent.roomId)?.name ?? "";
  const busy = agent.state === "working" || agent.state === "starting";
  const { events, error, reload } = useWork(agent);
  const lastPrompt = [...(events ?? [])].reverse().find((e) => e.kind === "prompt");
  const queued = lastPrompt?.kind === "prompt" && !!lastPrompt.queued;

  useEffect(() => {
    if (agent.unread) api.markSeen(agent.id);
  }, [agent.id, agent.unread]);
  useEffect(() => {
    setShowWork(false);
    setOpenHelper(null);
    setNotice(null);
    setSent(null);
  }, [agent.id]);

  // Takes back a message still waiting its turn (it comes back to the box to edit), or
  // interrupts what the agent is doing, as Escape does in the terminal.
  const stop = async () => {
    setStopping(true);
    setNotice(null);
    try {
      const r = await api.stopAgent(agent.id);
      if (r === "unqueued" && lastPrompt?.kind === "prompt") setRestore({ text: lastPrompt.text, n: Date.now() });
      setSent(null);
      setNotice(r === "unqueued" ? "Took your message back. It's in the box below if you want to change it." : `Stopped ${agent.name}.`);
      reload();
    } catch (e) {
      setNotice(String(e));
    } finally {
      setStopping(false);
    }
  };

  return (
    <Dialog
      label={agent.name}
      onClose={onClose}
      head={<Portrait who={agent} size={52} tone={tone(agent)} />}
      title={agent.name}
      sub={
        <>
          <span>
            {room}
            {agent.area ? `, ${agent.area}` : ""}
          </span>
          <span className={`state t-${tone(agent) ?? "none"}`}>{stateLine(agent, now)}</span>
          <SessionName agent={agent} room={room} />
        </>
      }
      footer={
        <>
          <button className="btn ghost" aria-expanded={showWork} onClick={() => setShowWork(!showWork)}>
            {showWork ? "Hide work" : "Show work"}
          </button>
          <span className="spacer" />
          {(busy || queued) && (
            <button
              className="btn ghost stop"
              disabled={stopping}
              title={queued ? "Takes back the message that's waiting its turn. What they're doing carries on." : `Interrupts ${agent.name}, like pressing Esc in the terminal`}
              onClick={stop}
            >
              <i aria-hidden="true" />
              {stopping ? "Stopping…" : queued ? "Take back message" : "Stop"}
            </button>
          )}
          {(agent.state === "done" || agent.state === "needs_you" || agent.state === "waiting") && (
            <button
              className="btn ghost"
              title="Marks this unread so it waits in Needs you / Finished until you come back"
              onClick={async () => {
                await api.markUnread(agent.id);
                onClose();
              }}
            >
              Read later
            </button>
          )}
          {!busy && agent.state !== "waiting" && !agent.ask && (
            <button
              className="btn ghost"
              disabled={resetting}
              title={`Clears ${agent.name}'s Claude session. They stay at this desk, ready for the next task.`}
              onClick={async () => {
                setNotice(null);
                setResetting(true);
                try {
                  await api.resetAgent(agent.id);
                } catch (e) {
                  setNotice(String(e));
                } finally {
                  setResetting(false);
                }
              }}
            >
              {resetting ? "Clearing…" : "Done, start fresh"}
            </button>
          )}
          <MoreMenu
            items={[
              { label: `Open in ${host}`, onClick: () => api.openTerminal(agent.id) },
              { label: "Remove", danger: true, confirm: `Close ${agent.name}'s ${host === "the terminal" ? "terminal" : "tab"}`, onClick: () => api.removeAgent(agent.id).then(onClose) },
            ]}
          />
        </>
      }
    >
      <Latest agent={agent} events={events ?? []} busy={busy} sent={sent} />
      {agent.helpers.length > 0 && (
        <section className="helpers-box" aria-label="Helpers">
          <h3>
            {agent.helpers.length === 1 ? "1 helper" : `${agent.helpers.length} helpers`} {busy ? "" : "still "}working for{" "}
            {agent.name}
          </h3>
          <ul>
            {agent.helpers.map((h, i) => (
              <li key={h.id || i}>
                <button
                  className="helper-link"
                  disabled={!h.id}
                  title={h.id ? "Read what this helper is doing" : undefined}
                  onClick={() => {
                    setShowWork(true);
                    setOpenHelper(h.id);
                  }}
                >
                  <i className="pulse" />
                  {h.task}
                  {!plainKind(h.kind) && <em>{h.kind}</em>}
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {agent.ask ? <AskForm agent={agent} host={host} /> : !busy && agent.brief && <BriefView agent={agent} />}
      {!busy && !agent.brief && !(events ?? []).length && <p className="quiet">Nothing to report yet.</p>}
      {(agent.state === "waiting" || agent.blocked) && (
        <button className="btn primary" onClick={() => api.openTerminal(agent.id)}>
          Answer in {host}
        </button>
      )}
      {notice && <p className="notice">{notice}</p>}
      <Links agent={agent} />
      {agent.state !== "waiting" && !agent.blocked && !agent.ask && (
        <Reply
          agent={agent}
          restore={restore}
          onSent={(text) => {
            setNotice(null);
            setSent({ text, at: Date.now() });
            reload();
          }}
        />
      )}
      {showWork && <Work agent={agent} events={events} error={error} openHelper={openHelper} onOpenHelper={setOpenHelper} now={now} />}
    </Dialog>
  );
}
