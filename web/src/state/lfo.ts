// LFO shapes: each LFO has a drawn curve (Normal type) and an XY path (Path
// type). The defaults and the evaluation mirror the engine's (crates/engine/
// src/lfo.rs) so the editor draws exactly what plays.

export interface LfoPoint {
  x: number;
  y: number;
  /** Bends the segment that starts here: >0 eases in, <0 eases out. */
  c: number;
}

export type ShapeKind = 'curve' | 'path';

export const LFO_COUNT = 10;
export const MAX_POINTS = 64;

export function defaultCurve(): LfoPoint[] {
  return [
    { x: 0, y: 0, c: -0.3 },
    { x: 0.25, y: 1, c: 0.3 },
    { x: 0.5, y: 0, c: -0.3 },
    { x: 0.75, y: -1, c: 0.3 },
  ];
}

export function defaultPath(): LfoPoint[] {
  return Array.from({ length: 8 }, (_, i) => {
    const a = (2 * Math.PI * i) / 8;
    return { x: Math.fround(Math.cos(a)), y: Math.fround(Math.sin(a)), c: 0 };
  });
}

/** Shapes to start drawing from. */
export const SHAPE_PRESETS: Record<string, () => LfoPoint[]> = {
  Sine: defaultCurve,
  Triangle: () => [
    { x: 0, y: -1, c: 0 },
    { x: 0.5, y: 1, c: 0 },
  ],
  'Saw Up': () => [
    { x: 0, y: -1, c: 0 },
    { x: 0.999, y: 1, c: 0 },
  ],
  'Saw Down': () => [
    { x: 0, y: 1, c: 0 },
    { x: 0.999, y: -1, c: 0 },
  ],
  Square: () => [
    { x: 0, y: 1, c: 0 },
    { x: 0.499, y: 1, c: 0 },
    { x: 0.5, y: -1, c: 0 },
    { x: 0.999, y: -1, c: 0 },
  ],
  'Exp Decay': () => [
    { x: 0, y: 1, c: -0.6 },
    { x: 0.999, y: -1, c: 0 },
  ],
  Steps: () =>
    [1, 0.3, -0.4, 0.6].flatMap((y, i) => [
      { x: i / 4, y, c: 0 },
      { x: i / 4 + 0.249, y, c: 0 },
    ]),
};

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** Clamp and order points as the engine will (curves sort by x; paths keep their order). */
export function normalize(pts: readonly LfoPoint[], kind: ShapeKind): LfoPoint[] {
  const out = pts.slice(0, MAX_POINTS).map((p) => ({
    x: Math.fround(kind === 'path' ? clamp(p.x, -1, 1) : clamp(p.x, 0, 1)),
    y: Math.fround(clamp(p.y, -1, 1)),
    c: Math.fround(clamp(p.c, -1, 1)),
  }));
  if (!out.length) out.push({ x: 0, y: 0, c: 0 });
  if (kind === 'curve') out.sort((a, b) => a.x - b.x);
  return out;
}

/** Segment easing (the engine's lfo `bend`). */
export function ease(t: number, c: number): number {
  if (c > 0) return t ** (1 + 7 * c);
  if (c < 0) return 1 - (1 - t) ** (1 - 7 * c);
  return t;
}

/** A drawn curve's value at x in [0, 1). */
export function evalCurve(pts: readonly LfoPoint[], x: number): number {
  const n = pts.length;
  if (n === 1) return pts[0].y;
  let i = n - 1;
  for (let k = 0; k < n; k++) {
    if (pts[k].x > x) {
      i = k === 0 ? n - 1 : k - 1;
      break;
    }
  }
  const a = pts[i];
  const b = pts[(i + 1) % n];
  let bx = b.x;
  let xx = x;
  if (bx <= a.x) {
    bx += 1;
    if (xx < a.x) xx += 1;
  }
  const t = bx > a.x ? clamp((xx - a.x) / (bx - a.x), 0, 1) : 0;
  return a.y + (b.y - a.y) * ease(t, a.c);
}

/** Position on a closed path at t in [0, 1). */
export function evalPath(pts: readonly LfoPoint[], t: number): [number, number] {
  const n = pts.length;
  const x = t * n;
  const i = Math.min(Math.floor(x), n - 1);
  const u = ease(x - i, pts[i].c);
  const a = pts[i];
  const b = pts[(i + 1) % n];
  return [a.x + (b.x - a.x) * u, a.y + (b.y - a.y) * u];
}

export type ShapeSink = (lfo: number, kind: ShapeKind, pts: readonly LfoPoint[]) => void;

export class LfoShapes {
  readonly curves: LfoPoint[][] = Array.from({ length: LFO_COUNT }, () => normalize(defaultCurve(), 'curve'));
  readonly paths: LfoPoint[][] = Array.from({ length: LFO_COUNT }, () => normalize(defaultPath(), 'path'));
  private readonly subs = new Set<(lfo: number, kind: ShapeKind) => void>();
  private sink: ShapeSink | null = null;

  attach(sink: ShapeSink | null): void {
    this.sink = sink;
  }

  /** Send every shape that differs from the default (the engine starts with defaults). */
  resync(): void {
    const same = (a: LfoPoint[], b: LfoPoint[]) => a.length === b.length && a.every((p, i) => p.x === b[i].x && p.y === b[i].y && p.c === b[i].c);
    for (let i = 0; i < LFO_COUNT; i++) {
      if (!same(this.curves[i], normalize(defaultCurve(), 'curve'))) this.sink?.(i, 'curve', this.curves[i]);
      if (!same(this.paths[i], normalize(defaultPath(), 'path'))) this.sink?.(i, 'path', this.paths[i]);
    }
  }

  get(lfo: number, kind: ShapeKind): LfoPoint[] {
    return kind === 'path' ? this.paths[lfo] : this.curves[lfo];
  }

  set(lfo: number, kind: ShapeKind, pts: readonly LfoPoint[]): void {
    const n = normalize(pts, kind);
    if (kind === 'path') this.paths[lfo] = n;
    else this.curves[lfo] = n;
    this.sink?.(lfo, kind, n);
    for (const fn of this.subs) fn(lfo, kind);
  }

  subscribe(fn: (lfo: number, kind: ShapeKind) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }
}
