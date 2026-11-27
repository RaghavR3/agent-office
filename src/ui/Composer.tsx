import { Fragment, ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Skill, api } from "../api";

/** Where a message box is aimed: an agent's folder, or a room's for a new hire. */
export interface SkillsFor {
  agentId?: string | null;
  roomId?: string | null;
}

// Skills rarely change, so each folder's list is kept for a minute.
const skillCache = new Map<string, { at: number; list: Promise<Skill[]> }>();
function loadSkills(f: SkillsFor): Promise<Skill[]> {
  const key = `${f.agentId ?? ""}|${f.roomId ?? ""}`;
  const hit = skillCache.get(key);
  if (hit && Date.now() - hit.at < 60_000) return hit.list;
  const list = api.skills(f.agentId ?? null, f.roomId ?? null).catch(() => [] as Skill[]);
  skillCache.set(key, { at: Date.now(), list });
  return list;
}

/** Names that start with what you typed, then names containing it, then descriptions with a word starting with it. */
function rank(skills: Skill[], query: string): Skill[] {
  const q = query.toLowerCase();
  const score = (s: Skill) => {
    const n = s.name.toLowerCase();
    const tail = n.includes(":") ? n.split(":").pop()! : n;
    if (n.startsWith(q) || tail.startsWith(q)) return 0;
    if (n.includes(q)) return 1;
    if (s.description.toLowerCase().split(/[^a-z0-9]+/).some((w) => w.startsWith(q))) return 2;
    return 3;
  };
  return skills
    .map((s) => ({ s, k: score(s) }))
    .filter((x) => x.k < 3)
    .sort((a, b) => a.k - b.k || a.s.name.length - b.s.name.length)
    .map((x) => x.s);
}

const SLASH = /(\/[A-Za-z0-9:_.-]+)/;

/** The text again, with each skill you've named marked, drawn behind the text box. */
function marked(text: string, known: Set<string>): ReactNode[] {
  const parts = text.split(SLASH);
  return parts.map((p, i) => {
    const starts = i === 1 || (i > 0 && /\s$/.test(parts[i - 1]));
    return i % 2 === 1 && starts && known.has(p.slice(1)) ? <mark key={i}>{p}</mark> : <Fragment key={i}>{p}</Fragment>;
  });
}

export interface Attachment {
  path: string;
  name: string;
  preview: string | null;
}

const IMAGE = /^image\/(png|jpe?g|gif|webp)$/;
const fileName = (p: string) => p.split("/").pop() ?? "File";

function toBase64(f: File): Promise<string> {
  return new Promise((res, rej) => {
    const r = new FileReader();
    r.onload = () => res(String(r.result).split(",")[1] ?? "");
    r.onerror = rej;
    r.readAsDataURL(f);
  });
}

/** The message as the agent receives it: your words, then where each attached file is. */
export function withAttachments(text: string, files: Attachment[]): string {
  if (!files.length) return text.trim();
  const lines = files.map((f) => `[Attached file: ${f.path}]`);
  return `${text.trim()}\n\n${lines.join("\n")}`.trim();
}

/** A message box that grows with what you write and takes files by paste, drop or picker. */
export function Composer({
  value,
  onChange,
  files,
  onFiles,
  onSubmit,
  placeholder,
  autoFocus,
  rows = 2,
  primary = true,
  skills: skillsFor,
}: {
  value: string;
  onChange: (v: string) => void;
  files: Attachment[];
  onFiles: (f: Attachment[]) => void;
  onSubmit: () => void;
  placeholder: string;
  autoFocus?: boolean;
  rows?: number;
  /** Takes files dropped anywhere on the window, not just on the box itself. */
  primary?: boolean;
  /** Offers the skills and slash commands available there when you type a slash. */
  skills?: SkillsFor;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const mirror = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLUListElement>(null);
  const caretAfter = useRef<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [over, setOver] = useState(false);
  const [skillList, setSkillList] = useState<Skill[] | null>(null);
  const [menu, setMenu] = useState<{ start: number; query: string } | null>(null);
  const [choice, setChoice] = useState(0);
  const known = new Set((skillList ?? []).map((s) => s.name));
  const matches = menu && skillList ? rank(skillList, menu.query).slice(0, 8) : [];

  const loadList = () => {
    if (skillsFor && !skillList) loadSkills(skillsFor).then(setSkillList);
  };
  // Text that already names a skill gets it highlighted straight away.
  useEffect(() => {
    if (value.includes("/")) loadList();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  /** Opens the list while the word at the caret is a slash command. */
  const follow = (text: string, caret: number) => {
    const m = skillsFor ? text.slice(0, caret).match(/(^|\s)\/([A-Za-z0-9:_.-]*)$/) : null;
    if (!m) return setMenu(null);
    loadList();
    setMenu({ start: caret - m[2].length - 1, query: m[2] });
    setChoice(0);
  };

  const choose = (s: Skill) => {
    const el = ref.current;
    if (!menu || !el) return;
    const caret = el.selectionStart;
    const next = `${value.slice(0, menu.start)}/${s.name} ${value.slice(caret)}`;
    caretAfter.current = menu.start + s.name.length + 2;
    onChange(next);
    setMenu(null);
  };
  useEffect(() => {
    menuRef.current?.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  }, [choice]);
  const named = (skillList ?? []).find((s) => new RegExp(`(^|\\s)/${s.name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(\\s|$)`).test(value));

  // Focus without scrolling, so a long card opens at its top, not at this box.
  useEffect(() => {
    if (autoFocus) ref.current?.focus({ preventScroll: true });
  }, []);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    // After filling in a skill, the caret goes just past it, before the next keypress lands.
    if (caretAfter.current !== null) {
      el.setSelectionRange(caretAfter.current, caretAfter.current);
      caretAfter.current = null;
    }
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight + 2, 260)}px`;
  }, [value]);

  const add = async (list: File[]) => {
    setError(null);
    const out: Attachment[] = [];
    for (const f of list) {
      try {
        const name = f.name || (IMAGE.test(f.type) ? `pasted.${f.type.split("/")[1].replace("jpeg", "jpg")}` : "pasted");
        const path = await api.saveAttachment(name, await toBase64(f));
        out.push({ path, name: fileName(path), preview: IMAGE.test(f.type) ? URL.createObjectURL(f) : null });
      } catch (e) {
        setError(String(e));
      }
    }
    if (out.length) onFiles([...files, ...out]);
  };

  // Files dropped anywhere else in the window land here. Drops on a box are handled (and
  // stopped) by that box, so a card with two boxes never gets the same file twice.
  const addRef = useRef(add);
  addRef.current = add;
  useEffect(() => {
    if (!primary) return;
    let timer: number | undefined;
    const hasFiles = (e: DragEvent) => !!e.dataTransfer && [...e.dataTransfer.types].includes("Files");
    const onOver = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      setOver(true);
      clearTimeout(timer);
      timer = window.setTimeout(() => setOver(false), 200);
    };
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      setOver(false);
      addRef.current([...(e.dataTransfer?.files ?? [])]);
    };
    window.addEventListener("dragover", onOver);
    window.addEventListener("drop", onDrop);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("dragover", onOver);
      window.removeEventListener("drop", onDrop);
    };
  }, [primary]);

  const pick = async () => {
    setError(null);
    const picked = await open({ multiple: true, title: "Attach files" });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    const out: Attachment[] = [];
    for (const p of paths) {
      try {
        out.push({ path: await api.importAttachment(p), name: fileName(p), preview: null });
      } catch (e) {
        setError(String(e));
      }
    }
    if (out.length) onFiles([...files, ...out]);
  };

  return (
    <div
      className={`composer${over ? " over" : ""}`}
      onDragOver={(e) => {
        if ([...e.dataTransfer.items].some((i) => i.kind === "file")) {
          e.preventDefault();
          setOver(true);
        }
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        e.stopPropagation();
        setOver(false);
        add([...e.dataTransfer.files]);
      }}
    >
      <div className="composer-field">
        <div className="composer-mirror" ref={mirror} aria-hidden="true">
          {marked(value, known)}
          {"\n"}
        </div>
        <textarea
          ref={ref}
          rows={rows}
          value={value}
          placeholder={placeholder}
          onChange={(e) => {
            onChange(e.target.value);
            follow(e.target.value, e.target.selectionStart);
          }}
          onClick={(e) => follow(e.currentTarget.value, e.currentTarget.selectionStart)}
          onBlur={() => setTimeout(() => setMenu(null), 150)}
          onScroll={(e) => {
            if (mirror.current) mirror.current.scrollTop = e.currentTarget.scrollTop;
          }}
          onPaste={(e) => {
            const pasted = [...e.clipboardData.files];
            if (pasted.length) {
              e.preventDefault();
              add(pasted);
            }
          }}
          onKeyDown={(e) => {
            if (menu && matches.length) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                const step = e.key === "ArrowDown" ? 1 : -1;
                setChoice((c) => (c + step + matches.length) % matches.length);
                return;
              }
              if (e.key === "Enter" || e.key === "Tab") {
                e.preventDefault();
                choose(matches[Math.min(choice, matches.length - 1)]);
                return;
              }
            }
            if (menu && e.key === "Escape") {
              e.preventDefault();
              setMenu(null);
              return;
            }
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              // A held key or an input-method confirm isn't a second send.
              if (e.repeat || e.nativeEvent.isComposing) return;
              onSubmit();
            }
          }}
        />
      </div>
      {menu && matches.length > 0 && (
        <ul className="slash-menu" ref={menuRef} role="listbox" aria-label="Skills and commands">
          {matches.map((s, i) => (
            <li key={s.name} className={i === choice ? "on" : ""} role="option" aria-selected={i === choice}>
              <button type="button" onMouseDown={(e) => e.preventDefault()} onClick={() => choose(s)} onMouseEnter={() => setChoice(i)}>
                <b>/{s.name}</b>
                <span>{s.description}</span>
                <em>{s.source === "you" ? "yours" : s.source}</em>
              </button>
            </li>
          ))}
        </ul>
      )}
      {named && !menu && (
        <p className="skill-note">
          <b>/{named.name}</b> {named.description}
        </p>
      )}
      <div className="composer-bar">
        {files.map((f) => (
          <span className="attachment" key={f.path} title={f.path}>
            {f.preview ? <img src={f.preview} alt="" /> : <span className="file-icon" aria-hidden="true" />}
            <span>{f.name}</span>
            <button type="button" aria-label={`Remove ${f.name}`} onClick={() => onFiles(files.filter((x) => x.path !== f.path))}>
              ×
            </button>
          </span>
        ))}
        <button type="button" className="attach" onClick={pick} title="Attach files (or paste or drop them in)" aria-label="Attach files">
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <path d="M10.6 4.6 5.4 9.8a1.6 1.6 0 0 0 2.3 2.3l5.4-5.4a3 3 0 0 0-4.3-4.3L3.4 7.8a4.4 4.4 0 0 0 6.2 6.2l4.1-4.1" />
          </svg>
        </button>
      </div>
      {error && <p className="error">{error}</p>}
    </div>
  );
}
