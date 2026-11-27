// The walkable pixel office: a 2D canvas world built from the live snapshot. It redraws every
// frame only while something moves, and drops to a slow tick when the office is quiet.

import { type Agent, type Room, areaColor } from "../api";
import { Layout, PITCH, RoomBox, layoutOffice } from "./layout";
import { NavGrid } from "./nav";
import { Dir, Frames, SCREEN, TILE, YOU, chairSprite, deskSprite, floorLayer, lookForAgent, outlined, personFrames, propSprite } from "./art";

export type Focus =
  | { kind: "agent"; id: string; name: string }
  | { kind: "cos"; name: string }
  | { kind: "hire"; roomId: string; roomName: string; slot: number }
  | { kind: "addroom" }
  | null;

export interface WorldEvents {
  onFocus(f: Focus): void;
  onOpen(agentId: string): void;
  onCos(): void;
  onHire(roomId: string, roomName: string, slot: number): void;
  onAddRoom(): void;
}

type Pose = "stand" | "sit" | "wave";

class Walker {
  x = 0;
  y = 0;
  dir: Dir = "down";
  path: { x: number; y: number }[] = [];
  onArrive: (() => void) | null = null;
  t = Math.random() * 10;
  home = { x: 0, y: 0, dir: "down" as Dir, pose: "stand" as Pose };
  constructor(
    public frames: Frames,
    public speed = 3.2,
  ) {}

  get walking() {
    return this.path.length > 0;
  }

  walkTo(path: { x: number; y: number }[], done?: () => void) {
    this.path = path.slice();
    this.onArrive = done ?? null;
  }

  step(dt: number): boolean {
    if (!this.path.length) return false;
    const p = this.path[0];
    const dx = p.x - this.x;
    const dy = p.y - this.y;
    const d = Math.hypot(dx, dy);
    const m = this.speed * dt;
    if (Math.abs(dx) > 0.01 || Math.abs(dy) > 0.01) this.dir = Math.abs(dx) > Math.abs(dy) ? (dx > 0 ? "right" : "left") : dy > 0 ? "down" : "up";
    if (d <= m) {
      this.x = p.x;
      this.y = p.y;
      this.path.shift();
      if (!this.path.length) {
        const cb = this.onArrive;
        this.onArrive = null;
        cb?.();
      }
    } else {
      this.x += (dx / d) * m;
      this.y += (dy / d) * m;
    }
    return true;
  }

  image(moving: boolean): HTMLCanvasElement {
    if (moving) return this.frames[`${this.dir}-${[1, 0, 2, 0][Math.floor(this.t * 8) % 4]}`];
    const pose = this.home.pose;
    if (pose === "sit") return this.frames[`sit-${Math.floor(this.t * 7) % 2}`];
    if (pose === "wave") return this.frames[`wave-${Math.floor(this.t * 3) % 2}`];
    return this.frames[`${this.dir}-0`];
  }
}

interface Resident {
  agent: Agent;
  w: Walker;
  roomId: string;
  slot: number;
  leaving: boolean;
}

// Zoom levels in device pixels per art pixel, so pixel art always lands on whole pixels.
const LEVELS = [2, 3, 4, 5, 6, 8];
const DESK = outlined(deskSprite());
const CHAIR = outlined(chairSprite());
const FONT = "'Archivo Variable', -apple-system, sans-serif";

const helperFrames = new Map<string, Frames>();
function helperLook(agent: string, i: number): Frames {
  const k = `${agent}:${i}`;
  if (!helperFrames.has(k)) helperFrames.set(k, personFrames(lookForAgent({ name: `${agent} helper ${i}`, role: "worker" })));
  return helperFrames.get(k)!;
}

// Where a desk's helpers stand: beside the desk, then behind the chair.
const HELPER_SPOTS = [
  { dx: 2.45, dy: 1.15 },
  { dx: -0.45, dy: 1.15 },
  { dx: 2.45, dy: 2.35 },
];

const isTyping = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.tagName === "TEXTAREA" || t.tagName === "INPUT" || t.isContentEditable);

export class World {
  private cv = document.createElement("canvas");
  private g = this.cv.getContext("2d")!;
  private layout: Layout | null = null;
  private floor: HTMLCanvasElement | null = null;
  private nav: NavGrid | null = null;
  private residents = new Map<string, Resident>();
  private slotOf = new Map<string, { roomId: string; slot: number }>();
  private player = new Walker(personFrames(YOU()), 7);
  private keys = new Set<string>();
  private zoom = 3;
  private cosState: "idle" | "busy" | "asking" | "brief" = "idle";
  private cam = { x: 0, y: 0 };
  private focus: Focus = null;
  private focusKey = "";
  private selected: string | null = null;
  private cosId: string | null = null;
  private first = true;
  private raf = 0;
  private last = performance.now();
  private lastDraw = 0;
  private dirty = true;
  private cssW = 1;
  private cssH = 1;
  private dpr = 0;
  private disposers: (() => void)[] = [];
  private props: { y: number; img: HTMLCanvasElement; x: number; top: number }[] = [];
  private waitingRooms = new Set<string>();
  private dialogOpen = false;
  private hold: { x: number; y: number } | null = null;
  /** How far you've scrolled the view away from yourself, in tiles. Walking snaps it back. */
  private pan = { x: 0, y: 0 };
  private calm = matchMedia("(prefers-reduced-motion: reduce)").matches;

  constructor(
    private host: HTMLElement,
    private ev: WorldEvents,
  ) {
    host.appendChild(this.cv);
    this.listen(window, "keydown", (e: KeyboardEvent) => {
      if (isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      const k = e.key.toLowerCase();
      if (["w", "a", "s", "d", "arrowup", "arrowdown", "arrowleft", "arrowright", "shift"].includes(k)) {
        this.keys.add(k);
        this.player.path = [];
        this.dirty = true;
        if (k.startsWith("arrow")) e.preventDefault();
      }
      if (k === "e") this.interact();
      if (k === "=" || k === "+") this.setZoom(1);
      if (k === "-") this.setZoom(-1);
    });
    this.listen(window, "keyup", (e: KeyboardEvent) => this.keys.delete(e.key.toLowerCase()));
    this.listen(window, "blur", () => this.keys.clear());
    // Scroll looks around; pinch (which arrives as ctrl+wheel) zooms.
    let pinch = 0;
    this.listen(this.cv, "wheel", (e: WheelEvent) => {
      e.preventDefault();
      if (e.ctrlKey) {
        pinch += e.deltaY;
        if (Math.abs(pinch) > 24) {
          this.setZoom(pinch > 0 ? -1 : 1);
          pinch = 0;
        }
        return;
      }
      const per = (LEVELS[this.zoom] / this.dpr) * TILE;
      const p = this.hold ?? this.pan;
      p.x += e.deltaX / per;
      p.y += e.deltaY / per;
      this.dirty = true;
    });
    this.listen(this.cv, "click", (e: MouseEvent) => this.click(e));
    const ro = new ResizeObserver(() => this.resize());
    ro.observe(host);
    this.disposers.push(() => ro.disconnect());
    this.resize();
    document.fonts?.ready.then(() => (this.dirty = true));
    this.raf = requestAnimationFrame(this.tick);
  }

  private listen(t: EventTarget, type: string, fn: (e: any) => void) {
    const opts = type === "wheel" ? { passive: false } : undefined;
    t.addEventListener(type, fn, opts);
    this.disposers.push(() => t.removeEventListener(type, fn));
  }

  /** A card sits over the office without moving it; scrolling still looks around underneath. */
  setDialogOpen(open: boolean) {
    if (open !== this.dialogOpen) this.dirty = true;
    if (open && !this.hold) this.hold = { ...this.cam };
    if (!open && this.hold) {
      this.pan.x = this.hold.x - this.player.x;
      this.pan.y = this.hold.y - this.player.y;
      this.hold = null;
    }
    this.dialogOpen = open;
  }


  zoomBy(d: number) {
    this.setZoom(d);
  }

  private setZoom(d: number) {
    this.zoom = Math.max(0, Math.min(LEVELS.length - 1, this.zoom + d));
    this.dirty = true;
  }

  private resize() {
    const dpr = Math.min(devicePixelRatio || 1, 2);
    if (dpr !== this.dpr) this.zoom = LEVELS.indexOf(dpr >= 2 ? 4 : 2);
    this.dpr = dpr;
    this.cssW = this.host.clientWidth;
    this.cssH = this.host.clientHeight;
    this.cv.width = Math.round(this.cssW * this.dpr);
    this.cv.height = Math.round(this.cssH * this.dpr);
    this.cv.style.width = `${this.cssW}px`;
    this.cv.style.height = `${this.cssH}px`;
    this.dirty = true;
  }

  dispose() {
    cancelAnimationFrame(this.raf);
    this.disposers.forEach((d) => d());
    this.cv.remove();
  }

  // ---- data -----------------------------------------------------------------------------

  setData(rooms: Room[], agents: Agent[], cosName: string, selected: string | null, cosState: World["cosState"] = "idle") {
    this.cosState = cosState;
    // Sorted by area so people in the same area sit together.
    const workers = agents
      .filter((a) => a.role === "worker" && rooms.some((r) => r.id === a.roomId))
      .sort((a, b) => (a.area ?? "").localeCompare(b.area ?? "") || a.name.localeCompare(b.name));
    const counts = new Map<string, number>();
    for (const a of workers) counts.set(a.roomId, (counts.get(a.roomId) ?? 0) + 1);
    const layout = layoutOffice(rooms, counts);
    if (layout.key !== this.layout?.key) this.rebuild(layout);

    const used = new Map<string, Set<number>>();
    for (const a of workers) {
      const s = this.slotOf.get(a.id);
      const box = this.box(a.roomId);
      if (s && s.roomId === a.roomId && box && s.slot < box.slots.length) used.set(a.roomId, (used.get(a.roomId) ?? new Set()).add(s.slot));
      else this.slotOf.delete(a.id);
    }
    // Anyone hired to a particular desk sits there; everyone else takes the next free one.
    const byDesk = [...workers].sort((a, b) => (b.desk != null ? 1 : 0) - (a.desk != null ? 1 : 0));
    for (const a of byDesk) {
      if (this.slotOf.has(a.id)) continue;
      // A pending hire turns into a real terminal with the same name: same desk, same body.
      const pending = [...this.residents.values()].find(
        (r) => r.agent.id.startsWith("pending-") && r.agent.name === a.name && r.roomId === a.roomId,
      );
      const taken = used.get(a.roomId) ?? new Set<number>();
      const desks = this.box(a.roomId)?.slots.length ?? 0;
      const wanted = a.desk != null && a.desk < desks && !taken.has(a.desk) ? a.desk : null;
      let slot = pending ? pending.slot : (wanted ?? 0);
      while (!pending && wanted == null && taken.has(slot)) slot++;
      taken.add(slot);
      used.set(a.roomId, taken);
      this.slotOf.set(a.id, { roomId: a.roomId, slot });
      if (pending) {
        // Same object, new key: anything holding a reference (like a walk callback) stays valid.
        this.slotOf.delete(pending.agent.id);
        this.residents.delete(pending.agent.id);
        pending.agent = a;
        pending.leaving = false;
        this.residents.set(a.id, pending);
      }
    }

    const seen = new Set<string>();
    for (const a of workers) {
      seen.add(a.id);
      this.upsert(a, this.slotOf.get(a.id)!.slot);
    }
    const cos = agents.find((a) => a.role === "cos");
    this.cosId = cos?.id ?? null;
    const cosAgent = {
      ...(cos ?? {
        name: cosName,
        roomId: "hq",
        role: "cos",
        managed: true,
        title: "",
        state: "idle",
        brief: null,
        activity: null,
        prompt: null,
        updatedAt: 0,
        unread: false,
        worktree: null,
      }),
      id: "cos",
    } as Agent;
    seen.add("cos");
    this.upsert(cosAgent, 0);
    for (const [id, r] of this.residents) if (!seen.has(id) && !r.leaving) this.leave(r);

    this.selected = selected && selected === this.cosId ? "cos" : selected;
    this.waitingRooms = new Set(workers.filter((a) => a.state === "needs_you" || a.state === "waiting").map((a) => a.roomId));
    this.first = false;
    this.dirty = true;
  }

  goTo(agentId: string) {
    const r = this.residents.get(agentId === this.cosId ? "cos" : agentId);
    if (!r) return;
    const s = this.approach(r);
    this.player.path = [];
    this.pan = { x: 0, y: 0 };
    this.player.x = s.x;
    this.player.y = s.y;
    this.player.dir = s.y > r.w.y ? "up" : "down";
    this.dirty = true;
  }

  private box(id: string): RoomBox | undefined {
    return this.layout?.rooms.find((r) => r.room.id === id);
  }

  private rebuild(l: Layout) {
    this.layout = l;
    this.floor = floorLayer(l);
    const nav = new NavGrid(l.ox, 0, l.ox + l.w, l.h);
    this.nav = nav;
    // Walls and anything you can't walk through.
    for (let j = 0; j < l.h; j++)
      for (let i = 0; i < l.w; i++) {
        const t = l.tiles[j * l.w + i];
        if (t === 1 || t === 0) nav.block(i + l.ox, j, i + l.ox + 0.999, j + 0.999, 0.28);
      }
    this.props = [];
    for (const p of l.props) {
      const s = propSprite(p.kind);
      if (p.blocks) nav.block(p.x, p.y, p.x + p.w - 0.001, p.y + p.h - 0.001, 0.25);
      if (!s) continue;
      const bottom = (p.y + p.h) * TILE;
      this.props.push({ y: p.y + p.h, img: s.img, x: (p.x - l.ox) * TILE - 1, top: bottom - s.img.height + 1 });
    }
    for (const rb of l.rooms)
      for (const s of rb.slots) {
        nav.block(s.x, s.y, s.x + 1.999, s.y + 0.999, 0.25);
        nav.block(s.x + 0.6, s.y + 1, s.x + 1.4, s.y + 1.9, 0.12);
      }
    for (const r of this.residents.values()) this.settle(r, true);
    if (this.first || !nav.walkable(this.player.x, this.player.y)) {
      this.player.x = l.spawn.x;
      this.player.y = l.spawn.y;
      this.player.dir = "up";
      this.cam = { x: this.player.x, y: this.player.y };
    }
  }

  // ---- residents --------------------------------------------------------------------------

  private homeFor(a: Agent, slot: number): Walker["home"] {
    if (a.role === "cos" && this.layout) return { x: this.layout.cos.x, y: this.layout.cos.y, dir: "down", pose: "stand" };
    const s = this.box(a.roomId)?.slots[slot];
    if (!s) return { x: 0, y: 0, dir: "down", pose: "stand" };
    if (a.state === "needs_you" || a.state === "waiting") return { x: s.x + 2.5, y: s.y + 1.7, dir: "down", pose: "wave" };
    return { x: s.x + 1, y: s.y + 1.75, dir: "up", pose: "sit" };
  }

  private upsert(a: Agent, slot: number) {
    let r = this.residents.get(a.id);
    if (!r) {
      const w = new Walker(personFrames(lookForAgent(a)));
      r = { agent: a, w, roomId: a.roomId, slot, leaving: false };
      this.residents.set(a.id, r);
      // Everyone simply appears at their desk: new hires included, no walking in.
      w.home = this.homeFor(a, slot);
      this.settle(r, true);
    }
    r.agent = a;
    r.slot = slot;
    r.roomId = a.roomId;
    const home = this.homeFor(a, slot);
    const moved = Math.hypot(home.x - r.w.home.x, home.y - r.w.home.y) > 0.05;
    r.w.home = home;
    if (moved) this.settle(r, true);
  }

  private settle(r: Resident, snap: boolean) {
    if (r.leaving) return;
    if (snap) {
      r.w.path = [];
      r.w.x = r.w.home.x;
      r.w.y = r.w.home.y;
    }
    r.w.dir = r.w.home.dir;
  }

  private leave(r: Resident) {
    r.leaving = true;
    // Only remove this exact resident; its key may have been taken over meanwhile.
    if (this.residents.get(r.agent.id) === r) this.residents.delete(r.agent.id);
    this.slotOf.delete(r.agent.id);
    this.dirty = true;
  }

  // ---- interaction ------------------------------------------------------------------------

  /** Where to stand to talk to someone: beside a seated agent, in front of anyone else. */
  private approach(r: Resident) {
    const { x, y } = r.w;
    const seated = r.w.home.pose === "sit" && !r.w.walking;
    // From behind a seated agent's chair, so the spots beside the desk stay free for their helpers.
    const cand = seated
      ? [
          { x, y: y + 0.9 },
          { x: x + 1.4, y },
          { x: x - 1.4, y },
        ]
      : [
          { x, y: y + 1.2 },
          { x: x + 0.9, y: y + 0.5 },
          { x: x - 0.9, y: y + 0.5 },
          { x: x + 1, y },
          { x: x - 1, y },
          { x, y: y - 1 },
        ];
    return cand.find((c) => this.nav?.walkable(c.x, c.y)) ?? { x, y: y + 1.2 };
  }

  private interact() {
    const f = this.focus;
    if (!f) return;
    if (f.kind === "agent") this.ev.onOpen(f.id);
    else if (f.kind === "cos") this.ev.onCos();
    else if (f.kind === "hire") this.ev.onHire(f.roomId, f.roomName, f.slot);
    else this.ev.onAddRoom();
  }

  private toWorld(cx: number, cy: number) {
    const s = (LEVELS[this.zoom] / this.dpr) * TILE;
    return { x: (cx - this.cssW / 2) / s + this.cam.x, y: (cy - this.cssH / 2) / s + this.cam.y };
  }

  private click(e: MouseEvent) {
    if (!this.nav) return;
    const rect = this.cv.getBoundingClientRect();
    const p = this.toWorld(e.clientX - rect.left, e.clientY - rect.top);
    const hit = [...this.residents.values()]
      .filter((r) => !r.leaving && Math.abs(p.x - r.w.x) < 0.6 && p.y > r.w.y - 1.7 && p.y < r.w.y + 0.2)
      .sort((a, b) => b.w.y - a.w.y)[0];
    if (hit) {
      if (hit.agent.role === "cos") this.ev.onCos();
      else this.ev.onOpen(hit.agent.id);
      const s = this.approach(hit);
      const path = this.nav.path(this.player.x, this.player.y, s.x, s.y);
      if (path) {
        this.player.walkTo(path, () => {
          this.player.dir = hit.w.y < this.player.y ? "up" : "down";
        });
        this.pan = { x: 0, y: 0 };
        this.dirty = true;
      }
      return;
    }
    // A free desk: hire someone to sit at exactly this one.
    const desk = this.freeDeskAt(p.x, p.y);
    if (desk) {
      this.ev.onHire(desk.roomId, desk.roomName, desk.slot);
      const path = this.nav.path(this.player.x, this.player.y, desk.x + 1, desk.y + 2.2);
      if (path) {
        this.player.walkTo(path);
        this.pan = { x: 0, y: 0 };
      }
      this.dirty = true;
      return;
    }
    if (this.nav.walkable(p.x, p.y)) {
      const path = this.nav.path(this.player.x, this.player.y, p.x, p.y);
      if (path) {
        this.player.walkTo(path);
        this.pan = { x: 0, y: 0 };
      }
      this.dirty = true;
    }
  }

  /** The free desk (desk, chair or the floor just behind it) at a point, if there is one. */
  private freeDeskAt(x: number, y: number) {
    if (!this.layout) return null;
    for (const rb of this.layout.rooms) {
      const taken = new Set([...this.residents.values()].filter((r) => r.roomId === rb.room.id && !r.leaving).map((r) => r.slot));
      for (let i = 0; i < rb.slots.length; i++) {
        const s = rb.slots[i];
        if (!taken.has(i) && x >= s.x - 0.2 && x <= s.x + 2.2 && y >= s.y - 0.9 && y <= s.y + 2) {
          return { roomId: rb.room.id, roomName: rb.room.name, slot: i, x: s.x, y: s.y };
        }
      }
    }
    return null;
  }

  private updateFocus() {
    let best: Focus = null;
    let bestD = 1.6;
    for (const r of this.residents.values()) {
      if (r.leaving) continue;
      const d = Math.hypot(r.w.x - this.player.x, r.w.y - this.player.y) - (r.agent.role === "cos" ? 0.5 : 0);
      if (d < bestD) {
        bestD = d;
        best = r.agent.role === "cos" ? { kind: "cos", name: r.agent.name } : { kind: "agent", id: r.agent.id, name: r.agent.name };
      }
    }
    if (!best && this.layout) {
      for (const rb of this.layout.rooms) {
        const taken = new Set([...this.residents.values()].filter((r) => r.roomId === rb.room.id && !r.leaving).map((r) => r.slot));
        rb.slots.forEach((s, i) => {
          if (taken.has(i)) return;
          const d = Math.hypot(s.x + 1 - this.player.x, s.y + 2.2 - this.player.y);
          if (d < Math.min(bestD, 1.1)) {
            bestD = d;
            best = { kind: "hire", roomId: rb.room.id, roomName: rb.room.name, slot: i };
          }
        });
      }
    }
    const vacant = this.layout?.rooms.find((rb) => rb.vacant);
    if (!best && vacant) {
      const { x0, x1, y0, y1 } = vacant;
      const p = this.player;
      if (p.x > x0 - 0.6 && p.x < x1 + 0.6 && p.y > y0 - 0.6 && p.y < y1 + 0.6) best = { kind: "addroom" };
    }
    const key = best ? JSON.stringify(best) : "";
    if (key !== this.focusKey) {
      this.focusKey = key;
      this.focus = best;
      this.ev.onFocus(best);
      this.dirty = true;
    }
  }

  private movePlayer(dt: number): boolean {
    if (this.player.step(dt)) return true;
    const k = this.keys;
    let dx = 0;
    let dy = 0;
    if (k.has("w") || k.has("arrowup")) dy -= 1;
    if (k.has("s") || k.has("arrowdown")) dy += 1;
    if (k.has("a") || k.has("arrowleft")) dx -= 1;
    if (k.has("d") || k.has("arrowright")) dx += 1;
    if (!dx && !dy) return false;
    this.pan = { x: 0, y: 0 };
    const len = Math.hypot(dx, dy);
    const v = (k.has("shift") ? 10 : 7) * dt;
    const nx = this.player.x + (dx / len) * v;
    const ny = this.player.y + (dy / len) * v;
    const nav = this.nav;
    if (!nav || nav.walkable(nx, ny)) {
      this.player.x = nx;
      this.player.y = ny;
    } else if (nav.walkable(nx, this.player.y)) this.player.x = nx;
    else if (nav.walkable(this.player.x, ny)) this.player.y = ny;
    this.player.dir = Math.abs(dx) > Math.abs(dy) ? (dx > 0 ? "right" : "left") : dy > 0 ? "down" : "up";
    return true;
  }

  // ---- loop -------------------------------------------------------------------------------

  private tick = (now: number) => {
    this.raf = requestAnimationFrame(this.tick);
    const dt = Math.min(0.05, (now - this.last) / 1000);
    this.last = now;
    const moving = this.movePlayer(dt);
    let walkers = false;
    let animated = false;
    for (const r of this.residents.values()) {
      if (r.w.step(dt)) walkers = true;
      if (r.w.home.pose !== "stand" || r.agent.helpers?.length) animated = true;
      r.w.t += dt;
    }
    this.player.t += dt;
    const target = this.camTarget();
    const settling = Math.hypot(this.cam.x - target.x, this.cam.y - target.y) > 0.01;
    const active = moving || walkers || settling || this.dirty;
    const interval = active ? 0 : animated ? 1000 / 12 : 1000 / 4;
    if (now - this.lastDraw < interval) return;
    const step = Math.min(0.1, (now - this.lastDraw) / 1000);
    this.lastDraw = now;
    this.dirty = false;
    const k = 1 - Math.exp(-step * 7);
    this.cam.x += (target.x - this.cam.x) * k;
    this.cam.y += (target.y - this.cam.y) * k;
    this.updateFocus();
    this.draw(now, moving || this.player.walking);
  };

  /** Follow the player, but keep the office filling the view instead of showing empty space. */
  private camTarget() {
    const l = this.layout;
    if (!l) return { x: this.player.x, y: this.player.y };
    const per = (LEVELS[this.zoom] / this.dpr) * TILE;
    const hw = this.cssW / 2 / per;
    const hh = this.cssH / 2 / per;
    // A little margin around the building, plus room for the top bar.
    const pad = 1.5;
    const top = 40 / per;
    const fit = (v: number, min: number, max: number, half: number) =>
      max - min <= half * 2 ? (min + max) / 2 : Math.min(max - half, Math.max(min + half, v));
    // With a card open the office holds still under it instead of following you.
    if (this.hold) {
      this.hold.x = fit(this.hold.x, l.ox - pad, l.ox + l.w + pad, hw);
      this.hold.y = fit(this.hold.y, -pad - top, l.h + pad, hh);
      return { ...this.hold };
    }
    const x = fit(this.player.x + this.pan.x, l.ox - pad, l.ox + l.w + pad, hw);
    const y = fit(this.player.y + this.pan.y, -pad - top, l.h + pad, hh);
    // Keep the pan inside what the camera can actually show, so scrolling back responds at once.
    this.pan.x = x - this.player.x;
    this.pan.y = y - this.player.y;
    return { x, y };
  }

  private draw(now: number, playerMoving: boolean) {
    const l = this.layout;
    const g = this.g;
    g.setTransform(1, 0, 0, 1, 0, 0);
    g.fillStyle = "#2b2d38";
    g.fillRect(0, 0, this.cv.width, this.cv.height);
    if (!l || !this.floor) return;

    const s = LEVELS[this.zoom];
    const z = s / this.dpr;
    // Snap the camera to whole art pixels so nothing shimmers.
    const camX = Math.round((this.cam.x - l.ox) * TILE * s) / s;
    const camY = Math.round(this.cam.y * TILE * s) / s;
    const tx = Math.round(this.cv.width / 2 - camX * s);
    const ty = Math.round(this.cv.height / 2 - camY * s);
    g.setTransform(s, 0, 0, s, tx, ty);
    g.imageSmoothingEnabled = false;
    g.drawImage(this.floor, 0, 0);

    const wx = (x: number) => (x - l.ox) * TILE;
    type D = { y: number; f: () => void };
    const list: D[] = [];
    for (const p of this.props) list.push({ y: p.y, f: () => g.drawImage(p.img, p.x, p.top) });

    const occupant = new Map<string, Resident>();
    for (const r of this.residents.values()) if (!r.leaving && r.agent.role === "worker") occupant.set(`${r.roomId}:${r.slot}`, r);
    for (const rb of l.rooms)
      rb.slots.forEach((sl, i) => {
        const r = occupant.get(`${rb.room.id}:${i}`);
        const dx = wx(sl.x) - 1;
        const dy = (sl.y + 1) * TILE - DESK.img.height + 1;
        list.push({
          y: sl.y + 1,
          f: () => {
            g.drawImage(DESK.img, dx, dy);
            this.screen(g, dx + 1 + SCREEN.x, dy + 1 + SCREEN.y, r?.agent, now);
            if (r?.agent.work) this.papers(g, dx + 1, dy + 1, r.agent.work);
            if (r?.agent.state === "done") {
              g.fillStyle = "#f4f1ea";
              g.fillRect(dx + 27, dy + 14, 3, 3);
              g.fillRect(dx + 30, dy + 15, 1, 1);
            }
          },
        });
        const cx = wx(sl.x) + 8 - 1;
        const cy = (sl.y + 2) * TILE - CHAIR.img.height + 1;
        // Seated people are drawn over their chair so you can see who's at the desk.
        list.push({ y: r && !r.w.walking && r.w.home.pose === "sit" ? sl.y + 1.5 : sl.y + 2.01, f: () => g.drawImage(CHAIR.img, cx, cy) });
      });

    const people: { r: Resident | null; w: Walker }[] = [{ r: null, w: this.player }];
    for (const r of this.residents.values()) people.push({ r, w: r.w });
    for (const r of this.residents.values()) {
      const helpers = r.agent.helpers ?? [];
      const slot = this.box(r.roomId)?.slots[r.slot];
      if (!helpers.length || !slot || r.w.walking || r.leaving) continue;
      helpers.slice(0, HELPER_SPOTS.length).forEach((_, i) => {
        const spot = HELPER_SPOTS[i];
        const hx = slot.x + spot.dx;
        const hy = slot.y + spot.dy;
        const frames = helperLook(r.agent.name, i);
        const face = hx > r.w.x ? "left" : "right";
        list.push({
          y: hy,
          f: () => {
            const px = Math.round(wx(hx));
            const py = Math.round(hy * TILE);
            const bob = this.calm ? 0 : Math.round(Math.sin(now / 160 + i * 2) * 0.6);
            g.fillStyle = "rgba(40,25,15,0.22)";
            g.fillRect(px - 3, py - 1, 6, 2);
            const img = frames[`${face}-0`];
            g.drawImage(img, px - 7.5, py - 19.5 + bob, 15, 21);
          },
        });
      });
    }
    for (const p of people) {
      const moving = p.r ? p.w.walking : playerMoving;
      list.push({
        y: p.w.y,
        f: () => {
          const px = Math.round(wx(p.w.x));
          const py = Math.round(p.w.y * TILE);
          g.fillStyle = "rgba(40,25,15,0.25)";
          g.fillRect(px - 4, py - 1, 8, 2);
          g.fillRect(px - 3, py + 1, 6, 1);
          if (!p.r || (this.selected && p.r.agent.id === this.selected)) {
            g.fillStyle = p.r ? "rgba(43,109,147,0.55)" : "rgba(255,255,255,0.55)";
            g.fillRect(px - 6, py, 12, 1);
          }
          const img = p.w.image(moving);
          g.drawImage(img, px - 10, py - 26);
        },
      });
    }
    list.sort((a, b) => a.y - b.y);
    for (const d of list) d.f();

    // Screen-space labels on top, at full resolution.
    g.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    const sx = (x: number) => (wx(x) * s + tx) / this.dpr;
    const sy = (y: number) => (y * TILE * s + ty) / this.dpr;
    // Each room's name hangs on the wall beside its door, where you walk past it.
    for (const rb of l.rooms)
      this.plaque(g, sx(rb.sign.x), sy(rb.sign.y), rb.vacant ? "+ Add a room" : rb.room.name, this.waitingRooms.has(rb.room.id), rb.vacant, rb.sign.align);
    for (const p of people) {
      const x = sx(p.w.x);
      const feet = sy(p.w.y);
      const head = feet - 25 * z;
      if (!p.r) {
        this.nameTag(g, x, feet + 3, "You", "rgba(28,26,36,0.72)");
        continue;
      }
      const a = p.r.agent;
      const needs = a.state === "needs_you" || a.state === "waiting";
      const isFocus = this.focus && ((this.focus.kind === "agent" && this.focus.id === a.id) || (this.focus.kind === "cos" && a.role === "cos"));
      const cos = a.role === "cos";
      const flag = needs || (cos && this.cosState === "asking");
      this.nameTag(g, x, feet + 3, a.name, flag ? "#bf3a2c" : cos ? "#2f7350" : "rgba(28,26,36,0.72)", a.area);
      // What they're on, so you can tell your agents apart at a glance.
      const helping = a.helpers?.length ?? 0;
      const caption = cos
        ? "Chief of staff"
        : a.state === "working" && helping
          ? `Running ${helping} ${helping === 1 ? "helper" : "helpers"}`
          : a.state === "working" && a.activity
            ? a.activity
            : a.task;
      // Keep each caption inside its own desk's width.
      if (caption && z >= 1.5) this.caption(g, x, feet + 21, caption, Math.min(170, PITCH * TILE * z - 30));
      const talking = this.dialogOpen && this.selected === a.id;
      if (isFocus && !talking && a.brief?.headline && !cos) this.speech(g, x, head - 4, a.brief.headline, needs);
      else {
        const mark = cos
          ? { idle: "", busy: "dots", asking: "?", brief: "!" }[this.cosState]
          : needs
            ? a.state === "waiting"
              ? "!"
              : "?"
            : a.state === "working"
              ? "dots"
              : a.state === "done" && a.unread
                ? "✓"
                : "";
        if (mark) this.badge(g, x, head - 4, mark, now);
      }
    }
  }

  /** Paper piling up on a desk as its agent works: a sheet every few tool calls, in short stacks
   * either side of the monitor, so a busy desk never grows into the one behind it. */
  private papers(g: CanvasRenderingContext2D, x: number, y: number, work: number) {
    const PER_SHEET = 6;
    const STACK = 6;
    // Back stacks first, so the front ones are drawn over them.
    const spots = [
      { sx: 25, base: 20 },
      { sx: 1, base: 20 },
      { sx: 26, base: 15 },
      { sx: 2, base: 15 },
    ];
    const sheets = Math.min(spots.length * STACK, Math.floor(work / PER_SHEET));
    const stacks = spots.slice(0, Math.ceil(sheets / STACK)).map((spot, i) => ({ ...spot, n: Math.min(STACK, sheets - i * STACK) }));
    for (const { sx, base, n } of [...stacks].reverse()) {
      const left = x + sx;
      const top = y + base - n - 3;
      g.fillStyle = "rgba(42,36,51,0.55)";
      g.fillRect(left - 1, top - 1, 9, n + 5);
      for (let k = 0; k < n; k++) {
        // Edges of the sheets, slightly out of line like a real pile.
        const nudge = k % 3 === 1 ? 1 : 0;
        g.fillStyle = k % 2 ? "#e6e0d4" : "#f3efe6";
        g.fillRect(left + nudge, y + base - k, 7, 1);
      }
      g.fillStyle = "#fbfaf6";
      g.fillRect(left, top, 7, 3);
      g.fillStyle = "#cfc8bb";
      g.fillRect(left + 1, top + 1, 4, 1);
    }
  }

  private screen(g: CanvasRenderingContext2D, x: number, y: number, a: Agent | undefined, now: number) {
    const { w, h } = SCREEN;
    const st = a?.state;
    if (st === "working") {
      g.fillStyle = "#1f3b52";
      g.fillRect(x, y, w, h);
      const off = Math.floor(now / 260);
      for (let row = 0; row < 4; row++) {
        const n = (row + off) * 7919;
        g.fillStyle = row % 2 ? "#58b4e6" : "#a7dcf7";
        g.fillRect(x + 1 + (n % 3), y + 1 + row * 2, 3 + (n % 7), 1);
      }
    } else if (st === "needs_you" || st === "waiting") {
      g.fillStyle = Math.floor(now / 600) % 2 ? "#e2614f" : "#c84a3a";
      g.fillRect(x, y, w, h);
      g.fillStyle = "#fff3ee";
      g.fillRect(x + 5, y + 1, 2, 4);
      g.fillRect(x + 5, y + 6, 2, 1);
    } else if (st === "done" && a?.unread) {
      g.fillStyle = "#2f6b4a";
      g.fillRect(x, y, w, h);
      g.fillStyle = "#a9e3bf";
      g.fillRect(x + 3, y + 4, 1, 1);
      g.fillRect(x + 4, y + 5, 1, 1);
      g.fillRect(x + 5, y + 4, 1, 1);
      g.fillRect(x + 6, y + 3, 1, 1);
      g.fillRect(x + 7, y + 2, 1, 1);
    } else {
      g.fillStyle = a ? "#3b4654" : "#262b34";
      g.fillRect(x, y, w, h);
      g.fillStyle = "rgba(255,255,255,0.08)";
      g.fillRect(x + 1, y + 1, 3, 1);
    }
  }

  /** A small sign beside a door. */
  private plaque(g: CanvasRenderingContext2D, x: number, y: number, text: string, flag: boolean, quiet = false, align: "left" | "right" | "center" = "right") {
    // Room names are signage: big enough to read across the office, growing as you zoom in.
    const z = LEVELS[this.zoom] / this.dpr;
    const size = Math.round(Math.min(19, Math.max(13, 6.5 * z)));
    const h = size + 11;
    g.font = `700 ${size}px ${FONT}`;
    const w = g.measureText(text).width + size * 1.3;
    if (align === "right") x -= w;
    if (align === "center") x -= w / 2;
    g.fillStyle = flag ? "#bf3a2c" : quiet ? "rgba(42,36,51,0.5)" : "#2a2433";
    this.round(g, x, y, w, h, 6);
    g.fill();
    if (quiet) {
      g.setLineDash([3, 2]);
      g.strokeStyle = "rgba(255,255,255,0.6)";
      g.lineWidth = 1;
      g.stroke();
      g.setLineDash([]);
    }
    g.fillStyle = "#fff";
    g.textBaseline = "middle";
    g.fillText(text, x + size * 0.65, y + h / 2 + 0.5);
  }

  private caption(g: CanvasRenderingContext2D, x: number, y: number, text: string, max: number) {
    g.font = `500 10.5px ${FONT}`;
    let t = text;
    if (g.measureText(t).width > max) {
      while (t.length > 4 && g.measureText(t + "…").width > max) t = t.slice(0, -1);
      t = t.trimEnd() + "…";
    }
    const w = g.measureText(t).width + 10;
    g.fillStyle = "rgba(28,26,36,0.5)";
    this.round(g, x - w / 2, y, w, 14, 7);
    g.fill();
    g.fillStyle = "rgba(255,255,255,0.92)";
    g.textBaseline = "middle";
    g.textAlign = "center";
    g.fillText(t, x, y + 7.5);
    g.textAlign = "left";
  }

  /** Name, plus the area in its own colour when the agent works in a named area. */
  private nameTag(g: CanvasRenderingContext2D, x: number, y: number, text: string, bg: string, area?: string | null) {
    g.font = `600 11px ${FONT}`;
    const nw = g.measureText(text).width + 12;
    g.font = `600 10.5px ${FONT}`;
    const aw = area ? g.measureText(area).width + 11 : 0;
    const left = x - (nw + aw) / 2;
    g.textBaseline = "middle";
    if (area) {
      g.fillStyle = areaColor(area);
      this.round(g, left, y, nw + aw, 16, 8);
      g.fill();
      g.fillStyle = "#fff";
      g.fillText(area, left + nw + 5, y + 8.5);
    }
    g.fillStyle = bg;
    this.round(g, left, y, nw, 16, area ? [8, 0, 0, 8] : 8);
    g.fill();
    g.font = `600 11px ${FONT}`;
    g.fillStyle = "#fff";
    g.textAlign = "center";
    g.fillText(text, left + nw / 2, y + 8.5);
    g.textAlign = "left";
  }

  private badge(g: CanvasRenderingContext2D, x: number, y: number, mark: string, now: number) {
    const bob = !this.calm && (mark === "?" || mark === "!") ? Math.round(Math.sin(now / 220) * 2) : 0;
    const w = mark === "dots" ? 26 : 20;
    const top = y - 20 + bob;
    g.fillStyle = "#fff";
    g.strokeStyle = "#2a2433";
    g.lineWidth = 1.5;
    this.round(g, x - w / 2, top, w, 18, 9);
    g.fill();
    g.stroke();
    g.beginPath();
    g.moveTo(x - 4, top + 17);
    g.lineTo(x, top + 22);
    g.lineTo(x + 3, top + 17);
    g.fill();
    if (mark === "dots") {
      const phase = Math.floor(now / 300) % 3;
      for (let i = 0; i < 3; i++) {
        g.fillStyle = i === phase ? "#2b6d93" : "#9cc1d6";
        g.fillRect(x - 7 + i * 6, top + 8, 3, 3);
      }
      return;
    }
    g.fillStyle = mark === "✓" ? "#2f7350" : "#bf3a2c";
    g.font = `800 12px ${FONT}`;
    g.textAlign = "center";
    g.textBaseline = "middle";
    g.fillText(mark, x, top + 9.5);
    g.textAlign = "left";
  }

  private speech(g: CanvasRenderingContext2D, x: number, y: number, text: string, needs: boolean) {
    g.font = `500 12.5px ${FONT}`;
    const maxW = 230;
    const words = text.split(/\s+/);
    const lines: string[] = [];
    let cur = "";
    for (const word of words) {
      const next = cur ? `${cur} ${word}` : word;
      if (g.measureText(next).width > maxW && cur) {
        lines.push(cur);
        cur = word;
      } else cur = next;
    }
    if (cur) lines.push(cur);
    if (lines.length > 3) {
      lines.length = 3;
      lines[2] = lines[2].replace(/\s*\S*$/, "") + "…";
    }
    const w = Math.min(maxW, Math.max(...lines.map((ln) => g.measureText(ln).width))) + 20;
    const h = lines.length * 17 + 14;
    const top = y - h - 8;
    g.fillStyle = "#fffdf8";
    g.strokeStyle = needs ? "#bf3a2c" : "#2a2433";
    g.lineWidth = 2;
    this.round(g, x - w / 2, top, w, h, 8);
    g.fill();
    g.stroke();
    g.beginPath();
    g.moveTo(x - 6, top + h - 1);
    g.lineTo(x, top + h + 7);
    g.lineTo(x + 5, top + h - 1);
    g.fill();
    g.fillStyle = "#2a2433";
    g.textBaseline = "top";
    lines.forEach((ln, i) => g.fillText(ln, x - w / 2 + 10, top + 8 + i * 17));
  }

  private round(g: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number | number[]) {
    g.beginPath();
    g.roundRect(x, y, w, h, r);
  }
}
