import { useEffect, useRef, useState } from "react";
import { Channel } from "@tauri-apps/api/core";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import "@xterm/xterm/css/xterm.css";
import { TerminalEvent, api } from "../api";

const bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));

const THEME = {
  background: "#1d1b24",
  foreground: "#ece7df",
  cursor: "#f4c86a",
  selectionBackground: "rgba(244, 200, 106, 0.3)",
  black: "#2a2433",
  brightBlack: "#6f6878",
};

/**
 * One of the office's own terminals, live: what Claude draws, and your keys back to it.
 * Escape goes to Claude (it interrupts a turn there), so the panel closes from its button.
 */
export function TerminalPanel({ id, title, oneOff, onClose }: { id: string; title: string; oneOff: boolean; onClose: () => void }) {
  const host = useRef<HTMLDivElement>(null);
  const [exit, setExit] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!host.current) return;
    const term = new Terminal({
      fontFamily: "Menlo, Monaco, monospace",
      fontSize: 12.5,
      lineHeight: 1.15,
      cursorBlink: true,
      macOptionIsMeta: true,
      allowProposedApi: true,
      scrollback: 5000,
      theme: THEME,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";
    term.open(host.current);
    fit.fit();

    // Shift+Return is a new line in Claude's message box, as in the terminals it sets itself up for.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type === "keydown" && e.key === "Enter" && e.shiftKey) {
        api.terminalWrite(id, "\u001b\r");
        return false;
      }
      return true;
    });

    let key: number | null = null;
    let ready = false;
    const early: Uint8Array[] = [];
    const channel = new Channel<TerminalEvent>();
    channel.onmessage = (e) => {
      if ("exit" in e) {
        setExit(e.exit);
        return;
      }
      const b = bytes(e.output);
      if (ready) term.write(b);
      else early.push(b);
    };
    let gone = false;
    api
      .terminalAttach(id, channel)
      .then(([k, screen]) => {
        if (gone) {
          api.terminalDetach(id, k);
          return;
        }
        key = k;
        term.write(bytes(screen));
        early.forEach((b) => term.write(b));
        ready = true;
        api.terminalResize(id, term.cols, term.rows);
        term.focus();
      })
      .catch((e) => setError(String(e)));

    const input = term.onData((d) => {
      api.terminalWrite(id, d).catch(() => {});
    });
    const resized = new ResizeObserver(() => {
      fit.fit();
      if (ready) api.terminalResize(id, term.cols, term.rows).catch(() => {});
    });
    resized.observe(host.current);

    return () => {
      gone = true;
      resized.disconnect();
      input.dispose();
      if (key !== null) api.terminalDetach(id, key);
      term.dispose();
    };
  }, [id]);

  // A one-off that worked (a sign-in, say) closes itself; a failed one stays for you to read.
  useEffect(() => {
    if (exit !== 0) return;
    const t = setTimeout(onClose, 900);
    return () => clearTimeout(t);
  }, [exit, onClose]);

  // A one-off (a sign-in) can't be opened again once its panel closes, so closing ends it.
  const close = () => {
    if (oneOff) api.terminalDismiss(id);
    onClose();
  };

  return (
    <section className="term-panel" role="dialog" aria-label={title}>
      <header className="term-head">
        <h2>{title}</h2>
        {exit !== null && <span className={`term-exit${exit === 0 ? "" : " bad"}`}>{exit === 0 ? "Done" : `Ended with code ${exit}`}</span>}
        {error && <span className="term-exit bad">{error}</span>}
        <span className="term-hint">{oneOff && exit === null ? "Closing this ends it" : "Esc goes to Claude"}</span>
        <button className="close" onClick={close} aria-label="Close the terminal">
          <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden="true">
            <path d="M2 2l8 8M10 2l-8 8" />
          </svg>
        </button>
      </header>
      <div className="term-body" ref={host} />
    </section>
  );
}
