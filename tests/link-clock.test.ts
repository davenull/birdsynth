// Measuring another instance's clock from ping round trips: close enough on
// a LAN to line up to well under a millisecond, following clocks that drift
// apart, and not thrown by a slow ping now and then.

import { describe, expect, it } from 'vitest';
import { ClockEstimate } from '../web/src/sync/clock';

function rng(seed: number): () => number {
  return () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 2 ** 32;
}

/** Ping every `every` ms for `ms`: each leg takes `base` plus an exponential delay of mean `jitter`; the other clock reads `skew(t)` ahead. */
function run(c: ClockEstimate, o: { ms: number; every: number; base: number; jitter: number; skew: (t: number) => number; seed?: number }): number {
  const r = rng(o.seed ?? 1);
  const leg = () => o.base - Math.log(1 - r()) * o.jitter;
  let t = 1_000;
  for (let e = 0; e < o.ms; e += o.every) {
    t += o.every;
    const up = leg();
    const down = leg();
    c.add(t, t + up + o.skew(t + up), t + up + down);
  }
  return t;
}

describe('clock estimate', () => {
  it('finds another clock to a fraction of a millisecond on a LAN', () => {
    for (const seed of [1, 2, 3, 4, 5]) {
      const c = new ClockEstimate();
      expect(c.settled).toBe(false);
      run(c, { ms: 10_000, every: 250, base: 0.4, jitter: 1.5, skew: () => 12_345.678, seed });
      expect(c.settled).toBe(true);
      expect(Math.abs(c.offset! - 12_345.678), `seed ${seed}`).toBeLessThan(0.3);
      expect(c.rtt!).toBeLessThan(2);
    }
  });

  it('follows two clocks drifting apart', () => {
    // 100 ppm, twice what a poor crystal does: 6 ms a minute
    const skew = (t: number) => -800 + t * 1e-4;
    const c = new ClockEstimate();
    const end = run(c, { ms: 120_000, every: 250, base: 0.4, jitter: 1, skew });
    expect(Math.abs(c.offset! - skew(end))).toBeLessThan(0.4);
  });

  it('shrugs off a slow ping now and then, and does well enough through the service', () => {
    const c = new ClockEstimate();
    const r = rng(9);
    let t = 1000;
    for (let i = 0; i < 80; i++) {
      t += 250;
      // one ping in five sits 40 ms in a queue on the way back
      const down = 0.5 + (r() < 0.2 ? 40 : 0);
      c.add(t, t + 0.5 + 250.25, t + 1 + down - 0.5);
    }
    expect(Math.abs(c.offset! - 250.25)).toBeLessThan(0.05);

    // through the service (about 20 ms each way, jittery): within a millisecond or two
    const s = new ClockEstimate();
    run(s, { ms: 20_000, every: 1000, base: 20, jitter: 4, skew: () => -3000 });
    expect(Math.abs(s.offset! + 3000)).toBeLessThan(2);
  });

  it('keeps its estimate over a change of path until the new one has a ping', () => {
    const c = new ClockEstimate();
    run(c, { ms: 5000, every: 1000, base: 20, jitter: 4, skew: () => 40 });
    const before = c.offset!;
    c.restart();
    expect(c.settled).toBe(true);
    expect(c.offset).toBe(before);
    c.add(0, 40.2, 0.4);
    expect(c.offset).toBeCloseTo(40, 6);
  });
});
