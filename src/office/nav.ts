// A walkability grid over the office floor (in tiles), used for collision and click-to-walk paths.

export const CELL = 0.25;

export class NavGrid {
  readonly w: number;
  readonly h: number;
  private blocked: Uint8Array;

  constructor(
    readonly minX: number,
    readonly minZ: number,
    maxX: number,
    maxZ: number,
  ) {
    this.w = Math.ceil((maxX - minX) / CELL) + 1;
    this.h = Math.ceil((maxZ - minZ) / CELL) + 1;
    this.blocked = new Uint8Array(this.w * this.h);
  }

  private ix(x: number) {
    return Math.floor((x - this.minX) / CELL);
  }
  private iz(z: number) {
    return Math.floor((z - this.minZ) / CELL);
  }

  /** Blocks an axis-aligned rectangle, grown by `pad` metres for the walker's body. */
  block(x0: number, z0: number, x1: number, z1: number, pad = 0.22) {
    const a = this.ix(Math.min(x0, x1) - pad);
    const b = this.ix(Math.max(x0, x1) + pad);
    const c = this.iz(Math.min(z0, z1) - pad);
    const d = this.iz(Math.max(z0, z1) + pad);
    for (let i = Math.max(0, a); i <= Math.min(this.w - 1, b); i++)
      for (let j = Math.max(0, c); j <= Math.min(this.h - 1, d); j++) this.blocked[j * this.w + i] = 1;
  }

  walkable(x: number, z: number): boolean {
    const i = this.ix(x);
    const j = this.iz(z);
    if (i < 0 || j < 0 || i >= this.w || j >= this.h) return false;
    return this.blocked[j * this.w + i] === 0;
  }

  /** A* over 8-connected cells. Returns world-space waypoints, or null if unreachable. */
  path(fx: number, fz: number, tx: number, tz: number): { x: number; y: number }[] | null {
    const start = this.nearestOpen(this.ix(fx), this.iz(fz));
    const goal = this.nearestOpen(this.ix(tx), this.iz(tz));
    if (start < 0 || goal < 0) return null;
    const n = this.w * this.h;
    const g = new Float32Array(n).fill(Infinity);
    const from = new Int32Array(n).fill(-1);
    const closed = new Uint8Array(n);
    const heap = new MinHeap();
    const gx = goal % this.w;
    const gz = Math.floor(goal / this.w);
    const hfn = (k: number) => {
      const dx = Math.abs((k % this.w) - gx);
      const dz = Math.abs(Math.floor(k / this.w) - gz);
      return Math.max(dx, dz) + 0.414 * Math.min(dx, dz);
    };
    g[start] = 0;
    heap.push(start, hfn(start));
    while (heap.size) {
      const k = heap.pop();
      if (k === goal) break;
      if (closed[k]) continue;
      closed[k] = 1;
      const kx = k % this.w;
      const kz = Math.floor(k / this.w);
      for (let dz = -1; dz <= 1; dz++)
        for (let dx = -1; dx <= 1; dx++) {
          if (!dx && !dz) continue;
          const nx = kx + dx;
          const nz = kz + dz;
          if (nx < 0 || nz < 0 || nx >= this.w || nz >= this.h) continue;
          const m = nz * this.w + nx;
          if (this.blocked[m] || closed[m]) continue;
          // No corner cutting through walls.
          if (dx && dz && (this.blocked[kz * this.w + nx] || this.blocked[nz * this.w + kx])) continue;
          const cost = g[k] + (dx && dz ? 1.414 : 1);
          if (cost < g[m]) {
            g[m] = cost;
            from[m] = k;
            heap.push(m, cost + hfn(m));
          }
        }
    }
    if (from[goal] < 0 && goal !== start) return null;
    const cells: number[] = [];
    for (let k = goal; k !== -1; k = from[k]) cells.push(k);
    cells.reverse();
    const pts = cells.map((k) => ({
      x: this.minX + ((k % this.w) + 0.5) * CELL,
      y: this.minZ + (Math.floor(k / this.w) + 0.5) * CELL,
    }));
    pts[pts.length - 1] = { x: tx, y: tz };
    return this.smooth(pts);
  }

  private nearestOpen(i: number, j: number): number {
    for (let r = 0; r < 12; r++)
      for (let dj = -r; dj <= r; dj++)
        for (let di = -r; di <= r; di++) {
          const a = i + di;
          const b = j + dj;
          if (a >= 0 && b >= 0 && a < this.w && b < this.h && !this.blocked[b * this.w + a]) return b * this.w + a;
        }
    return -1;
  }

  /** Drops waypoints that have a clear straight line between their neighbours. */
  private smooth(pts: { x: number; y: number }[]) {
    if (pts.length < 3) return pts;
    const out = [pts[0]];
    let anchor = 0;
    for (let i = 2; i < pts.length; i++) {
      if (!this.clear(pts[anchor], pts[i])) {
        out.push(pts[i - 1]);
        anchor = i - 1;
      }
    }
    out.push(pts[pts.length - 1]);
    return out;
  }

  private clear(a: { x: number; y: number }, b: { x: number; y: number }) {
    const steps = Math.ceil(Math.hypot(b.x - a.x, b.y - a.y) / (CELL / 2));
    for (let s = 1; s < steps; s++) {
      const t = s / steps;
      if (!this.walkable(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)) return false;
    }
    return true;
  }
}

class MinHeap {
  private k: number[] = [];
  private p: number[] = [];
  get size() {
    return this.k.length;
  }
  push(key: number, pri: number) {
    this.k.push(key);
    this.p.push(pri);
    let i = this.k.length - 1;
    while (i > 0) {
      const up = (i - 1) >> 1;
      if (this.p[up] <= this.p[i]) break;
      this.swap(i, up);
      i = up;
    }
  }
  pop(): number {
    const top = this.k[0];
    const lk = this.k.pop()!;
    const lp = this.p.pop()!;
    if (this.k.length) {
      this.k[0] = lk;
      this.p[0] = lp;
      let i = 0;
      for (;;) {
        const l = 2 * i + 1;
        const r = l + 1;
        let m = i;
        if (l < this.k.length && this.p[l] < this.p[m]) m = l;
        if (r < this.k.length && this.p[r] < this.p[m]) m = r;
        if (m === i) break;
        this.swap(i, m);
        i = m;
      }
    }
    return top;
  }
  private swap(a: number, b: number) {
    [this.k[a], this.k[b]] = [this.k[b], this.k[a]];
    [this.p[a], this.p[b]] = [this.p[b], this.p[a]];
  }
}
