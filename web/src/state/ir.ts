// Impulse responses for the four convolvers: a factory response (built by
// the tools worker at the engine's sample rate) or a file the user dropped,
// prepared for the engine's partitioned convolution and uploaded. Kept, so
// a restarted engine gets them back.

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';
import { tools } from '../tools/client';
import type { ParamBank } from './bank';
import { toPlain } from './param-math';

export const IR_INSTANCES = 4;
/** The "User File" option of fx.convolve.*.ir. */
export const USER_IR = 12;

export interface IrSink {
  loadIr(inst: number, taps: number, prepared: Float32Array): void;
}

export interface UserIr {
  name: string;
  l: Float32Array;
  r: Float32Array;
  rate: number;
}

/** Linear resampling (the response is band-limited enough for impulse work). */
function resample(x: Float32Array, from: number, to: number): Float32Array {
  if (from === to) return x;
  const n = Math.max(1, Math.round((x.length * to) / from));
  const out = new Float32Array(n);
  const step = from / to;
  for (let i = 0; i < n; i++) {
    const p = i * step;
    const k = Math.floor(p);
    const t = p - k;
    const a = x[k] ?? 0;
    const b = x[k + 1] ?? 0;
    out[i] = a + (b - a) * t;
  }
  return out;
}

export class IrStore {
  private sink: IrSink | null = null;
  private rate = 48_000;
  private readonly user: (UserIr | null)[] = Array.from({ length: IR_INSTANCES }, () => null);
  private readonly loaded: ({ taps: number; data: Float32Array; key: string } | null)[] = Array.from({ length: IR_INSTANCES }, () => null);
  private readonly wanted: string[] = Array.from({ length: IR_INSTANCES }, () => '');
  private readonly subs = new Set<(inst: number) => void>();
  /** Which instances are in use (only those get built). */
  active: (inst: number) => boolean = () => true;

  constructor(private readonly bank: ParamBank) {
    for (let i = 0; i < IR_INSTANCES; i++) {
      bank.subscribe(PARAM_ID[`fx.convolve.${i + 1}.ir` as ParamKey], () => void this.load(i));
    }
  }

  attach(sink: IrSink | null, sampleRate: number): void {
    this.sink = sink;
    this.rate = sampleRate;
  }

  subscribe(fn: (inst: number) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  /** The current response's name (for display). */
  name(inst: number): string {
    const info = PARAMS[PARAM_ID[`fx.convolve.${inst + 1}.ir` as ParamKey]];
    const v = toPlain(info, this.bank.get(info.id));
    if (v === USER_IR) return this.user[inst]?.name ?? 'No file';
    return info.curve.kind === 'enum' ? info.curve.options[v] : '';
  }

  /** The response as loaded, for the waveform view. */
  response(inst: number): Float32Array | null {
    return this.user[inst] && this.choice(inst) === USER_IR ? this.user[inst]!.l : null;
  }

  /** The file convolver `inst` plays, if it plays one. */
  userIr(inst: number): UserIr | null {
    return this.choice(inst) === USER_IR ? this.user[inst] : null;
  }

  private choice(inst: number): number {
    const info = PARAMS[PARAM_ID[`fx.convolve.${inst + 1}.ir` as ParamKey]];
    return toPlain(info, this.bank.get(info.id));
  }

  resync(): void {
    for (let i = 0; i < IR_INSTANCES; i++) {
      const l = this.loaded[i];
      if (l) this.sink?.loadIr(i, l.taps, l.data);
      else void this.load(i);
    }
  }

  /** Use a dropped file (any channel count; the first two channels are used). */
  setUser(inst: number, name: string, channels: Float32Array[], rate: number): void {
    const l = channels[0] ?? new Float32Array(1);
    const r = channels[1] ?? l;
    this.user[inst] = { name, l, r, rate };
    this.wanted[inst] = '';
    void this.load(inst, true);
  }

  async load(inst: number, force = false): Promise<void> {
    if (!this.active(inst) && !force) return;
    const v = this.choice(inst);
    const key = v === USER_IR ? `user:${this.user[inst]?.name ?? ''}:${this.rate}` : `factory:${v}:${this.rate}`;
    if (!force && key === this.wanted[inst]) return;
    this.wanted[inst] = key;
    let l: Float32Array;
    let r: Float32Array;
    if (v === USER_IR) {
      const u = this.user[inst];
      if (!u) return;
      l = resample(u.l, u.rate, this.rate);
      r = resample(u.r, u.rate, this.rate);
    } else {
      ({ l, r } = await tools().call({ op: 'irFactory', index: v, sr: this.rate }));
    }
    const { data, taps } = await tools().call({ op: 'irPrepare', l, r }, [l.buffer, r.buffer]);
    if (this.wanted[inst] !== key) return; // superseded
    this.loaded[inst] = { taps, data, key };
    this.sink?.loadIr(inst, taps, data);
    for (const fn of this.subs) fn(inst);
  }
}
