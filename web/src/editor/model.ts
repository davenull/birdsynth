// The wavetable editor's model: one oscillator's frames being edited, the
// current frame and the selection, undo and redo, and the link to the
// engine. Every edit is heard straight away: a changed frame is band-limited
// and sent on its own (the tools worker and UpdateFrame), and a change to
// the table's shape (frames added, removed, reordered, morphed, imported)
// replaces the table.

import { CONST } from '../gen/protocol';
import type { TableStore } from '../state/tables';
import { tools } from '../tools/client';

const FL = CONST.frameLen;
const MAX = CONST.maxFrames;
/** Undo keeps steps until they take more than this much memory. */
const UNDO_BYTES = 64 * 1024 * 1024;

type Step = { kind: 'frames'; at: number[]; data: Float32Array[] } | { kind: 'table'; frames: Float32Array; count: number; current: number; selected: number[] };

export type Scope = 'selection' | 'all';

/** The Process menu (the numbers are tools.wasm's; see crates/tools/src/wt/process.rs). */
export const PROCESS = {
  normalizeEach: 0,
  normalizeAll: 1,
  removeDc: 2,
  flipH: 3,
  flipV: 4,
  fadeIn: 5,
  fadeOut: 6,
  lowPass: 7,
  highPass: 8,
  downsample: 9,
  removeFundamental: 10,
  blurSpectrum: 11,
  clearAbove: 12,
  clearBelow: 13,
  randomizePhases: 14,
  octaveUp: 15,
  octaveDown: 16,
  oddOnly: 17,
  evenOnly: 18,
} as const;

export const MORPHS = ['Crossfade', 'Spectral', 'Spectral, fundamental phase zeroed', 'Spectral, all phases zeroed'] as const;

export const IMPORT_MODES = { constant: 0, dynamic: 1, dynamicSnap: 2, fft: 3 } as const;

export class WtEditor {
  osc = 0;
  name = '';
  frames = new Float32Array(FL);
  count = 1;
  current = 0;
  selected = new Set<number>([0]);
  /** Bumps on every change, for the views. */
  version = 0;
  busy = false;

  private readonly subs = new Set<() => void>();
  private undoSteps: Step[] = [];
  private redoSteps: Step[] = [];
  private stroke: Float32Array | null = null;
  // uploads: frames to send, and whether the whole table must go
  private dirty = new Set<number>();
  private whole = false;
  private sending = false;

  constructor(private readonly tables: TableStore) {}

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  private changed(): void {
    this.version++;
    for (const fn of this.subs) fn();
  }

  /** Start editing an oscillator's table. */
  open(osc: number): void {
    const t = this.tables.osc[osc];
    this.osc = osc;
    this.name = t?.name ?? 'Saw';
    this.count = t?.count ?? 1;
    this.frames = t ? t.frames.slice(0, this.count * FL) : Float32Array.from({ length: FL }, (_, i) => (2 * ((i / FL + 0.5) % 1)) - 1);
    this.current = 0;
    this.selected = new Set([0]);
    this.undoSteps = [];
    this.redoSteps = [];
    this.changed();
  }

  frame(i = this.current): Float32Array {
    return this.frames.subarray(i * FL, (i + 1) * FL);
  }

  /** The frames an operation on `scope` changes, in order. */
  targets(scope: Scope): number[] {
    if (scope === 'all') return [...Array(this.count).keys()];
    const s = [...this.selected].filter((i) => i < this.count).sort((a, b) => a - b);
    return s.length ? s : [this.current];
  }

  // ------------------------------------------------------------ selection
  select(i: number, mode: 'only' | 'toggle' | 'range' = 'only'): void {
    i = Math.max(0, Math.min(this.count - 1, i));
    if (mode === 'only') this.selected = new Set([i]);
    else if (mode === 'toggle') {
      const s = new Set(this.selected);
      if (s.has(i) && s.size > 1) s.delete(i);
      else s.add(i);
      this.selected = s;
    } else {
      const [a, b] = [Math.min(this.current, i), Math.max(this.current, i)];
      this.selected = new Set(Array.from({ length: b - a + 1 }, (_, k) => a + k));
    }
    this.current = i;
    this.changed();
  }

  /** Edit frame `i` without changing the selection. */
  focus(i: number): void {
    this.current = Math.max(0, Math.min(this.count - 1, i));
    this.changed();
  }

  selectAll(): void {
    this.selected = new Set(this.targets('all'));
    this.changed();
  }

  // ------------------------------------------------------------- undo
  private record(step: Step): void {
    this.undoSteps.push(step);
    this.redoSteps = [];
    let bytes = 0;
    for (let i = this.undoSteps.length - 1; i >= 0; i--) {
      const s = this.undoSteps[i];
      bytes += s.kind === 'table' ? s.frames.byteLength : s.data.reduce((a, d) => a + d.byteLength, 0);
      if (bytes > UNDO_BYTES) {
        this.undoSteps.splice(0, i + 1);
        break;
      }
    }
  }

  private snapFrames(at: number[]): Step {
    return { kind: 'frames', at, data: at.map((i) => this.frame(i).slice()) };
  }

  private snapTable(): Step {
    return { kind: 'table', frames: this.frames.slice(0, this.count * FL), count: this.count, current: this.current, selected: [...this.selected] };
  }

  get canUndo(): boolean {
    return this.undoSteps.length > 0;
  }

  get canRedo(): boolean {
    return this.redoSteps.length > 0;
  }

  undo(): void {
    this.swap(this.undoSteps, this.redoSteps);
  }

  redo(): void {
    this.swap(this.redoSteps, this.undoSteps);
  }

  private swap(from: Step[], to: Step[]): void {
    const s = from.pop();
    if (!s) return;
    if (s.kind === 'frames') {
      to.push(this.snapFrames(s.at));
      s.at.forEach((i, k) => this.frame(i).set(s.data[k]));
      this.touch(s.at);
    } else {
      to.push(this.snapTable());
      this.frames = s.frames.slice();
      this.count = s.count;
      this.current = Math.min(s.current, s.count - 1);
      this.selected = new Set(s.selected.filter((i) => i < s.count));
      this.touchAll();
    }
    this.changed();
  }

  // ----------------------------------------------------------- drawing
  /** A pen stroke begins: it's one undo step however long it runs. */
  beginStroke(): void {
    this.stroke = this.frame().slice();
  }

  /** Draw a straight segment in the current frame between two points (sample index, value). */
  segment(i0: number, v0: number, i1: number, v1: number): void {
    const f = this.frame();
    if (i1 < i0) [i0, v0, i1, v1] = [i1, v1, i0, v0];
    const a = Math.max(0, Math.round(i0));
    const b = Math.min(FL - 1, Math.round(i1));
    for (let i = a; i <= b; i++) {
      const t = b > a ? (i - a) / (b - a) : 0;
      f[i] = Math.max(-1, Math.min(1, v0 + (v1 - v0) * t));
    }
    this.touch([this.current]);
    this.changed();
  }

  endStroke(): void {
    if (this.stroke) {
      this.record({ kind: 'frames', at: [this.current], data: [this.stroke] });
      this.stroke = null;
    }
  }

  /** Write a frame without an undo step (inside a stroke: live changes while dragging). */
  writeFrame(data: Float32Array, i = this.current): void {
    this.frame(i).set(data.subarray(0, FL));
    this.touch([i]);
    this.changed();
  }

  /** Replace the current frame (from the harmonics view). */
  setFrame(data: Float32Array, i = this.current): void {
    this.record(this.snapFrames([i]));
    this.frame(i).set(data.subarray(0, FL));
    this.touch([i]);
    this.changed();
  }

  // -------------------------------------------------------- operations
  private async run(fn: () => Promise<void>): Promise<void> {
    this.busy = true;
    this.changed();
    try {
      await fn();
    } finally {
      this.busy = false;
      this.changed();
    }
  }

  /** Gather frames into one buffer, run `op` on it, put them back. */
  private async onFrames(at: number[], op: (buf: Float32Array) => Promise<Float32Array>): Promise<void> {
    const buf = new Float32Array(at.length * FL);
    at.forEach((i, k) => buf.set(this.frame(i), k * FL));
    const out = await op(buf);
    this.record(this.snapFrames(at));
    at.forEach((i, k) => this.frame(i).set(out.subarray(k * FL, (k + 1) * FL)));
    this.touch(at);
  }

  process(kind: number, scope: Scope, a = 0, b = 0): Promise<void> {
    const at = this.targets(scope);
    return this.run(() => this.onFrames(at, (frames) => tools().call({ op: 'wtProcess', kind, a, b, frames, count: at.length }, [frames.buffer])));
  }

  /** Run a formula; resolves to null, or the error and where it is. */
  async formula(src: string, scope: Scope, seed = 1): Promise<{ error: string; pos: number } | null> {
    const all = this.targets('all');
    const on = new Set(this.targets(scope));
    let fail: { error: string; pos: number } | null = null;
    await this.run(async () => {
      const frames = this.frames.slice(0, this.count * FL);
      const r = await tools().call({ op: 'formula', src, frames, count: this.count, apply: Uint8Array.from(all, (i) => (on.has(i) ? 1 : 0)), selected: Uint8Array.from(all, (i) => (this.selected.has(i) ? 1 : 0)), seed });
      if (r.error !== null) {
        fail = { error: r.error, pos: r.pos };
        return;
      }
      const at = [...on].sort((a, b) => a - b);
      this.record(this.snapFrames(at));
      for (const i of at) this.frame(i).set(r.frames.subarray(i * FL, (i + 1) * FL));
      this.touch(at);
    });
    return fail;
  }

  checkFormula(src: string): Promise<{ error: string | null; pos: number }> {
    return tools().call({ op: 'formulaCheck', src });
  }

  /** Swap the whole table for new frames (one undo step). */
  private replace(frames: Float32Array, count: number, current = 0): void {
    this.record(this.snapTable());
    this.count = Math.max(1, Math.min(MAX, count));
    this.frames = frames.slice(0, this.count * FL);
    this.current = Math.min(current, this.count - 1);
    this.selected = new Set([this.current]);
    this.touchAll();
  }

  /** Fill a new table of `target` frames from keyframes: the selection when it has two or more, else every frame. */
  morph(mode: number, target: number): Promise<void> {
    return this.run(async () => {
      const at = this.selected.size >= 2 ? this.targets('selection') : this.targets('all');
      const keys = new Float32Array(at.length * FL);
      at.forEach((i, k) => keys.set(this.frame(i), k * FL));
      const n = Math.max(at.length, Math.min(MAX, target));
      const out = await tools().call({ op: 'wtMorph', keys, count: at.length, target: n, mode }, [keys.buffer]);
      this.replace(out, out.length / FL);
    });
  }

  /** A pulse-width series from the current frame. */
  pwm(count: number): Promise<void> {
    return this.run(async () => {
      const out = await tools().call({ op: 'wtPwm', frame: this.frame().slice(), count: Math.min(MAX, count) });
      this.replace(out, out.length / FL);
    });
  }

  sortByBrightness(): Promise<void> {
    return this.run(async () => {
      const order = await tools().call({ op: 'wtSort', frames: this.frames.slice(0, this.count * FL), count: this.count });
      this.reorder([...order]);
    });
  }

  reverse(): void {
    this.reorder(this.targets('all').reverse());
    this.changed();
  }

  private reorder(order: number[]): void {
    const out = new Float32Array(this.count * FL);
    order.forEach((src, dst) => out.set(this.frame(src), dst * FL));
    const cur = order.indexOf(this.current);
    this.replace(out, this.count, Math.max(0, cur));
  }

  /** A copy of the current frame after it (or a silent one). */
  addFrame(copy = true): void {
    if (this.count >= MAX) return;
    const out = new Float32Array((this.count + 1) * FL);
    const at = this.current + 1;
    out.set(this.frames.subarray(0, at * FL));
    if (copy) out.set(this.frame(), at * FL);
    out.set(this.frames.subarray(at * FL, this.count * FL), (at + 1) * FL);
    this.replace(out, this.count + 1, at);
    this.changed();
  }

  /** Remove the selected frames (a table keeps at least one). */
  removeFrames(): void {
    const drop = new Set(this.targets('selection'));
    if (drop.size >= this.count) drop.delete(this.targets('selection')[0]);
    const keep = this.targets('all').filter((i) => !drop.has(i));
    const out = new Float32Array(keep.length * FL);
    keep.forEach((src, dst) => out.set(this.frame(src), dst * FL));
    const next = Math.min(keep.length - 1, Math.max(0, keep.findIndex((i) => i > this.current) - 1));
    this.replace(out, keep.length, next < 0 ? 0 : next);
    this.changed();
  }

  /** Move frame `from` to position `to`. */
  moveFrame(from: number, to: number): void {
    if (from === to || from < 0 || to < 0 || from >= this.count || to >= this.count) return;
    const order = this.targets('all');
    const [f] = order.splice(from, 1);
    order.splice(to, 0, f);
    this.reorder(order);
    this.current = to;
    this.selected = new Set([to]);
    this.changed();
  }

  /** Make the table from a recording (mono, at `sr`). Resolves with the pitch it found, if it looked for one. */
  importAudio(audio: Float32Array, sr: number, mode: number, arg: number, max: number = MAX): Promise<void> {
    return this.run(async () => {
      const r = await tools().call({ op: 'import', mode, audio, sr, arg, max });
      if (!r.count) throw new Error('nothing to import');
      this.replace(r.frames, r.count);
    });
  }

  pitch(audio: Float32Array, sr: number): Promise<number> {
    return tools().call({ op: 'pitch', audio, sr });
  }

  analyze(i = this.current): Promise<{ mag: Float32Array; phase: Float32Array }> {
    return tools().call({ op: 'wtAnalyze', frame: this.frame(i).slice() });
  }

  /** A frame built from harmonics (not written anywhere). */
  synthesizeFrame(mag: Float32Array, phase: Float32Array): Promise<Float32Array> {
    return tools().call({ op: 'wtSynthesize', mag, phase });
  }

  async setHarmonics(mag: Float32Array, phase: Float32Array, i = this.current): Promise<void> {
    const f = await tools().call({ op: 'wtSynthesize', mag, phase });
    this.setFrame(f, i);
  }

  // ------------------------------------------------------- the engine
  private touch(at: number[]): void {
    for (const i of at) this.dirty.add(i);
    void this.send();
  }

  private touchAll(): void {
    this.whole = true;
    this.dirty.clear();
    void this.send();
  }

  /** Send what changed; one request at a time, the newest state winning. */
  private async send(): Promise<void> {
    if (this.sending) return;
    this.sending = true;
    try {
      while (this.whole || this.dirty.size) {
        if (this.whole) {
          this.whole = false;
          this.dirty.clear();
          await this.tables.set(this.osc, { name: this.name, source: `edit:${this.name}`, frames: this.frames.slice(0, this.count * FL), count: this.count });
        } else {
          const at = [...this.dirty];
          this.dirty.clear();
          await Promise.all(at.map((i) => this.tables.setFrame(this.osc, i, this.frame(i))));
        }
      }
    } finally {
      this.sending = false;
    }
  }

  /** Resolves once everything changed so far has reached the engine. */
  async settled(): Promise<void> {
    while (this.sending || this.whole || this.dirty.size) await new Promise((r) => setTimeout(r, 2));
  }
}
