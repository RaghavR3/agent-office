import { useEffect, useRef, useState } from "react";
import { CosRequest } from "../api";

interface Toast {
  id: string;
  text: string;
  agentId: string | null;
  failed: boolean;
}

/** Briefly announces what the chief of staff did with each hand-off. */
export function Toasts({ requests, onGo }: { requests: CosRequest[]; onGo: (agentId: string) => void }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const seen = useRef<Map<string, string> | null>(null);

  useEffect(() => {
    const prev = seen.current;
    const next = new Map(requests.map((r) => [r.id, r.state]));
    seen.current = next;
    if (!prev) return;
    const fresh = requests.filter(
      (r) => ["routed", "replied", "failed"].includes(r.state) && prev.get(r.id) !== r.state && r.result,
    );
    if (!fresh.length) return;
    setToasts((t) => [
      ...t,
      ...fresh.map((r) => ({ id: `${r.id}:${r.state}`, text: r.result!, agentId: r.agentId, failed: r.state === "failed" })),
    ]);
    for (const r of fresh) {
      const id = `${r.id}:${r.state}`;
      setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 6000);
    }
  }, [requests]);

  if (!toasts.length) return null;
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast${t.failed ? " failed" : ""}`}>
          <span>{t.text}</span>
          {t.agentId && (
            <button onClick={() => onGo(t.agentId!)} className="go">
              Go there
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
