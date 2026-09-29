// The CPU guard steps up after half a second over 70% (or an underrun while busy) and back
// down after ten seconds under 35%, one level at a time.

import { describe, expect, it } from 'vitest';
import { CpuGuard, GUARD_MAX } from '../web/src/audio/guard';

describe('CPU guard', () => {
  it('steps up under load and down when it eases', () => {
    const g = new CpuGuard();
    expect(g.update(50, 1)).toBeNull();
    expect(g.update(80, 0.25)).toBeNull();
    expect(g.update(80, 0.25)).toBe(1);
    expect(g.update(90, 0.25), 'severe: at once').toBe(2);
    expect(g.update(10, 1, true), 'an underrun with the engine idle is not its doing').toBeNull();
    expect(g.update(40, 1, true), 'an underrun while busy counts as load').toBe(3);
    for (let i = 0; i < 5; i++) g.update(95, 1);
    expect(g.level).toBe(GUARD_MAX);
    // in between: holds
    for (let i = 0; i < 30; i++) expect(g.update(50, 1)).toBeNull();
    // quiet: one level per ten seconds
    const steps: number[] = [];
    for (let i = 0; i < 45; i++) {
      const r = g.update(10, 1);
      if (r !== null) steps.push(r);
    }
    expect(steps).toEqual([3, 2, 1, 0]);
    expect(g.update(10, 100)).toBeNull();
  });
});
