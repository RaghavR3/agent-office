import { useEffect, useRef, useState } from "react";
import { Snapshot } from "../api";
import { Focus, World } from "./world";

function readHintSeen() {
  try {
    return localStorage.getItem("office-hint") === "1";
  } catch {
    return false;
  }
}

/** The canvas office. Dialogs and the HUD live in App; this owns the world and the "E to talk" prompt. */
export function Office({
  snap,
  selected,
  travel,
  cosState,
  dialogOpen,
  onWorld,
  onOpenAgent,
  onOpenCos,
  onHire,
  onAddRoom,
}: {
  snap: Snapshot;
  selected: string | null;
  travel: { id: string; n: number } | null;
  cosState: "idle" | "busy" | "asking" | "brief";
  dialogOpen: boolean;
  onWorld: (w: World) => void;
  onOpenAgent: (id: string) => void;
  onOpenCos: () => void;
  onHire: (roomId: string, roomName: string, slot: number) => void;
  onAddRoom: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const world = useRef<World | null>(null);
  const [focus, setFocus] = useState<Focus>(null);
  const [hint, setHint] = useState(!readHintSeen());
  const cb = useRef({ onOpenAgent, onOpenCos, onHire, onAddRoom });
  cb.current = { onOpenAgent, onOpenCos, onHire, onAddRoom };

  useEffect(() => {
    const w = new World(host.current!, {
      onFocus: setFocus,
      onOpen: (id) => cb.current.onOpenAgent(id),
      onCos: () => cb.current.onOpenCos(),
      onHire: (roomId, roomName, slot) => cb.current.onHire(roomId, roomName, slot),
      onAddRoom: () => cb.current.onAddRoom(),
    });
    world.current = w;
    onWorld(w);
    return () => w.dispose();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    world.current?.setData(snap.rooms, snap.agents, snap.cosName, selected, cosState);
  }, [snap, selected, cosState]);

  useEffect(() => {
    if (travel) world.current?.goTo(travel.id);
  }, [travel]);

  useEffect(() => world.current?.setDialogOpen(dialogOpen), [dialogOpen]);

  useEffect(() => {
    if (!hint) return;
    const t = setTimeout(() => {
      setHint(false);
      try {
        localStorage.setItem("office-hint", "1");
      } catch {
        /* private window: the hint just shows again next time */
      }
    }, 12000);
    return () => clearTimeout(t);
  }, [hint]);

  const prompt =
    focus?.kind === "agent"
      ? `Talk to ${focus.name}`
      : focus?.kind === "cos"
        ? `Hand ${focus.name} a task`
        : focus?.kind === "hire"
          ? `Hire someone for ${focus.roomName}`
          : focus?.kind === "addroom"
            ? "Add a room"
            : null;

  return (
    <div className="office">
      <div ref={host} className="canvas-host" />
      {hint && !dialogOpen && (
        <p className="hint">Walk with WASD or a click, scroll to look around. Press E beside someone to talk.</p>
      )}
      {prompt && !dialogOpen && (
        <div className="prompt" aria-live="polite">
          <kbd>E</kbd>
          {prompt}
        </div>
      )}
    </div>
  );
}
