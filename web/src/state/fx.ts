// The FX racks as the UI sees them: which modules sit in each chain and in
// what order. Chain 0 is the Main rack, 1 and 2 the buses, and 3 + 3 ×
// splitter + band the splitters' band chains. Each module is one of four
// instances of its type; the store hands out free instances and sends
// every change to the engine (SetChain).

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';
import type { ParamBank } from './bank';
import { toNorm } from './param-math';

export const FX_TYPES = [
  { key: 'hyper', name: 'Hyper/Dimension', short: 'HYPER' },
  { key: 'distortion', name: 'Distortion', short: 'DIST' },
  { key: 'flanger', name: 'Flanger', short: 'FLANGE' },
  { key: 'phaser', name: 'Phaser', short: 'PHASER' },
  { key: 'chorus', name: 'Chorus', short: 'CHORUS' },
  { key: 'delay', name: 'Delay', short: 'DELAY' },
  { key: 'compressor', name: 'Compressor', short: 'COMP' },
  { key: 'reverb', name: 'Reverb', short: 'REVERB' },
  { key: 'eq', name: 'EQ', short: 'EQ' },
  { key: 'filter', name: 'Filter', short: 'FILTER' },
  { key: 'bode', name: 'Frequency Shifter', short: 'SHIFT' },
  { key: 'convolve', name: 'Convolve', short: 'CONV' },
  { key: 'utility', name: 'Utility', short: 'UTIL' },
  { key: 'splitter', name: 'Splitter', short: 'SPLIT' },
] as const;

export const INSTANCES = 4;
export const MAIN = 0;
export const BUS1 = 1;
export const BUS2 = 2;
export const CHAINS = 3 + INSTANCES * 3;
export const MAX_CHAIN = 16;
export const SPLITTER = 13;
export const CONVOLVE = 11;
export const RACK_NAMES = ['MAIN', 'BUS 1', 'BUS 2'];

export const bandChain = (inst: number, band: number) => 3 + inst * 3 + band;

export interface FxRef {
  type: number;
  inst: number;
}

export const refKey = (r: FxRef) => `fx.${FX_TYPES[r.type].key}.${r.inst + 1}`;
export const refParam = (r: FxRef, local: string) => `${refKey(r)}.${local}` as ParamKey;
export const refName = (r: FxRef) => `${FX_TYPES[r.type].name} ${r.inst + 1}`;

/** Every parameter of a module instance, in spec order. */
export function moduleParams(r: FxRef) {
  const prefix = `${refKey(r)}.`;
  return PARAMS.filter((p) => p.key.startsWith(prefix));
}

/** Factory module presets: plain values by local key. */
export const MODULE_PRESETS: Record<string, Record<string, Record<string, number>>> = {
  reverb: {
    'Small Room': { algo: 0, size: 0.2, decay: 0.6, predelay: 5, damp: 9000, mix: 0.2 },
    'Big Hall': { algo: 0, size: 0.8, decay: 4.5, predelay: 25, damp: 7000, mix: 0.3 },
    'Bright Plate': { algo: 1, size: 0.5, decay: 2.2, predelay: 0, damp: 16000, mix: 0.25 },
    'Bloom Pad': { algo: 3, size: 0.9, decay: 8, predelay: 40, movement: 0.6, mix: 0.4 },
    'Endless Basin': { algo: 4, size: 1, decay: 20, damp: 4000, mix: 0.45 },
  },
  delay: {
    Slapback: { mode: 0, bpm: 0, time_l: 90, link: 1, feedback: 0.1, mix: 0.25 },
    'Dotted Eighth': { mode: 0, bpm: 1, sync_l: 7, link: 1, feedback: 0.45, mix: 0.3 },
    'Ping-Pong Quarter': { mode: 1, bpm: 1, sync_l: 9, link: 1, feedback: 0.5, mix: 0.3 },
    'Dark Tape': { mode: 0, bpm: 0, time_l: 380, link: 1, feedback: 0.6, freq: 700, width: 0.3, mix: 0.3 },
  },
  distortion: {
    Warm: { mode: 0, drive: 0.25, mix: 1, output: -2 },
    Crunch: { mode: 14, drive: 0.55, filter: 2, filter_type: 0, freq: 6000, output: -6 },
    Fold: { mode: 6, drive: 0.6, mix: 0.8, output: -4 },
    'Lo-Fi': { mode: 8, drive: 0.5, mix: 1 },
  },
  compressor: {
    Glue: { mode: 0, threshold: -18, ratio: 2, attack: 30, release: 200, knee: 9, gain: 3 },
    Smash: { mode: 0, threshold: -30, ratio: 12, attack: 1, release: 80, gain: 12 },
    'OTT-ish': { mode: 1, threshold: -24, ratio: 6, depth: 0.8, upward: 0.8, downward: 0.8, time: 0.3, gain: 3 },
  },
  chorus: {
    Subtle: { rate: 0.3, delay1: 8, delay2: 13, depth: 0.2, mix: 0.3 },
    Wide: { rate: 0.6, delay1: 6, delay2: 20, depth: 0.5, mix: 0.5 },
  },
  eq: {
    'Low Cut': { low_type: 2, low_freq: 120, low_gain: 0 },
    Smile: { low_gain: 4, high_gain: 4, mid_gain: -2 },
    Telephone: { low_type: 2, low_freq: 400, high_type: 2, high_freq: 3000, mid_gain: 6, mid_freq: 1500 },
  },
};

export type ChainSink = (chain: number, refs: readonly FxRef[]) => void;

export class FxRacks {
  readonly chains: FxRef[][] = Array.from({ length: CHAINS }, () => []);
  private readonly subs = new Set<() => void>();
  private sink: ChainSink | null = null;

  constructor(private readonly bank: ParamBank) {}

  attach(sink: ChainSink | null): void {
    this.sink = sink;
  }

  resync(): void {
    this.chains.forEach((c, i) => (c.length || i < 3) && this.sink?.(i, c));
  }

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  private changed(chain: number): void {
    this.sink?.(chain, this.chains[chain]);
    for (const fn of this.subs) fn();
  }

  used(type: number, inst: number): boolean {
    return this.chains.some((c) => c.some((r) => r.type === type && r.inst === inst));
  }

  freeInstance(type: number): number {
    for (let i = 0; i < INSTANCES; i++) if (!this.used(type, i)) return i;
    return -1;
  }

  /** Every parameter of an instance back to its default (and switched on). */
  resetModule(r: FxRef): void {
    for (const p of moduleParams(r)) this.bank.set(p.id, p.def);
  }

  /** Add a fresh module of `type` at the end of `chain`. Null when none is free or it can't go there. */
  add(chain: number, type: number): FxRef | null {
    if (chain >= 3 && type === SPLITTER) return null;
    const c = this.chains[chain];
    if (!c || c.length >= MAX_CHAIN) return null;
    const inst = this.freeInstance(type);
    if (inst < 0) return null;
    const r = { type, inst };
    this.resetModule(r);
    c.push(r);
    this.changed(chain);
    return r;
  }

  remove(chain: number, index: number): void {
    const [r] = this.chains[chain].splice(index, 1);
    if (r?.type === SPLITTER) {
      for (let b = 0; b < 3; b++) {
        const bc = bandChain(r.inst, b);
        this.chains[bc] = [];
        this.sink?.(bc, []);
      }
    }
    this.changed(chain);
  }

  move(chain: number, from: number, to: number): void {
    const c = this.chains[chain];
    if (from === to || from < 0 || to < 0 || from >= c.length || to >= c.length) return;
    const [r] = c.splice(from, 1);
    c.splice(to, 0, r);
    this.changed(chain);
  }

  set(chain: number, refs: FxRef[]): void {
    this.chains[chain] = refs.slice(0, MAX_CHAIN);
    this.changed(chain);
  }

  /** Apply a module preset (plain values by local key) to an instance. */
  applyPreset(r: FxRef, values: Record<string, number>): void {
    this.resetModule(r);
    for (const [k, v] of Object.entries(values)) {
      const id = PARAM_ID[refParam(r, k)];
      if (id !== undefined) this.bank.set(id, toNorm(PARAMS[id], v));
    }
  }

  /** The racks and every used module's values, for saving. */
  snapshot(): { chains: FxRef[][]; params: Record<string, number> } {
    const params: Record<string, number> = {};
    for (const c of this.chains) for (const r of c) for (const p of moduleParams(r)) params[p.key] = this.bank.get(p.id);
    return { chains: this.chains.map((c) => c.map((r) => ({ ...r }))), params };
  }

  restore(s: { chains: FxRef[][]; params: Record<string, number> }): void {
    for (const [k, v] of Object.entries(s.params)) {
      const id = PARAM_ID[k as ParamKey];
      if (id !== undefined) this.bank.set(id, v);
    }
    s.chains.forEach((c, i) => i < CHAINS && this.set(i, c));
  }
}
