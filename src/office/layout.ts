// The office as a tile map. One tile is 16 art pixels. The chief of staff's desk sits in a central
// atrium and the rooms ring it, each with its door facing in, so every room is a few steps from the
// middle. Rooms are sized to the agents in them plus a free desk.

import type { Room } from "../api";

export const T = { VOID: 0, WALL: 1, WOOD: 2, STONE: 3, DOOR: 4 } as const;

const CORRIDOR = 2;
/** Tiles from one desk to the next, across and down. Room to stand up between desks. */
export const PITCH = 4;
const ATRIUM_W = 18;
const ATRIUM_H = 8;
export const VACANT_ID = "vacant";

export type Side = "top" | "right" | "bottom" | "left";

export interface Slot {
  x: number; // left tile of the 2-tile desk
  y: number; // desk tile row; the chair is the row below
}

export interface RoomBox {
  room: Room;
  side: Side;
  lounge?: boolean;
  vacant?: boolean;
  x0: number; // left wall column
  x1: number; // right wall column
  y0: number; // top wall row
  y1: number; // bottom wall row
  slots: Slot[];
  rug: number;
  /** Where the name sign hangs, and which way it reads from there. */
  sign: { x: number; y: number; align: "left" | "right" | "center" };
}

export interface Prop {
  kind: string;
  x: number; // tile coords of the prop's footprint
  y: number;
  w: number;
  h: number;
  blocks?: boolean;
}

export interface Layout {
  w: number;
  h: number;
  ox: number; // tile x of map column 0
  tiles: Uint8Array;
  rooms: RoomBox[];
  props: Prop[];
  /** Where new agents appear (beside the chief of staff who hired them) and leaving ones go. */
  entrance: { x: number; y: number };
  cos: { x: number; y: number };
  reception: { x: number; y: number };
  spawn: { x: number; y: number };
  key: string;
}

interface Shape {
  cols: number;
  rows: number;
  w: number; // left wall to right wall
  h: number; // top wall to bottom wall
}

/** Desks a room needs: one per agent, plus a free one to hire into. */
const desksFor = (agents: number) => agents + 1;

function sized(cols: number, rows: number): Shape {
  // A tile of floor either side of the desks, and below the last row of chairs.
  return { cols, rows, w: (cols - 1) * PITCH + 5, h: (rows - 1) * PITCH + 6 };
}

type Item = { room: Room; desks: number; s: Shape };

/** A top or bottom row shares its depth: the busiest room is up to four desks wide, with as
 * many rows as that takes, and every other room is only as wide as its desks need. */
function shapeRow(items: Item[]) {
  if (!items.length) return;
  const most = Math.max(...items.map((it) => it.desks));
  const rows = Math.max(1, Math.ceil(most / 4));
  for (const it of items) {
    // The add-a-room plot keeps a readable width.
    const min = it.room.id === VACANT_ID ? 2 : 1;
    const cols = it.desks === most ? Math.min(4, it.desks) : Math.ceil(it.desks / rows);
    it.s = sized(Math.max(min, cols), rows);
  }
}

/** A side column shares its width, and each room is only as deep as its desks need. */
function shapeColumn(items: Item[]) {
  if (!items.length) return;
  const most = Math.max(...items.map((it) => it.desks));
  const cols = most <= 4 ? 2 : 3;
  for (const it of items) it.s = sized(cols, Math.max(1, Math.ceil(it.desks / cols)));
}

export function layoutOffice(rooms: Room[], counts: Map<string, number>): Layout {
  const vacant: Room = { id: VACANT_ID, name: "New room", original: "New room", path: "", kind: "folder", color: null, areas: [] };
  const items: Item[] = [...rooms, vacant].map((room) => ({
    room,
    desks: room.id === VACANT_ID ? 1 : desksFor(counts.get(room.id) ?? 0),
    s: sized(2, 1),
  }));

  // Share rooms out around the ring, clockwise from the top left.
  const n = items.length;
  const perSide = n >= 7 ? Math.min(3, Math.floor((n - 4) / 3)) : 0;
  const nTop = Math.ceil((n - 2 * perSide) / 2);
  const nBottom = n - 2 * perSide - nTop;
  const top = items.slice(0, nTop);
  const right = items.slice(nTop, nTop + perSide);
  const bottom = items.slice(nTop + perSide, nTop + perSide + nBottom);
  const left = items.slice(nTop + perSide + nBottom);
  shapeRow(top);
  shapeRow(bottom);
  shapeColumn(left);
  shapeColumn(right);

  const rowW = (r: Item[]) => r.reduce((a, it) => a + it.s.w, 0);
  const colH = (c: Item[]) => (c.length ? c.reduce((a, it) => a + it.s.h - 1, 0) + 1 : 0);
  const colW = (c: Item[]) => (c.length ? Math.max(...c.map((it) => it.s.w)) : 0);
  const leftW = colW(left);
  const rightW = colW(right);
  const W = Math.max(rowW(top), rowW(bottom), leftW + ATRIUM_W + rightW);
  const topH = top.length ? Math.max(...top.map((it) => it.s.h)) : 1;
  const bottomH = bottom.length ? Math.max(...bottom.map((it) => it.s.h)) : 1;
  const bandH = Math.max(colH(left), colH(right), ATRIUM_H);
  const H = topH + CORRIDOR + bandH + CORRIDOR + bottomH;
  const bandY0 = topH + CORRIDOR;

  const w = W + 1;
  const tiles = new Uint8Array(w * H);
  const fill = (x0: number, y0: number, x1: number, y1: number, t: number) => {
    for (let y = y0; y <= y1; y++)
      for (let x = x0; x <= x1; x++) if (x >= 0 && x < w && y >= 0 && y < H) tiles[y * w + x] = t;
  };
  fill(0, 0, W, H - 1, T.STONE);
  fill(0, 0, W, 0, T.WALL);
  fill(0, H - 1, W, H - 1, T.WALL);
  fill(0, 0, 0, H - 1, T.WALL);
  fill(W, 0, W, H - 1, T.WALL);

  const out: RoomBox[] = [];
  const props: Prop[] = [];

  const build = (room: Room, s: Shape, side: Side, x0: number, y0: number, x1: number, y1: number) => {
    const isVacant = room.id === VACANT_ID;
    fill(x0 + 1, y0 + 1, x1 - 1, y1 - 1, T.WOOD);
    fill(x0, y0, x1, y0, T.WALL);
    fill(x0, y1, x1, y1, T.WALL);
    fill(x0, y0, x0, y1, T.WALL);
    fill(x1, y0, x1, y1, T.WALL);
    const mid = x0 + Math.round((x1 - x0) / 2) - 1;
    let sign: RoomBox["sign"];
    if (side === "top") {
      fill(mid, y1, mid + 1, y1, T.DOOR);
      sign = { x: mid - 0.15, y: y1 + 0.2, align: "right" };
    } else if (side === "bottom") {
      fill(mid, y0, mid + 1, y0, T.DOOR);
      sign = { x: mid - 0.15, y: y0 + 0.2, align: "right" };
    } else if (side === "left") {
      // Side rooms wear their name on their own back wall, at the end nearest the door.
      fill(x1, y0 + 3, x1, y0 + 4, T.DOOR);
      sign = { x: x1 - 0.3, y: y0 + 0.25, align: "right" };
    } else {
      fill(x0, y0 + 3, x0, y0 + 4, T.DOOR);
      sign = { x: x0 + 1.3, y: y0 + 0.25, align: "left" };
    }
    if (isVacant) sign = { x: (x0 + x1 + 1) / 2, y: (y0 + y1) / 2 - 0.1, align: "center" };
    const slots: Slot[] = [];
    if (!isVacant)
      for (let k = 0; k < s.cols * s.rows; k++)
        slots.push({ x: x0 + 2 + (k % s.cols) * PITCH, y: y0 + 2 + Math.floor(k / s.cols) * PITCH });
    out.push({ room, side, vacant: isVacant, x0, x1, y0, y1, slots, rug: out.length % 8, sign });
    if (isVacant) {
      props.push({ kind: "plot", x: x0 + 1, y: y0 + 1, w: x1 - x0 - 1, h: y1 - y0 - 1 });
      return;
    }
    // Furniture keeps clear of the door and the sign.
    const right = side === "right";
    props.push(
      { kind: "bookcase", x: right ? x1 - 1 : x0 + 1, y: y0 + 1, w: 1, h: 1, blocks: true },
      { kind: "plant-small", x: right ? x1 - 1 : x0 + 1, y: y1 - 1, w: 1, h: 1, blocks: true },
    );
    if (side === "top") props.push({ kind: "plant", x: x1 - 1, y: y0 + 1, w: 1, h: 1, blocks: true });
    if (side !== "bottom") props.push({ kind: "whiteboard", x: right ? x1 - 4 : x0 + 2, y: y0, w: 2, h: 1 });
    if (side === "top" && x1 - x0 >= 9) props.push({ kind: "window", x: x1 - 3, y: y0, w: 2, h: 1 });
  };

  // Rooms keep their own size; a row that's shorter than the office is centred, and the space
  // either side is open floor off the hall rather than a stretched, empty room.
  // Top row, left to right, doors facing down into the corridor.
  let x = Math.floor((W - rowW(top)) / 2);
  for (const it of top) {
    build(it.room, it.s, "top", x, 0, x + it.s.w, topH - 1);
    x += it.s.w;
  }
  // Bottom row, right to left, doors facing up.
  x = W - Math.floor((W - rowW(bottom)) / 2);
  for (const it of bottom) {
    build(it.room, it.s, "bottom", x - it.s.w, H - 1 - (it.s.h - 1), x, H - 1);
    x -= it.s.w;
  }
  // Side columns, stacked from the top of the band, doors facing the atrium.
  const stack = (col: Item[], side: Side, x0: number, x1: number) => {
    let y = bandY0;
    for (const it of col) {
      build(it.room, it.s, side, x0, y, x1, y + it.s.h - 1);
      y += it.s.h - 1;
    }
  };
  stack(right, "right", W - rightW, W);
  stack(left, "left", 0, leftW);

  // The atrium: the chief of staff in the middle; everyone else is a few steps away.
  const ax0 = leftW + 1;
  const ax1 = W - rightW - 1;
  const cx = Math.floor((ax0 + ax1 + 1) / 2);
  const cy = bandY0 + Math.floor(bandH / 2) - 2;
  // Kept deliberately bare: the chief of staff's desk on one big rug, a plant either side.
  const rugW = Math.min(ax1 - ax0 - 3, 20);
  const rugH = Math.min(bandH - 2, 8);
  props.push(
    { kind: "rug-hall", x: cx - Math.floor(rugW / 2), y: Math.round(cy + 1.5 - rugH / 2), w: rugW, h: rugH },
    { kind: "reception", x: cx - 2, y: cy + 1, w: 4, h: 1, blocks: true },
    { kind: "plant-big", x: cx - 4, y: cy + 1, w: 1, h: 1, blocks: true },
    { kind: "plant-big", x: cx + 3, y: cy + 1, w: 1, h: 1, blocks: true },
  );
  // A tree at each far end of the two halls, clear of every door.
  const topHall = topH + CORRIDOR - 1;
  // One row up from the bottom rooms' wall, so the trees clear their bookcases.
  const bottomHall = H - bottomH - CORRIDOR;
  for (const y of [topHall, bottomHall])
    for (const x of [1, W - 2]) props.push({ kind: "tree", x, y, w: 2, h: 1, blocks: true });

  const key = out.map((r) => `${r.room.id}:${r.slots.length}@${r.x0},${r.y0},${r.x1},${r.y1}`).join("|");
  return {
    w,
    h: H,
    ox: 0,
    tiles,
    rooms: out,
    props,
    entrance: { x: cx + 3.5, y: cy + 3 },
    cos: { x: cx, y: cy + 0.95 }, // just behind the desk, so it hides their legs
    reception: { x: cx, y: cy },
    spawn: { x: cx, y: cy + 2.8 },
    key,
  };
}

export function tileAt(l: Layout, x: number, y: number): number {
  const i = Math.floor(x) - l.ox;
  const j = Math.floor(y);
  if (i < 0 || j < 0 || i >= l.w || j >= l.h) return T.VOID;
  return l.tiles[j * l.w + i];
}
