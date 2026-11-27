# Agent Office

A walkable pixel office for your Claude Code agents. It runs them in its own terminals, or shows the
ones you run in [Orca](https://github.com/stablyai/orca) or [cmux](https://cmux.com). Each repo or
folder is a room, and each Claude session is someone at a desk. They type while
they work, stand up and wave when they need you, and leave a short brief when they finish: a
headline, a few points, questions you can answer with a click, and links. No walls of text.

https://github.com/user-attachments/assets/e3a1bc4d-6f90-4349-b68d-ee87a1204176

One task from hand-off to finished work: help the agent when it gets stuck, read its brief, then
check what it did. The agents and data are made up.

Raggy, the chief of staff at reception, takes anything you hand over and sends it to the right room,
asking you only when it could go to more than one. Click Raggy's name to rename them.

Workspaces you've named in Orca (say Building, GTM/Ops and Product in one repo), or a repo's git
worktrees elsewhere, become areas within that room. Agents wear their area next to their name, hiring
asks which area, and Raggy can route to one.

> A personal project, shared as is. No support, no warranty, no contributions. See
> [License and support](#license-and-support).

## Requirements

- macOS
- [Claude Code](https://claude.com/claude-code) installed and signed in. Agents run on your own Claude
  plan. That's all the office needs to run them itself.
- Optional: [Orca](https://github.com/stablyai/orca) or [cmux](https://cmux.com), if you'd rather your
  agents live there. For cmux, install the status hook from Settings, and in cmux's Settings set Socket
  Control Mode to allow local automation (or use a socket password and paste it into Agent Office's
  Settings).
- To build: Node 22+, Rust (`brew install rustup && rustup default stable`)

## Run it

```sh
npm install
npm run tauri dev                         # or: npm run tauri build -- --bundles app
```

If you installed Rust through Homebrew, put `/opt/homebrew/opt/rustup/bin` on your PATH first.

Or download the `.dmg` from [Releases](https://github.com/RaghavR3/agent-office/releases), when there is
one. Until releases are signed, macOS won't open it the first time: open System Settings, then Privacy &
Security, and choose Open Anyway.

## Using it

| Do this | To |
| --- | --- |
| WASD, arrow keys, or click the floor | Walk (hold Shift to hurry) |
| Click a person, or E beside them | Open their brief, answer, or read their work |
| ⌘K, or E beside Raggy | Hand off a task |
| E beside an empty desk | Hire a new agent for that room |
| N | Go to the next person waiting on you |
| Scroll, or + and − | Zoom |
| Esc | Close a dialog |

`http://localhost:1420/preview.html` (while `npm run dev` runs) shows the whole app with made-up
agents, for design work without Orca. Add `?builtin` to see the office's own terminals.

## Where agents run

Settings picks where: Automatic (Orca when it's running, then cmux, then Built-in), Orca, cmux, or
Built-in. Room names, order and hidden rooms are kept separately for each, so switching back and forth
loses nothing.

- **Built-in.** The office runs each agent as `claude` in a terminal of its own. Rooms are folders you
  add; a repo's git worktrees are its areas. Opening an agent's terminal shows it in a panel over the
  office, where you can type to Claude directly (Escape goes to Claude, so close the panel with its
  button).
  - **Closing the window hides it**, and agents keep working; click the Dock icon to bring it back.
    **Quitting stops them**, asking first if anyone is mid-task, and each one picks its session back up
    (`claude --resume`) at its own desk the next time you open the office.
  - **A folder Claude hasn't seen before** makes it ask whether to trust the folder. The agent stands
    up to say so, and its task waits until you answer in its terminal.
  - **The usage bar** comes from Claude's own status line. Your status line still shows in the
    terminal. Switching between saved accounts needs Orca, which keeps them.
- **Orca.** Rooms are Orca's repos and folder workspaces.
- **cmux.** Rooms are folders: ones you add, and ones you start Claude in (the repo root, if it's in one).

The status hook (Settings, optional with Orca, needed for cmux) adds one marked command per event to
`~/.claude/settings.json`, after your existing hooks, and backs the file up to
`settings.json.before-agent-office` first. Built-in doesn't need it: its sessions get the same hook with
`--settings`, so nothing is installed. For sessions inside Orca, cmux or the office's own terminals
only, the hook writes the latest event to `~/.agent-office/sessions/<claude pid>.json` (readable only by
you). The app deletes each file once its Claude process has exited. Remove takes out only the marked
commands.

## How it works

- `src-tauri/src/pty.rs` is the office's own terminals: it starts `claude` in a pseudo-terminal,
  keeps a copy of each screen (`vt100`), and streams it to the terminal panel (`src/ui/TerminalPanel.tsx`,
  xterm.js). `orca.rs` talks to Orca's local runtime socket (`orca-runtime.json` gives the path and
  token). `cmux.rs` talks to cmux's socket (`/tmp/cmux.sock`). With Orca or cmux, the terminal app
  creates, prompts and closes the Claude terminals, and this app starts no processes.
- `status.rs` reads Orca's live hook cache (`agent-hooks/last-status.json`) for each pane's state,
  last prompt, current tool and last message. `hooks.rs` installs and reads the office's own hook;
  whichever heard from Claude last wins.
- `brief.rs` parses the `brief` block that agents started here are asked to write (see
  `contract.rs`), with a fallback for sessions started elsewhere.
- `office.rs` builds rooms and agents, and runs Raggy's routing: each request goes to a no-tools
  Haiku session as a roster, and Raggy answers with a `route` block that the app acts on.
- `transcript.rs` reads `~/.claude/projects/*.jsonl` (read-only, path-checked) for "Show work".
- `src/office/` is the canvas office: `layout.ts` (tile map), `art.ts` (all pixel art, drawn in
  code), `world.ts` (movement, pathfinding, drawing), `Office.tsx` (React wrapper).
- `src/ui/` is everything floating over it: the HUD, the dialogs for agents, Raggy and new hires,
  and the toasts.

## Safety

- The webview can call only this app's own commands (`capabilities/default.json`); there is no shell
  plugin, and the CSP blocks remote content.
- Commands sent to Orca or cmux are built from generated values only (session ids, names from a fixed list,
  the contract file path). Your task text is typed into Claude after it starts, never through a shell.
- With Built-in, the app starts `claude` itself, from an argument list, never through a shell. The
  terminal panel can type only into terminals the app started, and a terminal closes when its
  `claude` exits, so there's never a shell prompt to type into. Claude's latest status-line data (plan
  usage, plus the session's folder and cost) is saved to `~/.agent-office/usage.json`, readable only by
  you.
- Links open in the browser only if they are `https://`.

## License and support

MIT, see [LICENSE](LICENSE). The software is provided as is, without warranty of any kind, and I'm
not liable for bugs or for anything it does to your machine or setup. It reads files under
`~/.claude`, edits `~/.claude/settings.json` if you install the status hook, runs Claude Code, and types
into your terminals, so read the code before you run it.

I don't take contributions. Only I can push to this repo, and pull requests are switched off for
everyone else. You can open an issue if you find a bug, but I may not reply or fix it. You're free to
fork it.
