// The parameter bank: every parameter's normalized value, in one
// Float32Array, with a subscription per parameter. Components subscribe to
// just the parameters they show, so a change repaints one knob, not the page.

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';

type Sub = (value: number) => void;
type AnySub = (id: number, value: number) => void;

export class ParamBank {
  readonly values = new Float32Array(PARAMS.length);
  private readonly subs: (Set<Sub> | undefined)[] = [];
  private readonly any = new Set<AnySub>();

  constructor() {
    for (const p of PARAMS) this.values[p.id] = p.def;
  }

  static id(key: ParamKey): number {
    return PARAM_ID[key];
  }

  get(id: number): number {
    return this.values[id];
  }

  set(id: number, value: number): void {
    const v = Math.fround(value < 0 ? 0 : value > 1 ? 1 : value);
    if (this.values[id] === v) return;
    this.values[id] = v;
    const s = this.subs[id];
    if (s) for (const fn of s) fn(v);
    for (const fn of this.any) fn(id, v);
  }

  subscribe(id: number, fn: Sub): () => void {
    const s = (this.subs[id] ??= new Set());
    s.add(fn);
    return () => s.delete(fn);
  }

  /** Called for every change, for the engine host and undo. */
  onAny(fn: AnySub): () => void {
    this.any.add(fn);
    return () => this.any.delete(fn);
  }
}
