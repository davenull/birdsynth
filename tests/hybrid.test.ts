// Hybridize: sections come whole from one parent or the other, repeatably
// for a seed, and Blend only moves continuous values between the two.

import { describe, expect, it } from 'vitest';
import { PARAMS } from '../web/src/gen/params';
import { FACTORY } from '../web/src/presets/factory';
import { hybridize, hybridSections, sectionOf } from '../web/src/state/hybrid';
import { applyPatch } from '../web/src/state/patch';
import { stores } from './patch-kit';

const byName = (n: string) => FACTORY.find((p) => p.meta.name === n)!;
const A = byName('Supersaw Lead');
const B = byName('Wobble');
const val = (p: typeof A, key: string) => p.params[key] ?? PARAMS.find((x) => x.key === key)!.def;

describe('Hybridize', () => {
  it('takes each section whole from one parent, the same way for the same seed', () => {
    const h = hybridize(A, B, { seed: 7 });
    expect(hybridize(A, B, { seed: 7 })).toEqual(h);
    expect(hybridize(A, B, { seed: 8 })).not.toEqual(h);
    const from = hybridSections({ seed: 7 });
    expect(new Set(from.values())).toEqual(new Set(['a', 'b']));
    for (const p of PARAMS) {
      const want = from.get(sectionOf(p.key)) === 'b' ? val(B, p.key) : val(A, p.key);
      expect(val(h, p.key), p.key).toBe(want);
    }
    // what a section carries with it
    const src = (s: string) => (from.get(s) === 'b' ? B : A);
    ['a', 'b', 'c'].forEach((o, i) => expect(h.tables[i]).toEqual(src(`osc.${o}`).tables[i]));
    expect(h.fx).toEqual(src('fx').fx);
    expect(h.matrix).toEqual(src('matrix').matrix);
    for (let i = 0; i < 10; i++) expect(h.lfo.curves[i] ?? null).toEqual(src(`lfo.${i + 1}`).lfo.curves[i] ?? null);
    expect(h.meta.name).toBe('Supersaw Lead × Wobble');
    expect(applyPatch(stores(), h)).toEqual([]);
  });

  it('blends only continuous values, and only between the parents', () => {
    const plain = hybridize(A, B, { seed: 3 });
    const mixed = hybridize(A, B, { seed: 3, blend: 1 });
    let moved = 0;
    for (const p of PARAMS) {
      const a = val(A, p.key);
      const b = val(B, p.key);
      const v = val(mixed, p.key);
      if (p.curve.kind === 'enum' || p.curve.kind === 'bool' || p.curve.kind === 'int') expect(v, p.key).toBe(val(plain, p.key));
      else {
        expect(v, p.key).toBeGreaterThanOrEqual(Math.min(a, b) - 1e-9);
        expect(v, p.key).toBeLessThanOrEqual(Math.max(a, b) + 1e-9);
        if (v !== val(plain, p.key)) moved++;
      }
    }
    expect(moved).toBeGreaterThan(5);
    // bias 0 and 1: all from one side
    expect(hybridize(A, B, { seed: 1, bias: 0 }).params).toEqual(Object.fromEntries(Object.entries(A.params).filter(([k, v]) => v !== PARAMS.find((p) => p.key === k)!.def)));
    expect(hybridize(A, B, { seed: 1, bias: 1 }).tables).toEqual(B.tables);
  });
});
