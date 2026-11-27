import { useState } from "react";
import { Room, SavedSession, ago, api, areaColor } from "../api";
import { Dialog } from "./Dialog";
import { Attachment, Composer, withAttachments } from "./Composer";

export function HireDialog({
  room,
  roomName,
  host,
  saved,
  desk,
  onClose,
}: {
  room?: Room;
  roomName: string;
  desk: number | null;
  host: string;
  saved: SavedSession[];
  onClose: () => void;
}) {
  const now = Date.now();
  const mine = saved.filter((s) => s.roomId === room?.id && !s.open);
  const [task, setTask] = useState("");
  const [files, setFiles] = useState<Attachment[]>([]);
  const areas = room?.areas ?? [];
  const [area, setArea] = useState<string | null>(areas[0]?.id ?? null);
  const hire = () => {
    const text = withAttachments(task, files);
    if (room) api.spawnAgent(room.id, area, text || null, desk);
    onClose();
  };
  return (
    <Dialog
      label={`Hire for ${roomName}`}
      onClose={onClose}
      head={
        <span className="portrait t-empty" style={{ width: 52, height: 52 }} aria-hidden="true">
          <svg viewBox="0 0 16 16" width="22" height="22">
            <path d="M8 3v10M3 8h10" />
          </svg>
        </span>
      }
      title={`A new desk in ${roomName}`}
      sub={<span>The new agent walks in, sits down and starts on whatever you write here.</span>}
    >
      {areas.length > 1 && (
        <div className="area-pick" role="radiogroup" aria-label="Area">
          <span className="quiet">Area</span>
          {areas.map((a) => (
            <button
              key={a.id}
              role="radio"
              aria-checked={area === a.id}
              className={`opt${area === a.id ? " on" : ""}`}
              style={area === a.id ? { background: areaColor(a.name), borderColor: areaColor(a.name) } : undefined}
              onClick={() => setArea(a.id)}
            >
              {a.name}
            </button>
          ))}
        </div>
      )}
      {mine.length > 0 && (
        <section className="saved-list" aria-label="Saved sessions">
          <h3>Bring back a saved session</h3>
          <ul>
            {mine.map((s) => (
              <li key={s.id}>
                <span>
                  <b>{s.name}</b> <em>saved {ago(s.savedAt, now)}</em>
                </span>
                <button className="btn ghost" onClick={() => api.forgetSession(s.id)} title="Take it off this list. The session itself stays in Claude.">
                  Forget
                </button>
                <button
                  className="btn primary"
                  onClick={() => {
                    api.resumeSession(s.id);
                    onClose();
                  }}
                >
                  Bring back
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      <form
        className="handoff"
        onSubmit={(e) => {
          e.preventDefault();
          hire();
        }}
      >
        <Composer
          value={task}
          onChange={setTask}
          files={files}
          onFiles={setFiles}
          onSubmit={hire}
          skills={{ roomId: room?.id }}
          placeholder="What should they work on? Leave it blank to start them empty. Type / for skills."
          rows={3}
          autoFocus
        />
        <div className="handoff-row">
          <span className="quiet">They open as a new Claude session{host === "the terminal" ? "" : ` in ${host}`}.</span>
          <button type="submit" className="btn primary">
            Hire
          </button>
        </div>
      </form>
    </Dialog>
  );
}
