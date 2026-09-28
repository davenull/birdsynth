// Shared by the patch tests and the factory level calibration: stores to
// apply patches to, the commands a Synth would send for them, and renders.

import { PARAMS, PARAM_ID, type ParamKey } from '../web/src/gen/params';
import type { CmdWriter } from '../web/src/gen/protocol';
import { ParamBank } from '../web/src/state/bank';
import { CONVOLVE, FxRacks } from '../web/src/state/fx';
import { LfoShapes } from '../web/src/state/lfo';
import { ModMatrix, slotFlags } from '../web/src/state/matrix';
import { toPlain } from '../web/src/state/param-math';
import type { Patch, PatchTarget } from '../web/src/state/patch';
import { RemapCurves } from '../web/src/state/remap';
import { Engine, loadIr, loadNoise, loadTable } from './engine/harness';

export function stores(): PatchTarget {
  const bank = new ParamBank();
  return { bank, matrix: new ModMatrix(), lfo: new LfoShapes(), remap: new RemapCurves(), fx: new FxRacks(bank) };
}

/** Everything the stores hold, as engine commands (what the Synth sends on a resync). */
export function send(t: PatchTarget, w: CmdWriter): void {
  t.bank.values.forEach((v, id) => w.setParam(0, id, v));
  w.clearMod(0);
  t.matrix.attach((i, s) => (s ? w.setModSlot(0, i, s.source, s.aux, slotFlags(s), s.dest, s.amount, s.curve, s.output) : w.setModSlot(0, i, 0, 0, 0, 0, 0, 0, 1)));
  t.lfo.attach((l, kind, pts) => w.setLfoShape(0, l, kind === 'path' ? 1 : 0, pts.length, pts.flatMap((p) => [p.x, p.y, p.c])));
  t.remap.attach((o, lut) => w.setOscCurve(0, o, lut.length, lut));
  t.fx.attach((c, refs) => w.setChain(0, c, refs.length, refs.map((r) => r.type * 256 + r.inst)));
  t.matrix.resync();
  t.lfo.resync();
  t.remap.resync();
  t.fx.resync();
}

/** Render a little phrase and hash the output bits. */
export function render(t: PatchTarget): string {
  const e = new Engine(48_000);
  send(t, e.w);
  e.render(512);
  e.w.noteOn(0, 57, 0, 0.8, 1);
  e.w.noteOn(0, 64, 0, 0.6, 2);
  const a = e.render(24_000);
  e.w.noteOff(0, 57, 0, 0, 1);
  const b = e.render(24_000);
  let h = 0x811c9dc5;
  for (const x of [a.l, a.r, b.l, b.r]) {
    const u = new Uint32Array(x.buffer, x.byteOffset, x.length);
    for (let i = 0; i < u.length; i++) h = Math.imul(h ^ u[i], 0x01000193) >>> 0;
  }
  return h.toString(16);
}

/** Give an engine the patch's factory tables, its noise and its convolvers' responses. */
export function assets(e: Engine, t: PatchTarget, p: Patch): void {
  p.tables.forEach((r, o) => r?.source.startsWith('factory:') && loadTable(e, o, r.source.slice(8)));
  const plain = (k: string) => toPlain(PARAMS[PARAM_ID[k as ParamKey]], t.bank.get(PARAM_ID[k as ParamKey]));
  loadNoise(e, plain('noise.type'));
  for (const c of t.fx.chains) for (const r of c) if (r.type === CONVOLVE) loadIr(e, r.inst, plain(`fx.convolve.${r.inst + 1}.ir`));
}

// ITU-R BS.1770 K-weighting at 48 kHz: a +4 dB high shelf, then a highpass near 38 Hz.
const K = [
  { b: [1.53512485958697, -2.69169618940638, 1.19839281085285], a: [-1.69065929318241, 0.73248077421585] },
  { b: [1.0, -2.0, 1.0], a: [-1.99004745483398, 0.99007225036621] },
];

function kWeight(x: Float32Array): Float64Array {
  let y = Float64Array.from(x);
  for (const { b, a } of K) {
    const out = new Float64Array(y.length);
    let x1 = 0, x2 = 0, y1 = 0, y2 = 0;
    for (let i = 0; i < y.length; i++) {
      const v = b[0] * y[i] + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
      x2 = x1; x1 = y[i]; y2 = y1; y1 = v;
      out[i] = v;
    }
    y = out;
  }
  return y;
}

/** The loudest 400 ms (BS.1770 momentary loudness, LUFS) of a stereo signal at 48 kHz, and its sample peak. */
export function loudness(l: Float32Array, r: Float32Array): { lufs: number; peak: number } {
  const kl = kWeight(l);
  const kr = kWeight(r);
  const win = 19_200;
  const hop = 4_800;
  let best = 1e-12;
  for (let s = 0; s + win <= kl.length; s += hop) {
    let e = 0;
    for (let i = s; i < s + win; i++) e += kl[i] * kl[i] + kr[i] * kr[i];
    best = Math.max(best, e / win);
  }
  let peak = 0;
  for (const x of [l, r]) for (let i = 0; i < x.length; i++) peak = Math.max(peak, Math.abs(x[i]));
  return { lufs: -0.691 + 10 * Math.log10(best), peak };
}

/**
 * How a factory preset is auditioned for its level: drums one hit, the
 * rest a held triad (the Riser longer, as it builds), then the release.
 */
export function audition(t: PatchTarget, p: Patch): { l: Float32Array; r: Float32Array } {
  const e = new Engine(48_000);
  send(t, e.w);
  assets(e, t, p);
  e.render(24_000); // let the smoothed parameters reach the patch's values
  const notes = p.meta.category === 'Drums' ? [60] : [48, 55, 60];
  const hold = p.meta.category === 'FX' ? 8 * 48_000 : 3 * 48_000;
  notes.forEach((n, k) => e.w.noteOn(0, n, 0, 0.8, k + 1));
  const a = e.render(hold);
  notes.forEach((n, k) => e.w.noteOff(0, n, 0, 0, k + 1));
  const b = e.render(48_000);
  const cat = (x: Float32Array, y: Float32Array) => {
    const o = new Float32Array(x.length + y.length);
    o.set(x);
    o.set(y, x.length);
    return o;
  };
  return { l: cat(a.l, b.l), r: cat(a.r, b.r) };
}
