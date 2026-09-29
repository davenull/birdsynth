// Hybridize: a new patch from two, section by section. Each section (an
// oscillator with its wavetable or recording, a filter, an envelope, an LFO
// with its shape, the FX racks, the matrix, the macros...) comes whole from
// one parent or the other, picked by a seeded roll, so the result still
// hangs together; Blend then nudges the continuous values toward the other
// parent by a random share of that amount.

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';
import type { Patch } from './patch';

export interface HybridOptions {
  /** The roll: the same seed and parents give the same hybrid. */
  seed: number;
  /** 0..1: how far continuous values move toward the other parent (by a random share each). */
  blend?: number;
  /** The chance each section comes from `b` (0.5: even). */
  bias?: number;
}

/** The section a parameter belongs to: an instance ("osc.a", "lfo.3", "fx.delay.2") or a whole group ("sub", "voice"). */
export function sectionOf(key: string): string {
  const p = PARAMS[PARAM_ID[key as ParamKey]];
  if (!p) return key.split('.')[0];
  if (p.group === 'macro') return 'macro';
  // the effects and their racks travel together (a chain names instances by type and number)
  if (p.group.startsWith('fx.') || p.group === 'rack') return 'fx';
  return p.instance ? `${p.group}.${p.instance}` : p.group;
}

/** A small seeded generator (mulberry32). */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const CONTINUOUS = new Set(['lin', 'exp', 'pow', 'db']);
const OSCS = ['a', 'b', 'c'];

export function hybridize(a: Patch, b: Patch, opt: HybridOptions): Patch {
  const rand = rng(opt.seed);
  const blend = Math.max(0, Math.min(1, opt.blend ?? 0));
  const bias = opt.bias ?? 0.5;
  // every section either parent sets, and the matrix: which parent each comes from
  const sections = new Set<string>(['matrix']);
  for (const p of PARAMS) sections.add(sectionOf(p.key));
  const fromB = new Map<string, boolean>();
  for (const s of [...sections].sort()) fromB.set(s, rand() < bias);
  const pick = <T>(section: string, x: T, y: T): T => (fromB.get(section) ? y : x);

  const params: Record<string, number> = {};
  for (const p of PARAMS) {
    const va = a.params[p.key] ?? p.def;
    const vb = b.params[p.key] ?? p.def;
    const mine = fromB.get(sectionOf(p.key)) ? vb : va;
    const other = fromB.get(sectionOf(p.key)) ? va : vb;
    let v = mine;
    if (blend > 0 && CONTINUOUS.has(p.curve.kind) && other !== mine) v = mine + (other - mine) * blend * rand();
    if (v !== p.def) params[p.key] = v;
  }

  const o = (i: number) => `osc.${OSCS[i]}`;
  const lfo = (i: number) => `lfo.${i + 1}`;
  const name = `${a.meta.name} × ${b.meta.name}`;
  return {
    format: a.format,
    version: a.version,
    meta: {
      ...a.meta,
      name,
      author: '',
      rating: 0,
      tags: [...new Set([...a.meta.tags, ...b.meta.tags])],
      notes: `A hybrid of ${a.meta.name} and ${b.meta.name} (roll ${opt.seed}${blend ? `, blend ${Math.round(blend * 100)}%` : ''}).`,
    },
    params,
    matrix: structuredClone(pick('matrix', a.matrix, b.matrix)),
    lfo: {
      curves: Array.from({ length: Math.max(a.lfo.curves.length, b.lfo.curves.length) }, (_, i) => structuredClone(pick(lfo(i), a.lfo.curves[i] ?? null, b.lfo.curves[i] ?? null))),
      paths: Array.from({ length: Math.max(a.lfo.paths.length, b.lfo.paths.length) }, (_, i) => structuredClone(pick(lfo(i), a.lfo.paths[i] ?? null, b.lfo.paths[i] ?? null))),
    },
    remap: OSCS.map((_, i) => structuredClone(pick(o(i), a.remap[i] ?? null, b.remap[i] ?? null))),
    fx: structuredClone(pick('fx', a.fx, b.fx)),
    tables: OSCS.map((_, i) => structuredClone(pick(o(i), a.tables[i] ?? null, b.tables[i] ?? null))),
    irs: structuredClone(pick('fx', a.irs, b.irs)),
    recordings: OSCS.map((_, i) => structuredClone(pick(o(i), a.recordings[i] ?? null, b.recordings[i] ?? null))),
    multis: OSCS.map((_, i) => structuredClone(pick(o(i), a.multis[i] ?? null, b.multis[i] ?? null))),
    specFilter: OSCS.map((_, i) => structuredClone(pick(o(i), a.specFilter[i] ?? null, b.specFilter[i] ?? null))),
    arp: structuredClone(pick('arp', a.arp, b.arp)),
    clips: structuredClone(pick('clip', a.clips, b.clips)),
  };
}

/** Which parent each section came from (for showing the roll). */
export function hybridSections(opt: HybridOptions): Map<string, 'a' | 'b'> {
  const rand = rng(opt.seed);
  const sections = new Set<string>(['matrix']);
  for (const p of PARAMS) sections.add(sectionOf(p.key));
  const out = new Map<string, 'a' | 'b'>();
  for (const s of [...sections].sort()) out.set(s, rand() < (opt.bias ?? 0.5) ? 'b' : 'a');
  return out;
}
