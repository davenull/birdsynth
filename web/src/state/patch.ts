// Patches: everything that makes a sound, as versioned JSON. Parameters are
// stored by key (numeric ids change between builds) and only where they
// differ from the default; structured state (matrix, LFO shapes, remap
// curves, FX racks) by name; wavetables and impulse responses from files by
// content hash, with the data kept alongside (the library's asset store, or
// embedded in a single-file export).

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';
import { SOURCES } from '../gen/protocol';
import type { ParamBank } from './bank';
import { CHAINS, FX_TYPES, type FxRacks, type FxRef } from './fx';
import { LFO_COUNT, defaultCurve, defaultPath, normalize, type LfoPoint, type LfoShapes } from './lfo';
import type { ModMatrix, ModSlot } from './matrix';
import { identityCurve, type RemapCurves } from './remap';
import type { ArpPatterns, ClipStore } from './seq';

export const PATCH_FORMAT = 'birdsynth-patch';
export const PATCH_VERSION = 3;

export interface PatchMeta {
  name: string;
  author: string;
  category: string;
  tags: string[];
  /** 0 (unrated) to 5. */
  rating: number;
  notes: string;
}

export interface MatrixRow {
  slot: number;
  source: string;
  aux: string;
  dest: string;
  amount: number;
  curve: number;
  output: number;
  bipolar: boolean;
  bypass: boolean;
}

/** A wavetable: factory ones by name; imported ones by the hash of their frames. */
export interface TableRef {
  name: string;
  source: string;
  count: number;
  hash?: string;
}

export interface IrRef {
  name: string;
  hash: string;
  rate: number;
}

/** A recording an oscillator plays (its audio an asset: the channels one after the other), or a picture for Spectral. */
export type RecordingRef =
  | { kind: 'audio'; name: string; hash: string; rate: number; channels: number; frames: number; slices: number[] }
  | { kind: 'picture'; name: string; hash: string; seconds: number };

/** A multisample: a factory one by name, or an SFZ's packed zones as an asset. */
export interface MultiRef {
  name: string;
  source: string;
  hash?: string;
  zones?: number;
}

export interface Patch {
  format: typeof PATCH_FORMAT;
  version: number;
  meta: PatchMeta;
  /** Normalized values, only where they differ from the default. */
  params: Record<string, number>;
  matrix: MatrixRow[];
  /** Per LFO: the drawn curve and XY path, or null for the default. */
  lfo: { curves: (LfoPoint[] | null)[]; paths: (LfoPoint[] | null)[] };
  /** Per oscillator: the remap curve, or null for the identity. */
  remap: (LfoPoint[] | null)[];
  fx: { chains: { type: string; inst: number }[][] };
  tables: (TableRef | null)[];
  irs: (IrRef | null)[];
  /** Per oscillator (version 2 on). */
  recordings: (RecordingRef | null)[];
  multis: (MultiRef | null)[];
  /** The Spectral type's drawn filter, or null when flat. */
  specFilter: (number[] | null)[];
  /** Version 3 on: the arpeggiator's twelve patterns (null: the default) and the twelve clips (null: empty). */
  arp: (number[] | null)[];
  clips: (PatchClip | null)[];
}

/** A clip in a patch: its automation lanes name parameters by key. */
export interface PatchClip {
  notes: { start: number; length: number; key: number; velocity: number; chance: number; bend: number }[];
  length: number;
  lanes: { param: string | null; points: [number, number][] }[];
}

/** The stores a patch is captured from and applied to (the Synth has them all). */
export interface PatchTarget {
  bank: ParamBank;
  matrix: ModMatrix;
  lfo: LfoShapes;
  remap: RemapCurves;
  fx: FxRacks;
  /** The sequencer's patterns and clips (optional for callers that don't have them). */
  arp?: ArpPatterns;
  clips?: ClipStore;
}

export function emptyMeta(name = 'Init'): PatchMeta {
  return { name, author: '', category: '', tags: [], rating: 0, notes: '' };
}

const samePoints = (a: readonly LfoPoint[], b: readonly LfoPoint[]) => a.length === b.length && a.every((p, i) => p.x === b[i].x && p.y === b[i].y && p.c === b[i].c);

/** The sound-making state of the stores (tables and IRs are added by the caller, who knows their data). */
export function capture(t: PatchTarget, meta: PatchMeta = emptyMeta()): Patch {
  const params: Record<string, number> = {};
  for (const p of PARAMS) {
    const v = t.bank.get(p.id);
    if (v !== Math.fround(p.def)) params[p.key] = v;
  }
  const matrix: MatrixRow[] = [];
  t.matrix.slots.forEach((s, slot) => {
    if (s) matrix.push({ slot, source: SOURCES[s.source], aux: SOURCES[s.aux], dest: PARAMS[s.dest].key, amount: s.amount, curve: s.curve, output: s.output, bipolar: s.bipolar, bypass: s.bypass });
  });
  const dc = normalize(defaultCurve(), 'curve');
  const dp = normalize(defaultPath(), 'path');
  const lfo = {
    curves: Array.from({ length: LFO_COUNT }, (_, i) => (samePoints(t.lfo.curves[i], dc) ? null : t.lfo.curves[i].map((p) => ({ ...p })))),
    paths: Array.from({ length: LFO_COUNT }, (_, i) => (samePoints(t.lfo.paths[i], dp) ? null : t.lfo.paths[i].map((p) => ({ ...p })))),
  };
  const id = identityCurve();
  const remap = t.remap.points.map((p) => (samePoints(p, id) ? null : p.map((q) => ({ ...q }))));
  const fx = { chains: t.fx.chains.map((c) => c.map((r) => ({ type: FX_TYPES[r.type].key as string, inst: r.inst }))) };
  return {
    format: PATCH_FORMAT,
    version: PATCH_VERSION,
    meta: { ...meta, tags: [...meta.tags] },
    params,
    matrix,
    lfo,
    remap,
    fx,
    tables: [null, null, null],
    irs: [null, null, null, null],
    recordings: [null, null, null],
    multis: [null, null, null],
    specFilter: [null, null, null],
    arp: Array.from({ length: 12 }, (_, b) => (t.arp && !t.arp.isDefault(b) ? Array.from(t.arp.banks[b]) : null)),
    clips: Array.from({ length: 12 }, (_, c) => {
      if (!t.clips || t.clips.isEmpty(c)) return null;
      const clip = t.clips.clips[c];
      return { notes: clip.notes.map((n) => ({ ...n })), length: clip.length, lanes: clip.lanes.map((l) => ({ param: l.param, points: l.points.map((p) => [p[0], p[1]] as [number, number]) })) };
    }),
  };
}

/** Put the stores in the patch's state (every parameter the patch doesn't set goes to its default). */
export function applyPatch(t: PatchTarget, patch: Patch): string[] {
  const warnings: string[] = [];
  for (const p of PARAMS) {
    const v = patch.params[p.key];
    t.bank.set(p.id, v === undefined ? p.def : v);
  }
  for (const k of Object.keys(patch.params)) if (!(k in PARAM_ID)) warnings.push(`unknown parameter ${k}`);
  t.matrix.clear();
  for (const r of patch.matrix) {
    const source = SOURCES.indexOf(r.source as (typeof SOURCES)[number]);
    const aux = SOURCES.indexOf(r.aux as (typeof SOURCES)[number]);
    const dest = PARAM_ID[r.dest as ParamKey];
    if (source < 0 || dest === undefined) {
      warnings.push(`matrix slot ${r.slot}: unknown ${source < 0 ? 'source ' + r.source : 'destination ' + r.dest}`);
      continue;
    }
    const slot: ModSlot = { source, aux: Math.max(0, aux), dest, amount: r.amount, curve: r.curve, output: r.output, bipolar: r.bipolar, bypass: r.bypass };
    t.matrix.set(r.slot, slot);
  }
  for (let i = 0; i < LFO_COUNT; i++) {
    t.lfo.set(i, 'curve', patch.lfo.curves[i] ?? defaultCurve());
    t.lfo.set(i, 'path', patch.lfo.paths[i] ?? defaultPath());
  }
  patch.remap.forEach((p, o) => t.remap.set(o, p ?? identityCurve()));
  for (let c = 0; c < CHAINS; c++) {
    const refs: FxRef[] = [];
    for (const r of patch.fx.chains[c] ?? []) {
      const type = FX_TYPES.findIndex((f) => f.key === r.type);
      if (type < 0) warnings.push(`unknown effect ${r.type}`);
      else refs.push({ type, inst: r.inst });
    }
    t.fx.set(c, refs);
  }
  if (t.arp) for (let b = 0; b < 12; b++) t.arp.load(b, patch.arp?.[b] ?? null);
  if (t.clips)
    for (let c = 0; c < 12; c++) {
      const clip = patch.clips?.[c] ?? null;
      for (const l of clip?.lanes ?? []) if (l.param && !(l.param in PARAM_ID)) warnings.push(`clip ${c + 1}: unknown automated parameter ${l.param}`);
      t.clips.load(c, clip);
    }
  return warnings;
}

// ------------------------------------------------------------- migrations

/**
 * Bring a patch from any earlier version to the current one. Version 0 is
 * the pre-release format of the first builds: each oscillator had an on/off
 * "filter" switch (now its Route, Filters or Main), and there was no FX,
 * LFO or remap state.
 */
export function migrate(raw: unknown): Patch {
  const p = raw as Record<string, unknown>;
  if (!p || typeof p !== 'object') throw new Error('not a patch');
  let version = typeof p.version === 'number' ? p.version : 0;
  let cur: Record<string, unknown> = { ...p };
  if (version === 0) {
    const params: Record<string, number> = { ...((cur.params as Record<string, number>) ?? {}) };
    for (const o of ['a', 'b', 'c']) {
      const k = `osc.${o}.filter`;
      if (k in params) {
        const route = PARAMS[PARAM_ID[`osc.${o}.route` as ParamKey]];
        // on: through the filters (option 0); off: straight to the main bus (option 1)
        params[`osc.${o}.route`] = params[k] >= 0.5 ? 0 : 1 / ((route.curve.kind === 'enum' ? route.curve.options.length : 2) - 1);
        delete params[k];
      }
    }
    cur = {
      format: PATCH_FORMAT,
      version: 1,
      meta: { ...emptyMeta(typeof cur.name === 'string' ? cur.name : 'Untitled') },
      params,
      matrix: (cur.matrix as MatrixRow[]) ?? [],
      lfo: { curves: Array(LFO_COUNT).fill(null), paths: Array(LFO_COUNT).fill(null) },
      remap: [null, null, null],
      fx: { chains: Array.from({ length: CHAINS }, () => []) },
      tables: (cur.tables as (TableRef | null)[]) ?? [null, null, null],
      irs: [null, null, null, null],
    };
    version = 1;
  }
  if (version === 1) {
    // version 2 added the oscillators' recordings, multisamples and spectral filters
    cur = { ...cur, version: 2, recordings: [null, null, null], multis: [null, null, null], specFilter: [null, null, null] };
    version = 2;
  }
  if (version === 2) {
    // version 3 added the arpeggiator's patterns and the clips
    cur = { ...cur, version: 3, arp: Array(12).fill(null), clips: Array(12).fill(null) };
    version = 3;
  }
  if (version > PATCH_VERSION) throw new Error(`this patch is from a newer birdsynth (format ${version})`);
  const out = cur as unknown as Patch;
  if (out.format !== PATCH_FORMAT) throw new Error('not a birdsynth patch');
  // fill in anything a hand-edited file left out
  out.meta = { ...emptyMeta(), ...out.meta };
  out.params ??= {};
  out.matrix ??= [];
  out.lfo ??= { curves: [], paths: [] };
  out.remap ??= [null, null, null];
  out.fx ??= { chains: [] };
  out.tables ??= [null, null, null];
  out.irs ??= [null, null, null, null];
  out.recordings ??= [null, null, null];
  out.multis ??= [null, null, null];
  out.specFilter ??= [null, null, null];
  out.arp ??= Array(12).fill(null);
  out.clips ??= Array(12).fill(null);
  return out;
}

// ------------------------------------------------------ assets and files

/** SHA-256 of some bytes, as hex. */
export async function hashBytes(data: ArrayBuffer | ArrayBufferView): Promise<string> {
  const bytes = data instanceof ArrayBuffer ? data : data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength);
  const d = await crypto.subtle.digest('SHA-256', bytes as ArrayBuffer);
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

function toBase64(buf: ArrayBuffer): string {
  const b = new Uint8Array(buf);
  let s = '';
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
}

function fromBase64(s: string): ArrayBuffer {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

/** A patch as one self-contained file: the JSON plus its assets. */
export function exportFile(patch: Patch, assets: Map<string, ArrayBuffer>): string {
  const embedded: Record<string, string> = {};
  for (const [h, data] of assets) embedded[h] = toBase64(data);
  return JSON.stringify({ ...patch, embedded }, null, 1);
}

export function importFile(text: string): { patch: Patch; assets: Map<string, ArrayBuffer> } {
  const raw = JSON.parse(text) as Record<string, unknown>;
  const assets = new Map<string, ArrayBuffer>();
  const embedded = (raw.embedded ?? {}) as Record<string, string>;
  for (const [h, b64] of Object.entries(embedded)) assets.set(h, fromBase64(b64));
  delete raw.embedded;
  return { patch: migrate(raw), assets };
}
