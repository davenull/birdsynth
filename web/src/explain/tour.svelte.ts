// Runs a tour: enters and leaves steps, keeps their timers, turns pages,
// and puts the listener's own sound back at the end.

import type { Synth } from '../synth';
import type { Patch, PatchMeta } from '../state/patch';
import { nav } from '../ui/nav.svelte';
import { TOURS, type Step, type StepCtx, type Tour } from './tours';

class TourRunner {
  // the definitions hold functions: keep them as they are, not deep-proxied
  tour = $state.raw<Tour | null>(null);
  index = $state(0);
  private synth: Synth | null = null;
  private timers: ReturnType<typeof setInterval>[] = [];
  private saved: { patch: Patch; assets: Map<string, ArrayBuffer>; id: string; meta: PatchMeta; dirty: boolean } | null = null;
  /** Whether this tour is holding the session (so it's released once, however the tour ends). */
  private held = false;
  /** Serialises step changes (a step's enter can be async). */
  private busy: Promise<void> = Promise.resolve();

  get step(): Step | null {
    return this.tour?.steps[this.index] ?? null;
  }

  start(synth: Synth, id: string): Promise<void> {
    return this.queue(async () => {
      const tour = TOURS.find((t) => t.id === id);
      if (!tour) throw new Error(`no tour ${id}`);
      if (this.tour) await this.finish();
      this.synth = synth;
      // the teaching patches aren't your sound: don't keep them as the session
      synth.holdSession(true);
      this.held = true;
      try {
        const { patch, assets } = await synth.savePatch();
        this.saved = { patch, assets, id: synth.presetId, meta: { ...synth.meta }, dirty: synth.history.dirty };
      } catch (e) {
        synth.holdSession(false);
        this.held = false;
        throw e;
      }
      this.tour = tour;
      this.index = 0;
      await this.enter();
    });
  }

  next(): Promise<void> {
    return this.go(1);
  }

  back(): Promise<void> {
    return this.go(-1);
  }

  exit(): Promise<void> {
    return this.queue(() => this.finish());
  }

  /** Wait for any step change in progress. */
  settled(): Promise<void> {
    return this.busy;
  }

  private go(d: number): Promise<void> {
    return this.queue(async () => {
      if (!this.tour) return;
      const to = this.index + d;
      if (to < 0) return;
      if (to >= this.tour.steps.length) return this.finish();
      this.leave();
      this.index = to;
      await this.enter();
    });
  }

  private queue(fn: () => Promise<void>): Promise<void> {
    this.busy = this.busy.then(fn, fn);
    return this.busy;
  }

  private async enter(): Promise<void> {
    const s = this.step;
    if (!s || !this.synth) return;
    if (s.page) nav.page = s.page;
    const cx: StepCtx = {
      every: (ms, fn) => void this.timers.push(setInterval(fn, ms)),
      after: (ms, fn) => void this.timers.push(setTimeout(fn, ms)),
    };
    await s.enter?.(this.synth, cx);
  }

  private leave(): void {
    for (const t of this.timers) clearInterval(t);
    this.timers = [];
    if (this.synth) this.step?.leave?.(this.synth);
  }

  private async finish(): Promise<void> {
    this.leave();
    const s = this.synth;
    this.tour = null;
    this.index = 0;
    if (!s) return;
    s.allNotesOff();
    s.setBandlimit(true);
    const saved = this.saved;
    this.saved = null;
    try {
      if (saved) {
        await s.loadPatch(saved.patch, async (h) => saved.assets.get(h), saved.id);
        s.setMeta(saved.meta);
        if (saved.dirty) s.history.markDirty();
      }
    } finally {
      if (this.held) s.holdSession(false);
      this.held = false;
    }
  }
}

export const tours = new TourRunner();
