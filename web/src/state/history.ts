// Undo and redo. The sound-making state (parameters, matrix, LFO shapes,
// remap curves, FX racks) is snapshotted once a gesture settles, so a knob
// drag is one step, not hundreds; undo puts an older snapshot back through
// the same stores, so the engine and the UI follow.

import { applyPatch, capture, type Patch, type PatchTarget } from './patch';

export class History {
  private stack: string[] = [];
  private index = -1;
  /** The step the current preset was loaded or saved at (-1: gone from the stack). */
  private clean = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private applying = false;
  private readonly subs = new Set<() => void>();

  /**
   * @param settle ms of quiet that ends a gesture
   * @param limit steps kept
   */
  constructor(
    private readonly t: PatchTarget,
    private readonly settle = 400,
    private readonly limit = 200,
  ) {
    const touch = () => this.touch();
    t.bank.onAny(touch);
    t.matrix.subscribe(touch);
    t.lfo.subscribe(touch);
    t.remap.subscribe(touch);
    t.fx.subscribe(touch);
    t.arp?.subscribe(touch);
    t.clips?.subscribe(touch);
    this.reset();
  }

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  /** Start again from the current state (after a preset loads: undo doesn't cross presets). */
  reset(): void {
    this.cancel();
    this.stack = [this.snapshot()];
    this.index = 0;
    this.clean = 0;
    this.emit();
  }

  /** The current state is what the preset holds now (it was just saved). */
  markClean(): void {
    this.commit();
    this.clean = this.index;
    this.emit();
  }

  /** Whether the state differs from the preset as loaded or last saved. */
  get dirty(): boolean {
    return this.index !== this.clean || this.timer !== null;
  }

  /** Record the current state now, if it changed (the end of a gesture). */
  commit(): void {
    const pending = this.timer !== null;
    this.cancel();
    const s = this.snapshot();
    if (s === this.stack[this.index]) {
      if (pending) this.emit();
      return;
    }
    this.stack.length = this.index + 1;
    this.stack.push(s);
    if (this.stack.length > this.limit) {
      this.stack.shift();
      this.clean--;
    }
    this.index = this.stack.length - 1;
    this.emit();
  }

  get canUndo(): boolean {
    return this.index > 0 || this.timer !== null;
  }

  get canRedo(): boolean {
    return this.index < this.stack.length - 1 && this.timer === null;
  }

  /** Steps that can be undone (for display). */
  get depth(): number {
    return this.index;
  }

  undo(): void {
    this.commit();
    if (this.index <= 0) return;
    this.index--;
    this.restore();
  }

  redo(): void {
    this.commit();
    if (this.index >= this.stack.length - 1) return;
    this.index++;
    this.restore();
  }

  private snapshot(): string {
    const p: Partial<Patch> = capture(this.t);
    delete p.meta; // renaming a preset isn't an undo step
    return JSON.stringify(p);
  }

  private restore(): void {
    this.applying = true;
    try {
      applyPatch(this.t, { ...JSON.parse(this.stack[this.index]), meta: undefined } as Patch);
    } finally {
      this.applying = false;
    }
    this.emit();
  }

  private touch(): void {
    if (this.applying) return;
    const first = this.timer === null;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => this.commit(), this.settle);
    if (first) this.emit(); // undo becomes available straight away
  }

  private cancel(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private emit(): void {
    for (const fn of this.subs) fn();
  }
}
