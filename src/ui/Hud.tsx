import { useState } from "react";
import { Agent, CosRequest, Room, Snapshot, api, areaColor, isBusy, needsSetup, needsYou } from "../api";
import { Portrait } from "./Dialog";
import { UsageButton, UsagePanel } from "./Usage";
import { ConnectionsButton, ConnectionsPanel } from "./Connections";

export type Waiting = { kind: "agent"; agent: Agent; room: string } | { kind: "cos"; req: CosRequest };

const roomName = (snap: Snapshot, id: string) => snap.rooms.find((r: Room) => r.id === id)?.name ?? "";
const where = (snap: Snapshot, a: Agent) => roomName(snap, a.roomId) + (a.area ? `, ${a.area}` : "");

export function AreaChip({ area }: { area: string | null }) {
  if (!area) return null;
  return (
    <span className="area-chip" style={{ background: areaColor(area) }}>
      {area}
    </span>
  );
}

/** Only things that are actually blocked on you: questions, permission prompts, Raggy asking where to route. */
export function needsList(snap: Snapshot): Waiting[] {
  const asks: Waiting[] = snap.requests.filter((r) => r.state === "asking").map((req) => ({ kind: "cos", req }));
  const agents: Waiting[] = snap.agents
    .filter((a) => a.role === "worker" && needsYou(a))
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .map((agent) => ({ kind: "agent", agent, room: where(snap, agent) }));
  return [...asks, ...agents];
}

export function finishedList(snap: Snapshot): Agent[] {
  return snap.agents.filter((a) => a.role === "worker" && a.state === "done" && a.unread).sort((a, b) => b.updatedAt - a.updatedAt);
}

const dot = (a: Agent) => (needsYou(a) ? "red" : a.state === "working" || a.state === "starting" ? "blue" : a.state === "done" ? "green" : "grey");

function Person({ a, onOpen }: { a: Agent; onOpen: (id: string) => void }) {
  return (
    <li>
      <button onClick={() => onOpen(a.id)}>
        <i className={`dot ${dot(a)}`} />
        <b>{a.name}</b>
        <span>{a.state === "working" && a.activity ? a.activity : a.task || "No task yet"}</span>
      </button>
      {a.helpers.length > 0 && (
        <ul className="helpers">
          {a.helpers.map((h, i) => (
            <li key={i}>{h.task}</li>
          ))}
        </ul>
      )}
    </li>
  );
}

/** A room's name, which you can click to rename, with controls to move or hide it. */
function RoomHeader({ room, count, first, last }: { room: Room; count: number; first: boolean; last: boolean }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(room.name);
  const save = () => {
    setEditing(false);
    if (value.trim() !== room.name) api.renameRoom(room.id, value.trim() === room.original ? "" : value);
  };
  if (editing)
    return (
      <form
        className="room-rename"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <input
          autoFocus
          value={value}
          maxLength={40}
          aria-label={`Name for ${room.original}`}
          placeholder={room.original}
          onChange={(e) => setValue(e.target.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              setValue(room.name);
              setEditing(false);
            }
          }}
        />
      </form>
    );
  return (
    <h3 className="room-head">
      <button
        className="room-name"
        onClick={() => {
          setValue(room.name);
          setEditing(true);
        }}
        title={room.name !== room.original ? `Rename (originally ${room.original})` : "Rename"}
      >
        {room.name}
      </button>
      <span className="room-tools">
        <button disabled={first} onClick={() => api.moveRoom(room.id, -1)} aria-label={`Move ${room.name} back`} title="Move back">
          ↑
        </button>
        <button disabled={last} onClick={() => api.moveRoom(room.id, 1)} aria-label={`Move ${room.name} on`} title="Move on">
          ↓
        </button>
        <button onClick={() => api.setRoomHidden(room.id, true)} title="Hide from the office (nothing changes in your terminal app)">
          Hide
        </button>
      </span>
      <span className="room-count">{count || ""}</span>
    </h3>
  );
}

function Team({ snap, onOpenAgent, onAddRoom }: { snap: Snapshot; onOpenAgent: (id: string) => void; onAddRoom: () => void }) {
  return (
    <aside className="team" aria-label="Team">
      {snap.rooms.map((room, i) => {
        const people = snap.agents.filter((a) => a.role === "worker" && a.roomId === room.id);
        // Rooms with named areas list each area, then anyone working outside them.
        const groups: [string | null, Agent[]][] = room.areas.length
          ? [
              ...room.areas.map((ar) => [ar.name, people.filter((a) => a.area === ar.name)] as [string, Agent[]]),
              [null, people.filter((a) => !a.area || !room.areas.some((ar) => ar.name === a.area))],
            ]
          : [[null, people]];
        return (
          <section key={room.id}>
            <RoomHeader room={room} count={people.length} first={i === 0} last={i === snap.rooms.length - 1} />
            {people.length === 0 && <p className="nobody">No one yet</p>}
            {groups.map(([area, list]) =>
              list.length === 0 ? null : (
                <div key={area ?? "rest"} className="area-group">
                  {area && <AreaChip area={area} />}
                  <ul>
                    {list.map((a) => (
                      <Person key={a.id} a={a} onOpen={onOpenAgent} />
                    ))}
                  </ul>
                </div>
              ),
            )}
          </section>
        );
      })}
      {snap.saved.some((s) => !s.open) && (
        <section className="side-list">
          <h3>Saved sessions</h3>
          <ul>
            {snap.saved
              .filter((s) => !s.open)
              .map((s) => (
                <li key={s.id}>
                  <span className="side-text" title={s.name}>
                    <b>{s.name}</b>
                    <span>{snap.rooms.find((r) => r.id === s.roomId)?.name ?? ""}</span>
                  </span>
                  <button className="glass-btn" onClick={() => api.resumeSession(s.id)}>
                    Bring back
                  </button>
                </li>
              ))}
          </ul>
        </section>
      )}
      {snap.hiddenRooms.length > 0 && (
        <section className="side-list">
          <h3>Hidden from the office</h3>
          <ul>
            {snap.hiddenRooms.map((r) => (
              <li key={r.id}>
                <span className="side-text" title={r.name}>
                  <b>{r.name}</b>
                </span>
                <button className="glass-btn" onClick={() => api.setRoomHidden(r.id, false)}>
                  Show
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}
      <button className="add-room" onClick={onAddRoom}>
        + Add a room
      </button>
    </aside>
  );
}

/** Floating controls over the office: counts, who needs you, the team list and the hand-off button. */
export function Hud({
  snap,
  needs,
  finished,
  team,
  onToggleTeam,
  onOpenAgent,
  onOpenCos,
  onAddRoom,
  onSettings,
  onZoom,
}: {
  snap: Snapshot;
  needs: Waiting[];
  finished: Agent[];
  team: boolean;
  onToggleTeam: () => void;
  onOpenAgent: (id: string) => void;
  onOpenCos: () => void;
  onAddRoom: () => void;
  onSettings: () => void;
  onZoom: (d: number) => void;
}) {
  const [panel, setPanel] = useState<"usage" | "mcp" | null>(null);
  const toggle = (p: "usage" | "mcp") => setPanel((cur) => (cur === p ? null : p));
  const workers = snap.agents.filter((a) => a.role === "worker");
  const working = workers.filter((a) => a.state === "working").length;
  const routing = snap.requests.some(isBusy);

  return (
    <>
      <div className="hud-top" data-tauri-drag-region>
        <span className="wordmark" data-tauri-drag-region>
          Agent Office
        </span>
        <span className="chip" data-tauri-drag-region>
          <i className="dot blue" />
          {working} working
        </span>
        {needs.length > 0 && (
          <span className="chip" data-tauri-drag-region>
            <i className="dot red" />
            {needs.length} {needs.length === 1 ? "needs" : "need"} you
          </span>
        )}
        {snap.setup.elsewhere.length > 0 && (
          <button
            className="bar-btn setup"
            onClick={() => api.setHost("builtin")}
            title={`${snap.setup.elsewhere.join(", ")} still running in the office's own terminals. Click to see them.`}
          >
            <i className="dot red" />
            {snap.setup.elsewhere.length} running in Built-in
          </button>
        )}
        <span className="spacer" data-tauri-drag-region />
        <ConnectionsButton servers={snap.mcp} open={panel === "mcp"} onToggle={() => toggle("mcp")} />
        {snap.usage && <UsageButton usage={snap.usage} open={panel === "usage"} onToggle={() => toggle("usage")} />}
        {needsSetup(snap.setup) && (
          <button className="bar-btn setup" onClick={onSettings}>
            <i className="dot red" />
            Finish cmux setup
          </button>
        )}
        <button
          className={`bar-btn icon${snap.notifications ? " on" : ""}`}
          onClick={() => api.setNotifications(!snap.notifications)}
          aria-pressed={snap.notifications}
          title={snap.notifications ? "Notifications are on: click to mute" : "Notifications are off: click to turn on"}
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="M8 2.2a3.6 3.6 0 0 0-3.6 3.6v2.3L3 10.6h10l-1.4-2.5V5.8A3.6 3.6 0 0 0 8 2.2Z" />
            <path d="M6.4 12.4a1.7 1.7 0 0 0 3.2 0" />
            {!snap.notifications && <path d="M2.5 2.5l11 11" />}
          </svg>
          {snap.notifications ? "On" : "Off"}
        </button>
        <button className={`bar-btn${team ? " on" : ""}`} onClick={onToggleTeam} aria-pressed={team}>
          Team <kbd>T</kbd>
        </button>
        <button className="bar-btn icon square" onClick={onSettings} title="Settings" aria-label="Settings">
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="M3 4.5h10M3 8h10M3 11.5h10" />
            <circle cx="6" cy="4.5" r="1.4" />
            <circle cx="10.5" cy="8" r="1.4" />
            <circle cx="5" cy="11.5" r="1.4" />
          </svg>
        </button>
      </div>

      {team && <Team snap={snap} onOpenAgent={onOpenAgent} onAddRoom={onAddRoom} />}
      {panel === "usage" && snap.usage && (
        <UsagePanel
          usage={snap.usage}
          agents={workers.length}
          midTask={[
            ...workers
              .filter((a) => a.state === "working" || a.state === "starting" || a.state === "waiting" || a.ask || a.helpers.length > 0)
              .map((a) => a.name),
            ...(snap.requests.some(isBusy) || snap.briefingBusy ? [snap.cosName] : []),
          ]}
          onClose={() => setPanel(null)}
        />
      )}
      {panel === "mcp" && <ConnectionsPanel servers={snap.mcp} onClose={() => setPanel(null)} />}

      {(needs.length > 0 || finished.length > 0) && (
        <aside className="waiting" aria-label="Needs you">
          {needs.length > 0 && (
            <>
              <h2>
                Needs you <kbd>N</kbd>
              </h2>
              <ul>
                {needs.slice(0, 5).map((w) =>
                  w.kind === "cos" ? (
                    <li key={w.req.id}>
                      <button onClick={onOpenCos}>
                        <Portrait who={{ name: snap.cosName, role: "cos" }} tone="red" />
                        <span className="who">
                          <b>{snap.cosName}</b>
                          <span>{w.req.question || "Where should this go?"}</span>
                        </span>
                      </button>
                    </li>
                  ) : (
                    <li key={w.agent.id}>
                      <button onClick={() => onOpenAgent(w.agent.id)}>
                        <Portrait who={w.agent} tone="red" />
                        <span className="who">
                          <b>
                            {w.agent.name} <em>{w.room}</em>
                          </b>
                          <span>{w.agent.brief?.headline || w.agent.task}</span>
                        </span>
                      </button>
                    </li>
                  ),
                )}
              </ul>
              {needs.length > 5 && <p className="more">{needs.length - 5} more</p>}
            </>
          )}
          {finished.length > 0 && (
            <>
              <h2 className={needs.length ? "sub" : ""}>
                Finished
                <button className="link-btn" onClick={() => finished.forEach((a) => api.markSeen(a.id))}>
                  Mark all read
                </button>
              </h2>
              <ul>
                {finished.slice(0, 3).map((a) => (
                  <li key={a.id}>
                    <button onClick={() => onOpenAgent(a.id)}>
                      <Portrait who={a} tone="green" />
                      <span className="who">
                        <b>
                          {a.name} <em>{where(snap, a)}</em>
                        </b>
                        <span>{a.brief?.headline || a.task}</span>
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
              {finished.length > 3 && <p className="more">{finished.length - 3} more</p>}
            </>
          )}
        </aside>
      )}

      <button className="handoff-btn" onClick={onOpenCos}>
        <Portrait who={{ name: snap.cosName, role: "cos" }} size={26} />
        <span>
          Hand off a task
          {routing ? (
            <em>{snap.cosName} is routing</em>
          ) : snap.briefing?.items.length ? (
            <em>
              {snap.briefing.items.length} {snap.briefing.items.length === 1 ? "thing" : "things"} from {snap.cosName}
            </em>
          ) : null}
        </span>
        <kbd>⌘K</kbd>
      </button>

      <div className="zoom" role="group" aria-label="Zoom">
        <button onClick={() => onZoom(1)} aria-label="Zoom in">
          +
        </button>
        <button onClick={() => onZoom(-1)} aria-label="Zoom out">
          −
        </button>
      </div>
    </>
  );
}
