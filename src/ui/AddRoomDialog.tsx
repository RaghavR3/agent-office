import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { Dialog } from "./Dialog";

export function AddRoomDialog({ host, onClose }: { host: string; onClose: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const pick = async () => {
    setError(null);
    const dir = await open({ directory: true, multiple: false, title: "Choose a folder for the new room" });
    if (typeof dir !== "string") return;
    setBusy(true);
    try {
      await api.addRoom(dir);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      label="Add a room"
      onClose={onClose}
      head={
        <span className="portrait t-empty" style={{ width: 52, height: 52 }} aria-hidden="true">
          <svg viewBox="0 0 16 16" width="22" height="22">
            <path d="M8 3v10M3 8h10" />
          </svg>
        </span>
      }
      title="Add a room"
      sub={<span>A room is a folder your agents work in, usually a code repo.</span>}
    >
      <p className="topic">
        {host === "Orca" ? "Pick a folder and it's added to Orca." : "Pick a folder."} Its room appears here with a free desk,
        ready to hire into.
      </p>
      <div className="handoff-row">
        <span className="quiet">Agents hired there start in that folder.</span>
        <button className="btn primary" onClick={pick} disabled={busy}>
          {busy ? "Adding…" : "Choose a folder"}
        </button>
      </div>
      {error && <p className="error">{error}</p>}
    </Dialog>
  );
}
