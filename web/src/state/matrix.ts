// The modulation matrix as the UI sees it: 64 slots, each adding a scaled,
// curved source (optionally times an aux source) to one parameter. The
// store holds what the engine holds and sends every change; `evaluate` is
// the reference model the engine's matrix is tested against.

import { CONST, SOURCES } from '../gen/protocol';

export type SourceName = (typeof SOURCES)[number];

/** Source index by name. */
export const SOURCE = Object.fromEntries(SOURCES.map((n, i) => [n, i])) as Record<SourceName, number>;

export interface ModSlot {
  source: number;
  /** A second source the first is multiplied by (0: none). */
  aux: number;
  /** Destination parameter id. */
  dest: number;
  /** -1..1 of the destination's range. */
  amount: number;
  /** Bends the source, -1..1: 0 is straight, positive pushes values down. */
  curve: number;
  /** Final scale, 0..1. */
  output: number;
  /** Read the source as -1..1 (else 0..1). */
  bipolar: boolean;
  bypass: boolean;
}

export const FLAG_BIPOLAR = 1;
export const FLAG_BYPASS = 2;

export function slotFlags(s: ModSlot): number {
  return (s.bipolar ? FLAG_BIPOLAR : 0) | (s.bypass ? FLAG_BYPASS : 0);
}

const AUDIO_SOURCES = new Set<string>(['Osc A', 'Osc B', 'Osc C', 'Sub', 'Noise', 'Filter 1', 'Filter 2']);

/** Sources that swing -1..1 by nature; the rest run 0..1. Mirrors the engine. */
export function bipolarSource(s: number): boolean {
  const n = SOURCES[s] as string;
  return n === 'Note' || n === 'Pitch Bend' || n.startsWith('LFO') || n === 'MPE X' || AUDIO_SOURCES.has(n);
}

/** A slot's curve applied to a source value (by magnitude for -1..1 values). */
export function bend(v: number, c: number): number {
  if (c === 0) return v;
  return Math.sign(v) * Math.abs(v) ** (2 ** (2 * c));
}

/** Normalized offset per destination for the source values `src` (the reference model). */
export function evaluate(slots: readonly (ModSlot | null)[], src: (source: number) => number): Map<number, number> {
  const out = new Map<number, number>();
  for (const s of slots) {
    if (!s || s.bypass || s.source === 0) continue;
    const raw = src(s.source);
    const bi = bipolarSource(s.source);
    const v = s.bipolar === bi ? raw : s.bipolar ? 2 * raw - 1 : 0.5 * (raw + 1);
    const aux = s.aux === 0 ? 1 : src(s.aux);
    out.set(s.dest, (out.get(s.dest) ?? 0) + s.amount * bend(v, s.curve) * aux * s.output);
  }
  return out;
}

export type SlotSink = (index: number, slot: ModSlot | null) => void;

export class ModMatrix {
  readonly slots: (ModSlot | null)[] = Array.from({ length: CONST.maxModSlots }, () => null);
  private readonly subs = new Set<() => void>();
  private readonly destSubs = new Map<number, Set<() => void>>();
  private sink: SlotSink | null = null;

  /** Where changes go (the engine host). */
  attach(sink: SlotSink | null): void {
    this.sink = sink;
  }

  /** Send every slot again (to an engine that restarted empty). */
  resync(): void {
    this.slots.forEach((s, i) => s && this.sink?.(i, s));
  }

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  /** Called when a slot aimed at `dest` changes (for that parameter's knob). */
  subscribeDest(dest: number, fn: () => void): () => void {
    const s = this.destSubs.get(dest) ?? new Set();
    this.destSubs.set(dest, s);
    s.add(fn);
    return () => s.delete(fn);
  }

  private changed(index: number, before: ModSlot | null): void {
    const after = this.slots[index];
    this.sink?.(index, after);
    for (const d of new Set([before?.dest, after?.dest])) {
      if (d !== undefined) for (const fn of this.destSubs.get(d) ?? []) fn();
    }
    for (const fn of this.subs) fn();
  }

  set(index: number, slot: ModSlot | null): void {
    if (index < 0 || index >= this.slots.length) return;
    const before = this.slots[index];
    this.slots[index] = slot ? { ...slot } : null;
    this.changed(index, before);
  }

  update(index: number, patch: Partial<ModSlot>): void {
    const s = this.slots[index];
    if (s) this.set(index, { ...s, ...patch });
  }

  /** Route `source` to `dest` in the first free slot (or return the slot already doing it). -1 when full. */
  add(source: number, dest: number, amount = 0.5): number {
    const existing = this.slots.findIndex((s) => s && s.source === source && s.dest === dest);
    if (existing >= 0) return existing;
    const free = this.slots.findIndex((s) => !s);
    if (free >= 0) this.set(free, { source, aux: 0, dest, amount, curve: 0, output: 1, bipolar: false, bypass: false });
    return free;
  }

  remove(index: number): void {
    this.set(index, null);
  }

  /** Move a slot to another position, shifting the ones between. */
  move(from: number, to: number): void {
    if (from === to || from < 0 || to < 0 || from >= this.slots.length || to >= this.slots.length) return;
    const moved = this.slots.splice(from, 1)[0];
    this.slots.splice(to, 0, moved);
    for (let i = Math.min(from, to); i <= Math.max(from, to); i++) this.sink?.(i, this.slots[i]);
    for (const fn of this.subs) fn();
  }

  clear(): void {
    this.slots.forEach((s, i) => s && this.set(i, null));
  }

  /** Slots aimed at a parameter, in slot order. */
  forDest(dest: number): number[] {
    const out: number[] = [];
    this.slots.forEach((s, i) => s?.dest === dest && out.push(i));
    return out;
  }

  get used(): number {
    return this.slots.filter(Boolean).length;
  }
}
