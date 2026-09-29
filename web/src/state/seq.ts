// The sequencer's data: the arpeggiator's twelve step patterns and the
// twelve clips (notes, length and automation lanes). The engine plays them;
// these stores hold them for the UI, patches and a restarted engine.

import { PARAM_ID, PARAMS, type ParamKey } from '../gen/params';
import { CONST } from '../gen/protocol';
import type { Smf } from '../midi/smf';

export const STEPS = 16;
export const BANKS = 12;
export const LANES = 7;
export const CLIPS = 12;
export const CLIP_NOTES = 512;
export const CLIP_LANES = 4;
export const CLIP_POINTS = 64;
/** The longest clip, in beats (64 bars of 4/4). */
export const CLIP_MAX_BEATS = 256;

/** Lane order in a pattern (as the engine reads it). */
export const LANE = { on: 0, gate: 1, velocity: 2, chance: 3, bend: 4, strum: 5, degree: 6 } as const;
export type LaneName = keyof typeof LANE;

export interface ArpSink {
  setArpPattern(bank: number, values: Float32Array): void;
}

export function defaultPattern(): Float32Array {
  const p = new Float32Array(LANES * STEPS);
  for (let s = 0; s < STEPS; s++) {
    p[LANE.on * STEPS + s] = 1;
    p[LANE.gate * STEPS + s] = 0.5;
    p[LANE.velocity * STEPS + s] = 1;
    p[LANE.chance * STEPS + s] = 1;
    p[LANE.degree * STEPS + s] = s;
  }
  return p;
}

export class ArpPatterns {
  readonly banks: Float32Array[] = Array.from({ length: BANKS }, () => defaultPattern());
  private sink: ArpSink | null = null;
  private readonly subs = new Set<(bank: number) => void>();

  subscribe(fn: (bank: number) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  attach(sink: ArpSink | null): void {
    this.sink = sink;
  }

  resync(): void {
    this.banks.forEach((b, i) => this.sink?.setArpPattern(i, b));
  }

  get(bank: number, lane: LaneName, step: number): number {
    return this.banks[bank][LANE[lane] * STEPS + step];
  }

  set(bank: number, lane: LaneName, step: number, v: number): void {
    this.banks[bank][LANE[lane] * STEPS + step] = v;
    this.sink?.setArpPattern(bank, this.banks[bank]);
    for (const fn of this.subs) fn(bank);
  }

  load(bank: number, values: ArrayLike<number> | null): void {
    const p = defaultPattern();
    if (values) for (let i = 0; i < Math.min(p.length, values.length); i++) p[i] = values[i];
    this.banks[bank] = p;
    this.sink?.setArpPattern(bank, p);
    for (const fn of this.subs) fn(bank);
  }

  isDefault(bank: number): boolean {
    const d = defaultPattern();
    return this.banks[bank].every((v, i) => v === d[i]);
  }
}

export interface ClipNote {
  start: number;
  length: number;
  key: number;
  velocity: number;
  chance: number;
  bend: number;
}

export interface AutoLane {
  /** A parameter key, or null for an unused lane. */
  param: string | null;
  points: [number, number][];
}

export interface Clip {
  notes: ClipNote[];
  /** Beats. */
  length: number;
  lanes: AutoLane[];
}

export interface ClipSink {
  setClip(slot: number, notes: Float32Array, count: number, length: number): void;
  setClipLane(slot: number, lane: number, param: number, points: Float32Array, count: number): void;
}

export function emptyClip(): Clip {
  return { notes: [], length: 4, lanes: Array.from({ length: CLIP_LANES }, () => ({ param: null, points: [] })) };
}

export class ClipStore {
  readonly clips: Clip[] = Array.from({ length: CLIPS }, () => emptyClip());
  private sink: ClipSink | null = null;
  private readonly subs = new Set<(slot: number) => void>();

  subscribe(fn: (slot: number) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  attach(sink: ClipSink | null): void {
    this.sink = sink;
  }

  resync(): void {
    for (let s = 0; s < CLIPS; s++) this.send(s);
  }

  private send(slot: number): void {
    const c = this.clips[slot];
    const notes = c.notes.slice(0, CLIP_NOTES).sort((a, b) => a.start - b.start);
    const data = new Float32Array(notes.length * 6);
    notes.forEach((n, i) => data.set([n.start, n.length, n.key, n.velocity, n.chance, n.bend], i * 6));
    this.sink?.setClip(slot, data, notes.length, c.length);
    c.lanes.forEach((l, i) => {
      const id = l.param ? PARAM_ID[l.param as ParamKey] : undefined;
      const pts = l.points.slice(0, CLIP_POINTS).sort((a, b) => a[0] - b[0]);
      this.sink?.setClipLane(slot, i, id ?? 0xffff, Float32Array.from(pts.flat()), id === undefined ? 0 : pts.length);
    });
  }

  /** Change a clip (the function edits it in place), then send it. */
  edit(slot: number, fn: (c: Clip) => void): void {
    fn(this.clips[slot]);
    this.clips[slot].notes = this.clips[slot].notes.filter((n) => n.length > 0 && n.key >= 0 && n.key <= 127);
    this.send(slot);
    for (const f of this.subs) f(slot);
  }

  load(slot: number, c: Clip | null): void {
    this.clips[slot] = c
      ? {
          notes: c.notes.map((n) => ({ ...n })),
          length: c.length,
          lanes: Array.from({ length: CLIP_LANES }, (_, i) => ({ param: c.lanes?.[i]?.param ?? null, points: [...(c.lanes?.[i]?.points ?? [])].map((p) => [p[0], p[1]] as [number, number]) })),
        }
      : emptyClip();
    this.send(slot);
    for (const f of this.subs) f(slot);
  }

  /** Put a MIDI file's notes in a clip, its length rounded up to whole bars. */
  importSmf(slot: number, smf: Pick<Smf, 'notes' | 'length' | 'beatsPerBar'>): void {
    const bar = smf.beatsPerBar;
    this.edit(slot, (c) => {
      c.notes = smf.notes.slice(0, CLIP_NOTES).map((n) => ({ start: n.start, length: n.length, key: n.key, velocity: n.velocity, chance: 1, bend: 0 }));
      c.length = Math.min(CLIP_MAX_BEATS, Math.max(bar, Math.ceil(smf.length / bar) * bar));
    });
  }

  isEmpty(slot: number): boolean {
    const c = this.clips[slot];
    return !c.notes.length && c.length === 4 && c.lanes.every((l) => !l.param);
  }
}

/** A parameter's normalized value (for automation lanes' defaults). */
export function paramNorm(key: string, bank: { get(id: number): number }): number {
  const id = PARAM_ID[key as ParamKey];
  return id === undefined ? 0 : bank.get(id);
}

export const PARAM_KEYS = PARAMS.filter((p) => (p.flags & 1) !== 0).map((p) => p.key);
export const MAX_FRAMES = CONST.maxFrames;
