import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Snapshot, api, hostName, isBusy } from "./api";
import { Office } from "./office/Office";
import type { World } from "./office/world";
import { Hud, finishedList, needsList } from "./ui/Hud";
import { AddRoomDialog } from "./ui/AddRoomDialog";
import { AgentDialog } from "./ui/AgentDialog";
import { CosDialog } from "./ui/CosDialog";
import { HireDialog } from "./ui/HireDialog";
import { NameCosDialog } from "./ui/NameCosDialog";
import { SettingsDialog } from "./ui/SettingsDialog";
import { Toasts } from "./ui/Toasts";

type Open =
  | { kind: "agent"; id: string }
  | { kind: "cos" }
  | { kind: "hire"; roomId: string; roomName: string; slot: number | null }
  | { kind: "addroom" }
  | { kind: "settings" }
  | { kind: "name" }
  | null;

function useNow(ms: number) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

const typing = (t: EventTarget | null) => t instanceof HTMLElement && (t.tagName === "TEXTAREA" || t.tagName === "INPUT");

export default function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [open, setOpen] = useState<Open>(null);
  const [travel, setTravel] = useState<{ id: string; n: number } | null>(null);
  const [team, setTeam] = useState(false);
  const world = useRef<World | null>(null);
  const now = useNow(30_000);

  useEffect(() => {
    api.snapshot().then(setSnap);
    const un = listen<Snapshot>("snapshot", (e) => setSnap(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);

  const askCosName = snap?.askCosName;
  useEffect(() => {
    if (askCosName) setOpen({ kind: "name" });
  }, [askCosName]);

  const goTo = useCallback((id: string) => {
    setTravel((t) => ({ id, n: (t?.n ?? 0) + 1 }));
    setOpen({ kind: "agent", id });
  }, []);
  const openCos = useCallback(() => setOpen({ kind: "cos" }), []);
  const close = useCallback(() => setOpen(null), []);

  const waiting = snap ? needsList(snap) : [];
  const finished = snap ? finishedList(snap) : [];
  const waitingRef = useRef(waiting);
  waitingRef.current = waiting;

  useEffect(() => {
    let cursor = 0;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen({ kind: "cos" });
        return;
      }
      if (typing(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key.toLowerCase() === "t") setTeam((t) => !t);
      // N walks you to the next person waiting on you.
      if (e.key.toLowerCase() === "n") {
        // Swallow the key, or the dialog that opens would type it.
        e.preventDefault();
        const list = waitingRef.current;
        if (!list.length) return;
        const w = list[cursor++ % list.length];
        if (w.kind === "cos") setOpen({ kind: "cos" });
        else goTo(w.agent.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [goTo]);

  if (!snap) return <div className="boot" data-tauri-drag-region />;

  const agent = open?.kind === "agent" ? snap.agents.find((a) => a.id === open.id) : undefined;
  const host = hostName(snap.setup);
  const openSettings = () => setOpen({ kind: "settings" });
  const cosAgent = snap.agents.find((a) => a.role === "cos");
  const cosState = snap.requests.some((r) => r.state === "asking")
    ? "asking"
    : snap.requests.some(isBusy) || snap.briefingBusy
      ? "busy"
      : snap.briefing?.items.length
        ? "brief"
        : "idle";

  return (
    <div className="app">
      <Office
        snap={snap}
        selected={agent?.id ?? (open?.kind === "cos" ? (cosAgent?.id ?? null) : null)}
        travel={travel}
        cosState={cosState}
        dialogOpen={!!open}
        onWorld={(w) => (world.current = w)}
        onOpenAgent={(id) => setOpen({ kind: "agent", id })}
        onOpenCos={openCos}
        onHire={(roomId, roomName, slot) => setOpen({ kind: "hire", roomId, roomName, slot })}
        onAddRoom={() => setOpen({ kind: "addroom" })}
      />
      <Hud
        snap={snap}
        needs={waiting}
        finished={finished}
        team={team}
        onToggleTeam={() => setTeam((t) => !t)}
        onOpenAgent={goTo}
        onOpenCos={openCos}
        onAddRoom={() => setOpen({ kind: "addroom" })}
        onSettings={openSettings}
        onZoom={(d) => world.current?.zoomBy(d)}
      />
      <Toasts requests={snap.requests} onGo={goTo} />

      {!snap.connected && !open && (
        <div className="notice">
          <h2>{snap.setup.preference === "auto" ? "Orca and cmux aren't running" : `${host} isn't running`}</h2>
          <p>
            Agent Office shows the Claude Code sessions you run in Orca or cmux. Open one and the office fills in by itself.
          </p>
          <button className="btn ghost" onClick={openSettings}>
            Settings
          </button>
        </div>
      )}
      {snap.connected && snap.rooms.length === 0 && !open && (
        <div className="notice">
          <h2>No rooms yet</h2>
          {snap.setup.host === "orca" ? (
            <p>Each repo or folder you add to Orca becomes a room here. Add one in Orca to open the office.</p>
          ) : (
            <p>
              Start Claude in a cmux terminal and its folder becomes a room, or add a folder yourself.
              {!snap.setup.hooks && " First, install the status hook in Settings so the office can see your sessions."}
            </p>
          )}
          {snap.setup.host === "cmux" && (
            <div className="notice-actions">
              <button className="btn ghost" onClick={openSettings}>
                Settings
              </button>
              <button className="btn primary" onClick={() => setOpen({ kind: "addroom" })}>
                Add a room
              </button>
            </div>
          )}
        </div>
      )}
      {snap.connected && snap.error && <p className="error-chip">{snap.error}</p>}

      {agent && <AgentDialog agent={agent} rooms={snap.rooms} now={now} host={host} onClose={close} />}
      {open?.kind === "cos" && <CosDialog name={snap.cosName} requests={snap.requests} snap={snap} onGo={goTo} onClose={close} />}
      {open?.kind === "hire" && (
        <HireDialog
          room={snap.rooms.find((r) => r.id === open.roomId)}
          roomName={open.roomName}
          desk={open.slot}
          host={host}
          saved={snap.saved}
          onClose={close}
        />
      )}
      {open?.kind === "addroom" && <AddRoomDialog host={host} onClose={close} />}
      {open?.kind === "settings" && <SettingsDialog setup={snap.setup} onClose={close} />}
      {open?.kind === "name" && <NameCosDialog name={snap.cosName} onClose={close} />}
    </div>
  );
}
