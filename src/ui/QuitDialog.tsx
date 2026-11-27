import { api } from "../api";
import { Dialog } from "./Dialog";

const list = (names: string[]) =>
  names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;

/** Asked on ⌘Q while agents in the office's own terminals are mid-task, since quitting stops them. */
export function QuitDialog({ names, onClose }: { names: string[]; onClose: () => void }) {
  return (
    <Dialog
      label="Quit Agent Office"
      onClose={onClose}
      head={
        <span className="portrait t-empty" style={{ width: 52, height: 52 }} aria-hidden="true">
          <svg viewBox="0 0 16 16" width="22" height="22">
            <path d="M8 2.5v5M4.6 4.6a5 5 0 1 0 6.8 0" fill="none" />
          </svg>
        </span>
      }
      title={`Quit while ${list(names)} ${names.length === 1 ? "is" : "are"} working?`}
      sub={<span>Quitting stops them mid-task. Each one picks its session back up when you next open the office.</span>}
    >
      <div className="handoff-row">
        <span className="quiet">Closing the window instead keeps them working.</span>
        <button className="btn ghost" onClick={onClose} autoFocus>
          Keep working
        </button>
        <button className="btn primary" onClick={() => api.quitApp()}>
          Quit
        </button>
      </div>
    </Dialog>
  );
}
