import { useEffect, useRef, useState } from "react";
import { Agent, api } from "./api";
import { Markdown } from "./Markdown";
import { Attachment, Composer, withAttachments } from "./ui/Composer";

/** The card an agent leaves behind: headline, the few facts that matter, decisions, links. */
export function BriefView({ agent, onReplied }: { agent: Agent; onReplied?: () => void }) {
  const b = agent.brief;
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const [sending, setSending] = useState(false);
  if (!b) return null;

  const withOptions = b.questions.filter((q) => q.options.length > 0);
  const ready = withOptions.length > 0 && withOptions.every((_, i) => answers[i]);

  const send = async (text: string) => {
    setSending(true);
    try {
      await api.messageAgent(agent.id, text);
      setAnswers({});
      onReplied?.();
    } finally {
      setSending(false);
    }
  };

  const pick = (qi: number, opt: string) => {
    const next = { ...answers, [qi]: opt };
    setAnswers(next);
    // One question: the click is the answer. Several: wait until each has one.
    if (withOptions.length === 1) send(opt);
  };

  if (b.body) return <Summary headline={b.headline} questions={b.questions.map((q) => q.q)} body={b.body} />;
  return (
    <div className="brief">
      <p className="headline">{b.headline}</p>
      {b.points.length > 0 && (
        <ul className="points">
          {b.points.map((p, i) => (
            <li key={i}>{p}</li>
          ))}
        </ul>
      )}
      {b.questions.map((q, qi) => (
        <div className="question" key={qi}>
          <p>{q.q}</p>
          {q.options.length > 0 && (
            <div className="options" role="group" aria-label={q.q}>
              {q.options.map((o) => (
                <button
                  key={o}
                  className={`opt${answers[qi] === o ? " on" : ""}`}
                  disabled={sending}
                  onClick={() => pick(qi, o)}
                >
                  {o}
                </button>
              ))}
            </div>
          )}
        </div>
      ))}
      {withOptions.length > 1 && (
        <button
          className="btn primary"
          disabled={!ready || sending}
          onClick={() => send(withOptions.map((q, i) => `${q.q} ${answers[i]}`).join("\n"))}
        >
          Send answers
        </button>
      )}
    </div>
  );
}

export function Reply({
  agent,
  onSent,
  restore,
}: {
  agent: Agent;
  onSent?: (text: string) => void;
  /** A message taken back from the queue, put back in the box to edit. */
  restore?: { text: string; n: number } | null;
}) {
  const [text, setText] = useState("");
  useEffect(() => {
    if (restore) setText(restore.text);
  }, [restore]);
  const [files, setFiles] = useState<Attachment[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<"sending" | "sent" | null>(null);
  const inFlight = useRef(false);
  const busy = agent.state === "working" || agent.state === "starting";

  useEffect(() => {
    if (status !== "sent") return;
    const t = setTimeout(() => setStatus(null), 2500);
    return () => clearTimeout(t);
  }, [status]);

  // The terminal can take a second to take the message, so the box clears at once and a
  // second Enter does nothing until it's through. If it fails, your words come back.
  const send = async () => {
    const msg = withAttachments(text, files);
    if (!msg || inFlight.current) return;
    inFlight.current = true;
    const [keepText, keepFiles] = [text, files];
    setText("");
    setFiles([]);
    setError(null);
    setStatus("sending");
    try {
      await api.messageAgent(agent.id, msg);
      setStatus("sent");
      onSent?.(msg);
    } catch (e) {
      setText(keepText);
      setFiles(keepFiles);
      setError(String(e));
      setStatus(null);
    } finally {
      inFlight.current = false;
    }
  };
  return (
    <div className="reply">
      <Composer
        value={text}
        onChange={setText}
        files={files}
        onFiles={setFiles}
        onSubmit={send}
        skills={{ agentId: agent.id }}
        placeholder={busy ? `${agent.name} is working. Your message waits until the turn ends.` : `Reply to ${agent.name}`}
      />
      {status && (
        <p className={`send-status ${status}`} role="status">
          {status === "sending" ? <i className="pulse" /> : "✓"}
          {status === "sending" ? `Sending to ${agent.name}…` : busy ? `Sent. ${agent.name} reads it when this turn ends.` : "Sent"}
        </p>
      )}
      {error && <p className="error">{error}</p>}
    </div>
  );
}

/** A reply without a brief: its opening line and any question for you, the rest on request. */
function Summary({ headline, questions, body }: { headline: string; questions: string[]; body: string }) {
  const [open, setOpen] = useState(false);
  const more = body.replace(/\s+/g, " ").length > headline.length + questions.join(" ").length + 40;
  return (
    <div className="brief">
      <p className="headline">{headline}</p>
      {questions.map((q) => (
        <p className="question-line" key={q}>
          {q}
        </p>
      ))}
      {more && (
        <button className="link-more" onClick={() => setOpen(!open)}>
          {open ? "Show less" : "Show all"}
        </button>
      )}
      {open && (
        <div className="brief-full">
          <Markdown text={body} />
        </div>
      )}
    </div>
  );
}
