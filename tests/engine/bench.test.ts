// Real-time cost of the release engine.wasm, measured in Node (the same V8
// as Chrome) after warm-up. The gates are from docs/plan.md.

import { describe, expect, it } from 'vitest';
import { PARAMS, PARAM_ID, type ParamKey } from '../../web/src/gen/params';
import { DEBUG } from '../../web/src/gen/protocol';
import { toNorm } from '../../web/src/state/param-math';
import { Engine } from './harness';

function load(setup: Partial<Record<ParamKey, number>>, voices: number, opts: { scalar?: boolean; secs?: number } = {}): number {
  const e = new Engine(48_000);
  const set = (key: ParamKey, plain: number) => {
    const p = PARAMS[PARAM_ID[key]];
    e.w.setParam(0, p.id, toNorm(p, plain));
  };
  set('voice.polyphony', 32);
  for (const [k, v] of Object.entries(setup)) set(k as ParamKey, v);
  if (opts.scalar) e.w.debug(0, DEBUG.Scalar, 1);
  for (let i = 0; i < voices; i++) e.w.noteOn(0, 40 + i * 2, 0, 1, i + 1);
  e.render(48_000); // warm up past V8's baseline tier
  const secs = opts.secs ?? 4;
  const t0 = performance.now();
  e.render(48_000 * secs);
  return (performance.now() - t0) / (secs * 1000);
}

const pct = (x: number) => `${(x * 100).toFixed(2)}%`;

describe('engine cost', () => {
  it('16 voices x 16 unison stays within 6% of real time', () => {
    const simd = load({ 'osc.a.unison': 16 }, 16);
    const scalar = load({ 'osc.a.unison': 16 }, 16, { scalar: true });
    console.log(`16 voices x 16 unison: SIMD ${pct(simd)}, scalar ${pct(scalar)}`);
    expect(simd).toBeLessThan(0.06);
  });

  it('reports the heavier cases', () => {
    const three = load({ 'osc.a.unison': 16, 'osc.b.enable': 1, 'osc.b.unison': 16, 'osc.c.enable': 1, 'osc.c.unison': 16 }, 16, { secs: 2 });
    const warped = load({ 'osc.a.unison': 16, 'osc.a.warp1_mode': 3, 'osc.a.warp1_amount': 0.5 }, 16, { secs: 2 });
    const typical = load({ 'osc.a.unison': 7, 'osc.b.enable': 1, 'osc.b.unison': 7, 'filter.1.enable': 1 }, 8, { secs: 2 });
    console.log(`16 voices x 3 osc x 16 unison: ${pct(three)}; 16x16 with Bend warp: ${pct(warped)}; 8 voices x 2 osc x 7 unison + SVF: ${pct(typical)}`);
    expect(typical).toBeLessThan(0.05);
  });
});
