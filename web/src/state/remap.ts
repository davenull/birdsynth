// Each oscillator's drawn remap curve (used by the Remap 1-4 warps). Points
// run x 0..1 left to right with y 0..1; the engine gets a 257-point lookup
// table (SetOscCurve), and the wavetable preview uses the same table.

import { CONST } from '../gen/protocol';
import { ease, type LfoPoint } from './lfo';

export const REMAP_POINTS = 257;

export function identityCurve(): LfoPoint[] {
  return [
    { x: 0, y: 0, c: 0 },
    { x: 1, y: 1, c: 0 },
  ];
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** The lookup table for a curve: 257 values over x = 0..1 (ends hold flat). */
export function curveLut(pts: readonly LfoPoint[]): Float32Array {
  const p = [...pts].sort((a, b) => a.x - b.x);
  const out = new Float32Array(REMAP_POINTS);
  for (let i = 0; i < REMAP_POINTS; i++) {
    const x = i / (REMAP_POINTS - 1);
    let y: number;
    if (x <= p[0].x) y = p[0].y;
    else if (x >= p[p.length - 1].x) y = p[p.length - 1].y;
    else {
      let k = 0;
      while (k < p.length - 2 && p[k + 1].x < x) k++;
      const a = p[k];
      const b = p[k + 1];
      const t = b.x > a.x ? (x - a.x) / (b.x - a.x) : 0;
      y = a.y + (b.y - a.y) * ease(t, a.c);
    }
    out[i] = clamp(y, 0, 1);
  }
  return out;
}

export type CurveSink = (osc: number, lut: Float32Array) => void;

export class RemapCurves {
  readonly points: LfoPoint[][] = Array.from({ length: CONST.oscCount }, identityCurve);
  private readonly luts: Float32Array[] = this.points.map(curveLut);
  private readonly subs = new Set<(osc: number) => void>();
  private sink: CurveSink | null = null;

  attach(sink: CurveSink | null): void {
    this.sink = sink;
  }

  /** Send every curve that isn't the identity (the engine starts with identities). */
  resync(): void {
    this.points.forEach((p, o) => {
      const id = identityCurve();
      if (p.length !== 2 || p.some((q, i) => q.x !== id[i].x || q.y !== id[i].y || q.c !== 0)) this.sink?.(o, this.luts[o]);
    });
  }

  lut(osc: number): Float32Array {
    return this.luts[osc];
  }

  set(osc: number, pts: readonly LfoPoint[]): void {
    const clean = pts.slice(0, 64).map((p) => ({ x: Math.fround(clamp(p.x, 0, 1)), y: Math.fround(clamp(p.y, 0, 1)), c: Math.fround(clamp(p.c, -1, 1)) }));
    if (!clean.length) clean.push(...identityCurve());
    clean.sort((a, b) => a.x - b.x);
    this.points[osc] = clean;
    this.luts[osc] = curveLut(clean);
    this.sink?.(osc, this.luts[osc]);
    for (const fn of this.subs) fn(osc);
  }

  subscribe(fn: (osc: number) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }
}
