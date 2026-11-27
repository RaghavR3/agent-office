// All of the office's pixel art, painted in code at 16 px per tile. Nothing is loaded from disk.

import { Layout, T, tileAt } from "./layout";

export const TILE = 16;

type Ctx = CanvasRenderingContext2D;

export function canvas(w: number, h: number): [HTMLCanvasElement, Ctx] {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const g = c.getContext("2d")!;
  g.imageSmoothingEnabled = false;
  return [c, g];
}

const rect = (g: Ctx, x: number, y: number, w: number, h: number, c: string) => {
  g.fillStyle = c;
  g.fillRect(x, y, w, h);
};

function shade(hex: string, f: number): string {
  const n = parseInt(hex.slice(1), 16);
  const ch = (s: number) => {
    const v = (n >> s) & 255;
    return Math.round(f < 0 ? v * (1 + f) : v + (255 - v) * f);
  };
  return `rgb(${ch(16)},${ch(8)},${ch(0)})`;
}

function rng(seed: number) {
  let s = seed % 2147483647 || 1;
  return () => (s = (s * 16807) % 2147483647) / 2147483647;
}

const P = {
  outline: "#2a2433",
  void: "#2b2d38",
  wood: ["#c99c6e", "#c09163", "#cfa577", "#c59767"],
  woodSeam: "#a67a50",
  // The halls: office carpet tiles, laid with the grain turning tile to tile.
  carpet: "#d8d1c4",
  carpetAlt: "#d5cec0",
  hallRug: "#93a69b",
  wallCap: "#4b4559",
  wallCapHi: "#605a72",
  wallFace: "#ede7dd",
  wallFaceLo: "#e0d8cb",
  base: "#b7aa97",
  rugs: ["#7e9c8d", "#b47f70", "#8f87aa", "#c4a35f", "#6f90a7", "#a4876d", "#82a271", "#b58b9b"],
  deskTop: "#e2b886",
  deskHi: "#edc899",
  deskFront: "#b88455",
  deskDark: "#986a42",
  leg: "#5b4636",
  bezel: "#2d2b36",
  steel: "#3d3a47",
  keys: "#f1eee8",
  chair: "#3f4858",
  chairHi: "#525d71",
  chairDark: "#2c313b",
  pot: "#c56a48",
  potRim: "#da8560",
  potDark: "#a6553a",
  leaves: ["#4e8b4a", "#62a258", "#3d6f3b", "#7ab86a"],
  shelf: "#7b5136",
  shelfLight: "#916242",
  shelfBack: "#5b3b28",
  books: ["#c25b4f", "#4f7aa3", "#d6b24c", "#5d9a6b", "#8b6aa8", "#e6dccb", "#d9824f", "#3f5b73"],
  sofa: "#5f8f96",
  sofaHi: "#79a9b0",
  sofaDark: "#4a7378",
  lobbyRug: "#b9a07a",
  stone: ["#a7a39c", "#bdb9b2", "#8f8b84"],
  lampShade: ["#f0d9a7", "#f8e9c8", "#d8bf8a"],
  windowFrame: "#5d5670",
  reception: { top: "#f2eee7", topHi: "#fbf9f5", front: "#8f5f3e", trim: "#c9a46a", slat: "#7d5135", base: "#6b4429" },
  lift: { frame: "#6f7480", frameHi: "#8a909b", door: "#aab1bb", doorHi: "#bdc4cd", seam: "#7d838e", sill: "#565a64" },
};

export type Theme = "office" | "summer";

/** Name your chief of staff Summer and the office becomes a pink dream house of golden retrievers. */
export const themeFor = (cosName: string): Theme => (cosName.trim().toLowerCase() === "summer" ? "summer" : "office");

const OFFICE: typeof P = structuredClone(P);

const SUMMER: typeof P = {
  outline: "#4a1035",
  void: "#3d0f2e",
  wood: ["#f6b3cf", "#f2a8c7", "#f9bdd6", "#f4aecb"],
  woodSeam: "#de86ad",
  carpet: "#ffe0ee",
  carpetAlt: "#fff7fb",
  hallRug: "#ff7eb6",
  wallCap: "#c2185b",
  wallCapHi: "#e0478a",
  wallFace: "#ffe4f1",
  wallFaceLo: "#ffd1e7",
  base: "#f06292",
  rugs: ["#ff69b4", "#c77dff", "#4dd0e1", "#ff8fab", "#f48fb1", "#ba68c8", "#ffb3d9", "#ef5d9a"],
  deskTop: "#fff6fb",
  deskHi: "#ffffff",
  deskFront: "#ff6fae",
  deskDark: "#e0438f",
  leg: "#b0185a",
  bezel: "#8e1450",
  steel: "#c2477f",
  keys: "#ffffff",
  chair: "#ff4fa3",
  chairHi: "#ff8cc6",
  chairDark: "#c2185b",
  pot: "#ff8fc8",
  potRim: "#ffb8dd",
  potDark: "#e0679f",
  leaves: ["#4e9a5a", "#63b066", "#3f7c45", "#ff5fa8"],
  shelf: "#f8bbd9",
  shelfLight: "#fde4f0",
  shelfBack: "#e48ab5",
  books: ["#ff4fa3", "#c77dff", "#ffd23f", "#4dd0e1", "#ff8fab", "#ffffff", "#ba68c8", "#ef5d9a"],
  sofa: "#ff69b4",
  sofaHi: "#ff9acb",
  sofaDark: "#d63384",
  lobbyRug: "#ff4fa3",
  stone: ["#fff3f9", "#ffffff", "#f6c6dd"],
  lampShade: ["#ff8cc6", "#ffc2e0", "#e0679f"],
  windowFrame: "#d81b60",
  reception: { top: "#ffffff", topHi: "#fff6fb", front: "#ff4fa3", trim: "#ffd23f", slat: "#e0438f", base: "#b0185a" },
  lift: { frame: "#ff69b4", frameHi: "#ff9acb", door: "#ffd23f", doorHi: "#ffe57a", seam: "#e0a800", sill: "#c2185b" },
};

let theme: Theme = "office";
export const currentTheme = () => theme;

/** Repaints everything in the theme's colours from here on. Says whether anything changed. */
export function setTheme(t: Theme): boolean {
  if (t === theme) return false;
  theme = t;
  Object.assign(P, structuredClone(t === "summer" ? SUMMER : OFFICE));
  propCache.clear();
  portraits.clear();
  return true;
}

/** Colours the office view draws itself, outside the sprites. */
export const scene = () => ({ void: P.void, cosTag: theme === "summer" ? "#d6338a" : "#2f7350" });

// ---- floor layer ----------------------------------------------------------------------------

function woodTile(g: Ctx, x: number, y: number, r: () => number) {
  for (let row = 0; row < 4; row++) {
    const py = y + row * 4;
    rect(g, x, py, TILE, 4, P.wood[Math.floor(r() * P.wood.length)]);
    rect(g, x, py + 3, TILE, 1, P.woodSeam);
    const joint = Math.floor(r() * 14) + 1;
    rect(g, x + joint, py, 1, 3, P.woodSeam);
    if (r() > 0.6) rect(g, x + Math.floor(r() * 12) + 2, py + 1, 3, 1, shade(P.wood[0], -0.06));
  }
}

function carpetTile(g: Ctx, x: number, y: number, turn: boolean, r: () => number) {
  const base = turn ? P.carpetAlt : P.carpet;
  rect(g, x, y, TILE, TILE, base);
  g.fillStyle = shade(base, -0.015);
  for (let k = 2; k < TILE; k += 4) {
    if (turn) g.fillRect(x + k, y, 1, TILE);
    else g.fillRect(x, y + k, TILE, 1);
  }
  for (let k = 0; k < 3; k++) rect(g, x + Math.floor(r() * 15), y + Math.floor(r() * 15), 1, 1, shade(base, -0.07));
}

function wallTile(g: Ctx, x: number, y: number, front: boolean) {
  if (!front) {
    rect(g, x, y, TILE, TILE, P.wallCap);
    rect(g, x, y, TILE, 1, P.wallCapHi);
    return;
  }
  rect(g, x, y, TILE, 4, P.wallCap);
  rect(g, x, y, TILE, 1, P.wallCapHi);
  rect(g, x, y + 4, TILE, 10, P.wallFace);
  rect(g, x, y + 11, TILE, 3, P.wallFaceLo);
  rect(g, x, y + 14, TILE, 2, P.base);
}

function rug(g: Ctx, x: number, y: number, w: number, h: number, color: string) {
  rect(g, x, y, w, h, shade(color, -0.22));
  rect(g, x + 2, y + 2, w - 4, h - 4, color);
  g.fillStyle = shade(color, 0.18);
  g.fillRect(x + 4, y + 4, w - 8, 1);
  g.fillRect(x + 4, y + h - 5, w - 8, 1);
  g.fillRect(x + 4, y + 4, 1, h - 8);
  g.fillRect(x + w - 5, y + 4, 1, h - 8);
  for (let py = y + 8; py < y + h - 8; py += 6)
    for (let px = x + 8 + ((py / 6) % 2) * 3; px < x + w - 8; px += 6) g.fillRect(px, py, 1, 1);
}

function whiteboard(g: Ctx, x: number, y: number) {
  rect(g, x + 2, y + 4, 28, 9, "#a9a39a");
  rect(g, x + 3, y + 5, 26, 7, "#fbfaf7");
  rect(g, x + 5, y + 7, 9, 1, "#4f7aa3");
  rect(g, x + 5, y + 9, 6, 1, "#c25b4f");
  rect(g, x + 16, y + 6, 1, 4, "#5d9a6b");
  rect(g, x + 17, y + 8, 6, 1, "#5d9a6b");
  rect(g, x + 24, y + 7, 3, 3, "#d6b24c");
  rect(g, x + 8, y + 13, 16, 1, "#8d877e");
}

function windowOnWall(g: Ctx, x: number, y: number) {
  rect(g, x + 3, y + 4, 26, 9, P.windowFrame);
  rect(g, x + 4, y + 5, 11, 7, "#bfe1ee");
  rect(g, x + 17, y + 5, 11, 7, "#bfe1ee");
  rect(g, x + 5, y + 6, 3, 1, "#e9f6fb");
  rect(g, x + 18, y + 6, 3, 1, "#e9f6fb");
  rect(g, x + 4, y + 10, 24, 2, "#a9d3e3");
}

export function floorLayer(l: Layout): HTMLCanvasElement {
  const [c, g] = canvas(l.w * TILE, l.h * TILE);
  for (let j = 0; j < l.h; j++)
    for (let i = 0; i < l.w; i++) {
      const t = l.tiles[j * l.w + i];
      const x = i * TILE;
      const y = j * TILE;
      const r = rng((i + 7) * 7919 + (j + 3) * 104729);
      if (t === T.VOID) rect(g, x, y, TILE, TILE, P.void);
      else if (t === T.WOOD) woodTile(g, x, y, r);
      else if (t === T.WALL) {
        const below = tileAt(l, i + l.ox, j + 1);
        wallTile(g, x, y, below !== T.WALL && below !== T.VOID);
      } else carpetTile(g, x, y, (i + j) % 2 === 1, r);
    }
  const X = (tx: number) => (tx - l.ox) * TILE;
  for (const rb of l.rooms) {
    if (rb.vacant) continue;
    rug(g, X(rb.x0 + 1) + 6, (rb.y0 + 2) * TILE + 6, (rb.x1 - rb.x0 - 1) * TILE - 12, (rb.y1 - rb.y0 - 2) * TILE - 12, P.rugs[rb.rug]);
  }
  for (const p of l.props) {
    if (p.kind === "whiteboard") whiteboard(g, X(p.x), p.y * TILE);
    if (p.kind === "window") windowOnWall(g, X(p.x), p.y * TILE);
    if (p.kind === "rug-lobby") rug(g, X(p.x), p.y * TILE, p.w * TILE, p.h * TILE, P.lobbyRug);
    if (p.kind === "rug-hall") rug(g, X(p.x) + 4, p.y * TILE + 4, p.w * TILE - 8, p.h * TILE - 8, P.hallRug);
    if (p.kind === "plot") {
      // A dashed outline where a room's desks would go: an invitation, not a building site.
      const x = X(p.x) + 8, y = p.y * TILE + 8, w = p.w * TILE - 16, h = p.h * TILE - 16;
      g.fillStyle = "rgba(255,255,255,0.18)";
      g.fillRect(x, y, w, h);
      g.fillStyle = "rgba(90,70,50,0.45)";
      for (let i = 0; i < w; i += 6) {
        g.fillRect(x + i, y, 3, 1);
        g.fillRect(x + i, y + h - 1, 3, 1);
      }
      for (let i = 0; i < h; i += 6) {
        g.fillRect(x, y + i, 1, 3);
        g.fillRect(x + w - 1, y + i, 1, 3);
      }
    }
    if (p.kind === "mat") {
      rect(g, X(p.x) + 2, p.y * TILE + 4, 28, 11, "#5b4d45");
      rect(g, X(p.x) + 4, p.y * TILE + 6, 24, 7, "#6d5d53");
    }
    // Soft contact shadow under anything that stands on the floor.
    if (p.blocks) {
      g.fillStyle = "rgba(60,40,25,0.16)";
      g.fillRect(X(p.x) + 1, (p.y + p.h) * TILE - 3, p.w * TILE - 2, 3);
    }
  }
  return c;
}

// ---- furniture ------------------------------------------------------------------------------

export interface Sprite {
  img: HTMLCanvasElement;
  /** Pixels the sprite rises above its footprint. */
  up: number;
}

function leaves(g: Ctx, cx: number, cy: number, rx: number, ry: number, n: number, seed: number) {
  const r = rng(seed);
  for (let k = 0; k < n; k++) {
    const a = r() * Math.PI * 2;
    const d = Math.sqrt(r());
    const x = Math.round(cx + Math.cos(a) * rx * d);
    const y = Math.round(cy + Math.sin(a) * ry * d);
    const col = P.leaves[Math.floor(r() * 3)];
    rect(g, x - 1, y - 1, 3, 2, col);
    rect(g, x, y - 2, 1, 4, col);
  }
  for (let k = 0; k < n / 3; k++) {
    const x = Math.round(cx + (r() - 0.5) * rx * 1.6);
    const y = Math.round(cy - ry * 0.4 + (r() - 0.5) * ry);
    rect(g, x, y, 1, 1, P.leaves[3]);
  }
}

/** A potted indoor tree, two tiles wide, for the corners of the halls. */
function tree(): Sprite {
  const up = 26;
  const [c, g] = canvas(32, 16 + up);
  const top = 16 + up;
  rect(g, 15, 14, 2, top - 24, "#7a5a40");
  rect(g, 16, 14, 1, top - 24, "#64472f");
  leaves(g, 16, 14, 13, 12, 150, 97);
  leaves(g, 11, 20, 7, 5, 40, 131);
  leaves(g, 21, 19, 7, 5, 40, 173);
  rect(g, 9, top - 10, 14, 9, P.stone[0]);
  rect(g, 8, top - 11, 16, 2, P.stone[1]);
  rect(g, 20, top - 9, 3, 8, P.stone[2]);
  return { img: c, up };
}

function plant(size: "small" | "mid" | "big"): Sprite {
  const up = size === "big" ? 24 : size === "mid" ? 14 : 6;
  const [c, g] = canvas(16, 16 + up);
  const top = 16 + up;
  const potH = size === "big" ? 9 : 7;
  const potTop = top - potH - 1;
  leaves(g, 8, size === "big" ? 13 : potTop - (size === "mid" ? 7 : 4), size === "big" ? 7 : 6, size === "big" ? 12 : size === "mid" ? 8 : 5, size === "big" ? 70 : 40, size.length * 31);
  const pot = size === "big" ? P.stone[0] : P.pot;
  rect(g, 4, potTop + 1, 8, potH, pot);
  rect(g, 3, potTop, 10, 2, size === "big" ? P.stone[1] : P.potRim);
  rect(g, 10, potTop + 2, 2, potH - 1, size === "big" ? P.stone[2] : P.potDark);
  return { img: c, up };
}

function bookcase(): Sprite {
  const [c, g] = canvas(16, 38);
  rect(g, 1, 0, 14, 38, P.shelf);
  rect(g, 3, 2, 10, 32, P.shelfBack);
  const r = rng(41);
  for (const sy of [11, 22, 33]) {
    rect(g, 3, sy, 10, 2, P.shelfLight);
    let x = 3;
    while (x < 12) {
      const w = 2 + Math.floor(r() * 2);
      const h = 6 + Math.floor(r() * 3);
      if (r() > 0.2) rect(g, x, sy - h, Math.min(w, 13 - x), h, P.books[Math.floor(r() * P.books.length)]);
      x += w;
    }
  }
  rect(g, 1, 35, 14, 3, shade(P.shelf, -0.2));
  return { img: c, up: 22 };
}

function coffee(): Sprite {
  const [c, g] = canvas(16, 26);
  rect(g, 1, 12, 14, 14, P.shelf);
  rect(g, 1, 12, 14, 2, P.shelfLight);
  rect(g, 3, 1, 10, 12, "#3b3a43");
  rect(g, 4, 2, 8, 3, "#55535f");
  rect(g, 10, 3, 1, 1, "#e2614f");
  rect(g, 6, 9, 4, 3, "#f1eee8");
  return { img: c, up: 10 };
}

function cooler(): Sprite {
  const [c, g] = canvas(16, 30);
  rect(g, 5, 0, 6, 9, "#9fd3ea");
  rect(g, 6, 1, 1, 6, "#d6eef7");
  rect(g, 3, 9, 10, 20, "#e9e7e2");
  rect(g, 3, 9, 10, 1, "#f7f6f3");
  rect(g, 11, 10, 2, 19, "#d3d0c9");
  rect(g, 5, 13, 2, 2, "#4f7aa3");
  rect(g, 9, 13, 2, 2, "#c25b4f");
  return { img: c, up: 14 };
}

function lamp(): Sprite {
  const [c, g] = canvas(16, 36);
  rect(g, 3, 0, 10, 7, P.lampShade[0]);
  rect(g, 4, 0, 8, 1, P.lampShade[1]);
  rect(g, 3, 6, 10, 1, P.lampShade[2]);
  rect(g, 7, 7, 2, 25, P.steel);
  rect(g, 4, 32, 8, 3, P.steel);
  return { img: c, up: 20 };
}

function table(): Sprite {
  const [c, g] = canvas(32, 34);
  rect(g, 3, 6, 26, 16, shade(P.deskTop, -0.08));
  rect(g, 2, 4, 28, 16, P.deskTop);
  rect(g, 2, 4, 28, 1, P.deskHi);
  rect(g, 4, 20, 2, 10, P.leg);
  rect(g, 26, 20, 2, 10, P.leg);
  rect(g, 8, 9, 4, 3, "#f1eee8");
  rect(g, 20, 12, 5, 4, "#fbfaf6");
  return { img: c, up: 2 };
}

function reception(): Sprite {
  const [c, g] = canvas(64, 30);
  rect(g, 40, 0, 14, 10, P.steel);
  rect(g, 46, 10, 2, 2, P.steel);
  const d = P.reception;
  rect(g, 0, 10, 64, 4, d.top);
  rect(g, 0, 10, 64, 1, d.topHi);
  rect(g, 0, 14, 64, 15, d.front);
  rect(g, 0, 14, 64, 1, d.trim);
  for (let x = 4; x < 64; x += 6) rect(g, x, 16, 1, 12, d.slat);
  rect(g, 0, 29, 64, 1, d.base);
  rect(g, 10, 6, 6, 4, P.pot);
  leaves(g, 13, 3, 4, 3, 14, 5);
  return { img: c, up: 14 };
}

function sofa(): Sprite {
  const [c, g] = canvas(48, 26);
  rect(g, 2, 0, 44, 11, P.sofa);
  rect(g, 2, 0, 44, 1, P.sofaHi);
  rect(g, 4, 10, 40, 8, P.sofaHi);
  rect(g, 18, 10, 1, 8, P.sofa);
  rect(g, 31, 10, 1, 8, P.sofa);
  rect(g, 0, 4, 6, 18, P.sofaDark);
  rect(g, 42, 4, 6, 18, P.sofaDark);
  rect(g, 2, 18, 44, 6, P.sofaDark);
  rect(g, 4, 24, 3, 2, P.leg);
  rect(g, 41, 24, 3, 2, P.leg);
  return { img: c, up: 10 };
}

function coffeeTable(): Sprite {
  const [c, g] = canvas(48, 18);
  rect(g, 2, 2, 44, 9, P.deskTop);
  rect(g, 2, 2, 44, 1, P.deskHi);
  rect(g, 2, 11, 44, 2, P.deskDark);
  rect(g, 5, 13, 2, 5, P.leg);
  rect(g, 41, 13, 2, 5, P.leg);
  rect(g, 10, 4, 8, 5, "#c25b4f");
  rect(g, 11, 5, 6, 3, "#e6dccb");
  rect(g, 32, 4, 3, 3, "#f1eee8");
  return { img: c, up: 2 };
}

export function deskSprite(): Sprite {
  const [c, g] = canvas(32, 28);
  rect(g, 9, 0, 14, 10, P.bezel);
  rect(g, 15, 10, 2, 3, P.steel);
  rect(g, 12, 12, 8, 1, P.steel);
  rect(g, 0, 12, 32, 10, P.deskTop);
  rect(g, 0, 12, 32, 1, P.deskHi);
  rect(g, 12, 12, 8, 1, P.steel);
  rect(g, 3, 18, 1, 1, shade(P.deskTop, -0.08));
  rect(g, 27, 15, 2, 1, shade(P.deskTop, -0.08));
  rect(g, 10, 16, 12, 3, P.keys);
  for (let x = 11; x < 21; x += 2) rect(g, x, 17, 1, 1, "#cfcac1");
  rect(g, 24, 17, 2, 2, P.keys);
  rect(g, 3, 14, 5, 4, "#fbfaf6");
  rect(g, 4, 15, 3, 1, "#cfcac1");
  rect(g, 0, 22, 32, 5, P.deskFront);
  rect(g, 0, 26, 32, 1, P.deskDark);
  rect(g, 1, 27, 3, 1, P.leg);
  rect(g, 28, 27, 3, 1, P.leg);
  return { img: c, up: 12 };
}

/** The monitor's screen area inside the desk sprite. */
export const SCREEN = { x: 10, y: 1, w: 12, h: 8 };

export function chairSprite(): Sprite {
  const [c, g] = canvas(16, 16);
  rect(g, 2, 8, 12, 3, P.chairDark);
  rect(g, 3, 2, 10, 7, P.chair);
  rect(g, 3, 2, 10, 1, P.chairHi);
  rect(g, 3, 2, 1, 7, P.chairHi);
  rect(g, 7, 11, 2, 3, P.chairDark);
  rect(g, 3, 14, 10, 1, P.chairDark);
  rect(g, 3, 15, 1, 1, P.chairDark);
  rect(g, 12, 15, 1, 1, P.chairDark);
  return { img: c, up: 0 };
}

function lift(): Sprite {
  const [c, g] = canvas(32, 42);
  const L = P.lift;
  rect(g, 0, 0, 32, 42, L.frame);
  rect(g, 0, 0, 32, 2, L.frameHi);
  rect(g, 3, 3, 26, 5, P.outline);
  rect(g, 14, 4, 4, 3, "#f0c15a");
  rect(g, 15, 4, 2, 1, "#fff0c2");
  rect(g, 3, 10, 26, 30, L.door);
  rect(g, 15, 10, 2, 30, L.seam);
  rect(g, 5, 12, 8, 26, L.doorHi);
  rect(g, 19, 12, 8, 26, L.doorHi);
  rect(g, 0, 40, 32, 2, L.sill);
  return { img: c, up: 26 };
}

function boxes(): Sprite {
  const [c, g] = canvas(16, 20);
  rect(g, 1, 9, 14, 11, "#c49a62");
  rect(g, 1, 9, 14, 2, "#d6b07a");
  rect(g, 7, 9, 2, 11, "#e5cf9e");
  rect(g, 3, 1, 10, 9, "#b88c55");
  rect(g, 3, 1, 10, 2, "#cba06a");
  rect(g, 7, 1, 2, 9, "#e5cf9e");
  return { img: c, up: 4 };
}

const PROP_ART: Record<string, () => Sprite> = {
  lift,
  boxes,
  bookcase,
  coffee,
  cooler,
  lamp,
  table,
  reception,
  sofa,
  "coffee-table": coffeeTable,
  plant: () => plant("mid"),
  "plant-small": () => plant("small"),
  "plant-big": () => plant("big"),
  tree,
};
const propCache = new Map<string, Sprite>();
export function propSprite(kind: string): Sprite | null {
  if (!PROP_ART[kind]) return null;
  if (!propCache.has(kind)) propCache.set(kind, outlined(PROP_ART[kind]()));
  return propCache.get(kind)!;
}

/** Adds the dark one-pixel outline that gives everything its pixel-game edge. */
function outlined(s: Sprite): Sprite {
  const { width: w, height: h } = s.img;
  const [c, g] = canvas(w + 2, h + 2);
  g.drawImage(s.img, 1, 1);
  const img = g.getImageData(0, 0, w + 2, h + 2);
  const d = img.data;
  const solid = (x: number, y: number) => x >= 0 && y >= 0 && x < w + 2 && y < h + 2 && d[(y * (w + 2) + x) * 4 + 3] > 0;
  const out: number[] = [];
  for (let y = 0; y < h + 2; y++)
    for (let x = 0; x < w + 2; x++)
      if (!solid(x, y) && (solid(x - 1, y) || solid(x + 1, y) || solid(x, y - 1) || solid(x, y + 1))) out.push(x, y);
  g.fillStyle = P.outline;
  for (let k = 0; k < out.length; k += 2) g.fillRect(out[k], out[k + 1], 1, 1);
  return { img: c, up: s.up + 1 };
}
export { outlined };

// ---- people ---------------------------------------------------------------------------------

const SKIN = ["#f6cfae", "#e8b48f", "#c98e66", "#a36a48", "#71472f"];
const HAIR = ["#2b2220", "#5a3a28", "#8b5a35", "#c9965a", "#e3c27e", "#9a9aa3", "#b24a3a", "#3c2f4a"];
const SHIRT = ["#4f7aa3", "#c25b4f", "#5d9a6b", "#d6b24c", "#8b6aa8", "#e6e1d6", "#3d4a5c", "#d9824f", "#4f9a9a"];
const PANTS = ["#2f3442", "#3d4456", "#5a4c3f", "#28354a", "#6b6559"];

export type Dir = "down" | "up" | "left" | "right";
export type Frames = Record<string, HTMLCanvasElement>;

interface Look {
  skin: string;
  skinLo: string;
  hair: string;
  hairHi: string;
  shirt: string;
  shirtLo: string;
  pants: string;
  pantsLo: string;
  style: number;
}

/** Every agent's look comes from their name, so a pending hire and the agent it becomes match. */
export function lookForAgent(a: { name: string; role: string }): Look {
  if (a.role === "cos") return lookFor("chief-of-staff", { shirt: "#5d9a6b", shirtLo: "#4a7d56", style: 2 });
  return lookFor(a.name);
}

export const YOU = () => lookFor("you", { shirt: "#2f3442", shirtLo: "#232733", style: 0 });

const portraits = new Map<string, string>();
/** Head and shoulders of a person, as a data URL for <img> in the UI. */
export function portraitURL(a: { name: string; role: string }): string {
  const k = `${a.role}:${a.name}`;
  if (!portraits.has(k)) {
    const f = frame((g) => (theme === "summer" ? dogFront : front)(g, lookForAgent(a), 0, -1));
    const [c, g] = canvas(16, 16);
    g.drawImage(f, -2, 0);
    portraits.set(k, c.toDataURL());
  }
  return portraits.get(k)!;
}

export function lookFor(seed: string, overrides: Partial<Look> = {}): Look {
  let h = 2166136261;
  for (let i = 0; i < seed.length; i++) h = Math.imul(h ^ seed.charCodeAt(i), 16777619);
  h >>>= 0;
  const skin = SKIN[h % SKIN.length];
  const hair = HAIR[(h >>> 3) % HAIR.length];
  const shirt = SHIRT[(h >>> 6) % SHIRT.length];
  const pants = PANTS[(h >>> 10) % PANTS.length];
  return {
    skin,
    skinLo: shade(skin, -0.14),
    hair,
    hairHi: shade(hair, 0.18),
    shirt,
    shirtLo: shade(shirt, -0.18),
    pants,
    pantsLo: shade(pants, -0.25),
    style: (h >>> 13) % 4,
    ...overrides,
  };
}

const EYE = "#2a2630";
const SHOE = "#2a2630";

function front(g: Ctx, L: Look, step: number, wave: number, dy = 0) {
  const r = (x: number, y: number, w: number, h: number, c: string) => rect(g, x + 1, y + 1 + dy, w, h, c);
  if (wave < 0) {
    const lf = step === 1 ? 0 : step === -1 ? -1 : 0;
    const rf = step === -1 ? 0 : step === 1 ? -1 : 0;
    r(5, 18, 2, 5 + lf, L.pants);
    r(9, 18, 2, 5 + rf, L.pants);
    r(5, 23 + lf, 2, 1, SHOE);
    r(9, 23 + rf, 2, 1, SHOE);
  } else {
    r(5, 18, 2, 5, L.pants);
    r(9, 18, 2, 5, L.pants);
    r(5, 23, 2, 1, SHOE);
    r(9, 23, 2, 1, SHOE);
  }
  r(4, 10, 8, 8, L.shirt);
  r(10, 11, 2, 6, L.shirtLo);
  r(6, 10, 4, 1, L.skinLo);
  r(4, 17, 8, 1, L.pantsLo);
  // Arms. A wave raises the right one.
  const la = step === 1 ? -1 : 0;
  r(3, 11, 1, 5 + la, L.shirtLo);
  r(3, 16 + la, 1, 1, L.skin);
  if (wave >= 0) {
    if (wave === 0) {
      r(12, 5, 1, 6, L.shirtLo);
      r(12, 4, 1, 1, L.skin);
    } else {
      r(12, 8, 1, 3, L.shirtLo);
      r(13, 5, 1, 3, L.shirtLo);
      r(13, 4, 1, 1, L.skin);
    }
  } else {
    const ra = step === -1 ? -1 : 0;
    r(12, 11, 1, 5 + ra, L.shirtLo);
    r(12, 16 + ra, 1, 1, L.skin);
  }
  r(6, 9, 4, 1, L.skinLo);
  r(4, 3, 8, 5, L.skin);
  r(5, 2, 6, 1, L.skin);
  r(5, 8, 6, 1, L.skin);
  r(6, 6, 1, 1, EYE);
  r(9, 6, 1, 1, EYE);
  r(7, 8, 2, 1, L.skinLo);
  hairFront(r, L);
}

function hairFront(r: (x: number, y: number, w: number, h: number, c: string) => void, L: Look) {
  switch (L.style) {
    case 1: // long
      r(5, 1, 6, 1, L.hair);
      r(4, 2, 8, 2, L.hair);
      r(3, 3, 1, 8, L.hair);
      r(12, 3, 1, 8, L.hair);
      r(4, 4, 1, 5, L.hair);
      r(11, 4, 1, 5, L.hair);
      r(6, 2, 3, 1, L.hairHi);
      break;
    case 2: // bun
      r(6, 0, 4, 1, L.hair);
      r(5, 1, 6, 1, L.hair);
      r(4, 2, 8, 2, L.hair);
      r(4, 4, 1, 2, L.hair);
      r(11, 4, 1, 2, L.hair);
      r(7, 0, 2, 1, L.hairHi);
      break;
    case 3: // curly
      r(4, 0, 8, 1, L.hair);
      r(3, 1, 10, 3, L.hair);
      r(3, 4, 1, 3, L.hair);
      r(12, 4, 1, 3, L.hair);
      r(5, 1, 1, 1, L.hairHi);
      r(8, 2, 1, 1, L.hairHi);
      r(10, 1, 1, 1, L.hairHi);
      break;
    default: // short with a fringe
      r(5, 1, 6, 1, L.hair);
      r(4, 2, 8, 2, L.hair);
      r(4, 4, 1, 2, L.hair);
      r(11, 4, 1, 2, L.hair);
      r(5, 4, 3, 1, L.hair);
      r(6, 2, 3, 1, L.hairHi);
  }
}

function back(g: Ctx, L: Look, step: number, sit: number, dy = 0) {
  const r = (x: number, y: number, w: number, h: number, c: string) => rect(g, x + 1, y + 1 + dy, w, h, c);
  if (sit < 0) {
    const lf = step === 1 ? -1 : 0;
    const rf = step === -1 ? -1 : 0;
    r(5, 18, 2, 5 + lf, L.pants);
    r(9, 18, 2, 5 + rf, L.pants);
    r(5, 23 + lf, 2, 1, SHOE);
    r(9, 23 + rf, 2, 1, SHOE);
  }
  r(4, 10, 8, 8, L.shirt);
  r(4, 11, 2, 6, L.shirtLo);
  r(4, 17, 8, 1, L.pantsLo);
  if (sit >= 0) {
    // Forearms reach forward to the keyboard; alternate hands while typing.
    r(3, 11 + (sit === 1 ? 0 : 1), 1, 3, L.shirtLo);
    r(12, 11 + (sit === 1 ? 1 : 0), 1, 3, L.shirtLo);
  } else {
    r(3, 11, 1, 5 + (step === -1 ? -1 : 0), L.shirtLo);
    r(3, 16 + (step === -1 ? -1 : 0), 1, 1, L.skin);
    r(12, 11, 1, 5 + (step === 1 ? -1 : 0), L.shirtLo);
    r(12, 16 + (step === 1 ? -1 : 0), 1, 1, L.skin);
  }
  r(6, 9, 4, 1, L.skinLo);
  r(3, 5, 1, 2, L.skin);
  r(12, 5, 1, 2, L.skin);
  r(4, 3, 8, 5, L.hair);
  r(5, 2, 6, 1, L.hair);
  r(5, 8, 6, 1, L.hair);
  r(5, 1, 6, 1, L.hair);
  r(6, 3, 4, 1, L.hairHi);
  if (L.style === 1) r(4, 9, 8, 2, L.hair);
  if (L.style === 2) r(6, 0, 4, 2, L.hair);
  if (L.style === 3) {
    r(3, 1, 10, 4, L.hair);
    r(4, 0, 8, 1, L.hair);
  }
}

function side(g: Ctx, L: Look, step: number) {
  const r = (x: number, y: number, w: number, h: number, c: string) => rect(g, x + 1, y + 1, w, h, c);
  if (step === 0) {
    r(6, 18, 4, 5, L.pants);
    r(6, 23, 5, 1, SHOE);
  } else {
    const a = step === 1;
    r(a ? 4 : 8, 18, 2, 5, L.pants);
    r(a ? 3 : 8, 23, 3, 1, SHOE);
    r(a ? 8 : 4, 18, 2, 5, L.pantsLo);
    r(a ? 8 : 3, 23, 3, 1, SHOE);
  }
  r(5, 10, 6, 8, L.shirt);
  r(5, 17, 6, 1, L.pantsLo);
  const ax = step === 1 ? 8 : step === -1 ? 6 : 7;
  r(ax, 11, 2, 5, L.shirtLo);
  r(ax, 16, 2, 1, L.skin);
  r(7, 9, 3, 1, L.skinLo);
  r(5, 3, 7, 5, L.skin);
  r(6, 2, 5, 1, L.skin);
  r(6, 8, 5, 1, L.skin);
  r(12, 6, 1, 1, L.skin);
  r(10, 5, 1, 1, EYE);
  r(7, 6, 1, 1, L.skinLo);
  r(6, 1, 5, 1, L.hair);
  r(5, 2, 7, 2, L.hair);
  r(5, 4, 2, 3, L.hair);
  r(7, 2, 3, 1, L.hairHi);
  if (L.style === 1) r(4, 3, 3, 8, L.hair);
  if (L.style === 2) r(4, 1, 2, 3, L.hair);
  if (L.style === 3) {
    r(4, 0, 8, 2, L.hair);
    r(4, 2, 3, 5, L.hair);
  }
}

function frame(draw: (g: Ctx) => void, mirror = false): HTMLCanvasElement {
  const [c, g] = canvas(18, 26);
  draw(g);
  let img = c;
  if (mirror) {
    const [m, mg] = canvas(18, 26);
    mg.translate(18, 0);
    mg.scale(-1, 1);
    mg.drawImage(c, 0, 0);
    img = m;
  }
  return outlined({ img, up: 0 }).img;
}

/** Every pose a person needs: four directions with a three-step walk, typing, and waving. */
export function personFrames(L: Look): Frames {
  const f: Frames = {};
  for (const [k, step] of [
    ["0", 0],
    ["1", 1],
    ["2", -1],
  ] as const) {
    f[`down-${k}`] = frame((g) => front(g, L, step, -1));
    f[`up-${k}`] = frame((g) => back(g, L, step, -1));
    f[`right-${k}`] = frame((g) => side(g, L, step));
    f[`left-${k}`] = frame((g) => side(g, L, step), true);
  }
  f["sit-0"] = frame((g) => back(g, L, 0, 0, 2));
  f["sit-1"] = frame((g) => back(g, L, 0, 1, 2));
  f["wave-0"] = frame((g) => front(g, L, 0, 0));
  f["wave-1"] = frame((g) => front(g, L, 0, 1));
  return f;
}

// ---- golden retrievers ------------------------------------------------------------------------

const FUR = ["#f5d49c", "#ecb86c", "#e0a25a", "#cf8a45", "#f2c682"];
const COLLAR = ["#ff4fa3", "#4dd0e1", "#c77dff", "#ffd23f", "#ff8fab", "#ffffff", "#ef5d9a", "#7ed9a0", "#ff9a5c"];
const NOSE = "#2a1f26";
const TONGUE = "#ff7aa8";
const BOW = "#ff3d9a";
const TAG = "#ffd23f";

interface Dog {
  fur: string;
  furHi: string;
  furLo: string;
  ear: string;
  collar: string;
  bow: boolean;
}

/** A dog's coat and collar come from the same name as the person it stands in for. */
function dogLook(L: Look): Dog {
  const fur = FUR[Math.max(0, SKIN.indexOf(L.skin)) % FUR.length];
  return {
    fur,
    furHi: shade(fur, 0.32),
    furLo: shade(fur, -0.16),
    ear: shade(fur, -0.24),
    collar: COLLAR[Math.max(0, SHIRT.indexOf(L.shirt)) % COLLAR.length],
    bow: L.style === 2,
  };
}

function dogFront(g: Ctx, L: Look, step: number, wave: number, dy = 0) {
  const r = (x: number, y: number, w: number, h: number, c: string) => rect(g, x + 1, y + 1 + dy, w, h, c);
  const D = dogLook(L);
  const lf = wave < 0 && step === 1 ? -1 : 0;
  const rf = wave < 0 && step === -1 ? -1 : 0;
  // Tail, wagging behind.
  const tw = step === 1 ? -1 : step === -1 ? 1 : 0;
  r(12, 14 + tw, 2, 2, D.furLo);
  r(13, 13 + tw, 2, 1, D.fur);
  // Hind legs and paws.
  r(5, 18, 2, 5 + lf, D.fur);
  r(9, 18, 2, 5 + rf, D.fur);
  r(5, 23 + lf, 2, 1, D.furHi);
  r(9, 23 + rf, 2, 1, D.furHi);
  // Body with a pale chest.
  r(4, 11, 8, 7, D.fur);
  r(6, 12, 4, 5, D.furHi);
  r(10, 12, 2, 5, D.furLo);
  r(5, 9, 6, 2, D.fur);
  r(4, 10, 8, 1, D.collar);
  r(7, 11, 2, 1, TAG);
  // Front paws. A wave lifts the right one.
  const la = step === 1 ? -1 : 0;
  r(3, 12, 1, 4 + la, D.furLo);
  r(3, 16 + la, 1, 1, D.furHi);
  if (wave === 0) {
    r(12, 6, 1, 5, D.furLo);
    r(12, 5, 1, 1, D.furHi);
  } else if (wave === 1) {
    r(12, 9, 1, 2, D.furLo);
    r(13, 6, 1, 3, D.furLo);
    r(13, 5, 1, 1, D.furHi);
  } else {
    const ra = step === -1 ? -1 : 0;
    r(12, 12, 1, 4 + ra, D.furLo);
    r(12, 16 + ra, 1, 1, D.furHi);
  }
  // Head, floppy ears, a happy muzzle.
  r(4, 2, 8, 7, D.fur);
  r(5, 1, 6, 1, D.fur);
  r(6, 1, 4, 1, D.furHi);
  r(2, 3, 2, 6, D.ear);
  r(12, 3, 2, 6, D.ear);
  r(2, 9, 1, 1, D.ear);
  r(13, 9, 1, 1, D.ear);
  r(6, 4, 1, 1, EYE);
  r(9, 4, 1, 1, EYE);
  r(6, 6, 4, 3, D.furHi);
  r(7, 6, 2, 1, NOSE);
  if (wave >= 0 || step === 0) r(8, 9, 1, 1, TONGUE);
  if (D.bow) {
    r(5, 0, 2, 2, BOW);
    r(9, 0, 2, 2, BOW);
    r(7, 0, 2, 1, shade(BOW, -0.25));
  }
}

function dogBack(g: Ctx, L: Look, step: number, sit: number, dy = 0) {
  const r = (x: number, y: number, w: number, h: number, c: string) => rect(g, x + 1, y + 1 + dy, w, h, c);
  const D = dogLook(L);
  if (sit < 0) {
    const lf = step === 1 ? -1 : 0;
    const rf = step === -1 ? -1 : 0;
    r(5, 18, 2, 5 + lf, D.fur);
    r(9, 18, 2, 5 + rf, D.fur);
    r(5, 23 + lf, 2, 1, D.furHi);
    r(9, 23 + rf, 2, 1, D.furHi);
    r(7 + step, 17, 2, 4, D.furHi);
  } else {
    // Typing at the desk, tail wagging from side to side.
    r(sit === 1 ? 9 : 4, 17, 3, 2, D.furHi);
    r(sit === 1 ? 11 : 3, 16, 2, 1, D.fur);
  }
  r(4, 10, 8, 8, D.fur);
  r(4, 11, 2, 6, D.furLo);
  r(4, 9, 8, 1, D.collar);
  if (sit >= 0) {
    r(3, 11 + (sit === 1 ? 0 : 1), 1, 3, D.furLo);
    r(12, 11 + (sit === 1 ? 1 : 0), 1, 3, D.furLo);
  } else {
    r(3, 11, 1, 5 + (step === -1 ? -1 : 0), D.furLo);
    r(3, 16 + (step === -1 ? -1 : 0), 1, 1, D.furHi);
    r(12, 11, 1, 5 + (step === 1 ? -1 : 0), D.furLo);
    r(12, 16 + (step === 1 ? -1 : 0), 1, 1, D.furHi);
  }
  r(4, 2, 8, 7, D.fur);
  r(5, 1, 6, 1, D.fur);
  r(6, 2, 4, 1, D.furHi);
  r(2, 3, 2, 6, D.ear);
  r(12, 3, 2, 6, D.ear);
  if (D.bow) {
    r(5, 0, 2, 2, BOW);
    r(9, 0, 2, 2, BOW);
    r(7, 0, 2, 1, shade(BOW, -0.25));
  }
}

function dogSide(g: Ctx, L: Look, step: number) {
  const r = (x: number, y: number, w: number, h: number, c: string) => rect(g, x + 1, y + 1, w, h, c);
  const D = dogLook(L);
  // Tail out behind, wagging as it walks.
  const tw = step === 1 ? -1 : step === -1 ? 1 : 0;
  r(2, 12 + tw, 3, 2, D.fur);
  r(1, 11 + tw, 2, 1, D.furHi);
  if (step === 0) {
    r(6, 18, 4, 5, D.fur);
    r(6, 23, 5, 1, D.furHi);
  } else {
    const a = step === 1;
    r(a ? 4 : 8, 18, 2, 5, D.fur);
    r(a ? 3 : 8, 23, 3, 1, D.furHi);
    r(a ? 8 : 4, 18, 2, 5, D.furLo);
    r(a ? 8 : 3, 23, 3, 1, D.furHi);
  }
  r(5, 10, 6, 8, D.fur);
  r(9, 11, 2, 5, D.furHi);
  r(5, 9, 6, 1, D.collar);
  const ax = step === 1 ? 8 : step === -1 ? 6 : 7;
  r(ax, 11, 2, 5, D.furLo);
  r(ax, 16, 2, 1, D.furHi);
  r(5, 2, 7, 6, D.fur);
  r(6, 1, 5, 1, D.fur);
  r(7, 1, 3, 1, D.furHi);
  r(12, 5, 3, 3, D.furHi);
  r(14, 5, 1, 1, NOSE);
  if (step === 0) r(13, 8, 1, 1, TONGUE);
  r(10, 4, 1, 1, EYE);
  r(5, 3, 3, 6, D.ear);
  if (D.bow) {
    r(7, 0, 2, 2, BOW);
    r(9, 0, 1, 1, shade(BOW, -0.25));
  }
}

/** The same poses as a person, as a golden retriever. */
function dogFrames(L: Look): Frames {
  const f: Frames = {};
  for (const [k, step] of [
    ["0", 0],
    ["1", 1],
    ["2", -1],
  ] as const) {
    f[`down-${k}`] = frame((g) => dogFront(g, L, step, -1));
    f[`up-${k}`] = frame((g) => dogBack(g, L, step, -1));
    f[`right-${k}`] = frame((g) => dogSide(g, L, step));
    f[`left-${k}`] = frame((g) => dogSide(g, L, step), true);
  }
  f["sit-0"] = frame((g) => dogBack(g, L, 0, 0, 2));
  f["sit-1"] = frame((g) => dogBack(g, L, 0, 1, 2));
  f["wave-0"] = frame((g) => dogFront(g, L, 0, 0));
  f["wave-1"] = frame((g) => dogFront(g, L, 0, 1));
  return f;
}

/** Everyone in the office, in the current theme: people, or golden retrievers. */
export function figureFrames(L: Look): Frames {
  return theme === "summer" ? dogFrames(L) : personFrames(L);
}
