// Real-time cost of the release engine.wasm, measured in Node (the same V8
// as Chrome) after warm-up. The gates are from docs/plan.md.

import { describe, expect, it } from 'vitest';
import { PARAMS, PARAM_ID, type ParamKey } from '../../web/src/gen/params';
import { DEBUG, SOURCES } from '../../web/src/gen/protocol';
import { toNorm } from '../../web/src/state/param-math';
import { Engine, factoryIr } from './harness';

type Route = [source: (typeof SOURCES)[number], dest: ParamKey, amount: number];

/** FX chains to set up: [chain, [type, instance][]], plus convolvers to load (instance → factory IR). */
interface FxSetup {
  chains: [number, [number, number][]][];
  irs?: [number, number][];
}

function load(setup: Partial<Record<ParamKey, number>>, voices: number, opts: { scalar?: boolean; secs?: number; mod?: Route[]; fx?: FxSetup } = {}): number {
  const e = new Engine(48_000);
  const set = (key: ParamKey, plain: number) => {
    const p = PARAMS[PARAM_ID[key]];
    e.w.setParam(0, p.id, toNorm(p, plain));
  };
  set('voice.polyphony', 32);
  for (const [k, v] of Object.entries(setup)) set(k as ParamKey, v);
  if (opts.scalar) e.w.debug(0, DEBUG.Scalar, 1);
  (opts.mod ?? []).forEach(([src, dest, amount], slot) => e.w.setModSlot(0, slot, SOURCES.indexOf(src), 0, 0, PARAM_ID[dest], amount, 0, 1));
  for (const [inst, ir] of opts.fx?.irs ?? []) {
    const [taps, data] = factoryIr(ir, e.sr);
    e.w.loadIr(0, inst, taps, e.asset(data), data.byteLength);
  }
  for (const [chain, refs] of opts.fx?.chains ?? []) e.w.setChain(0, chain, refs.length, refs.map(([t, i]) => t * 256 + i));
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

  it('heavy patch without FX stays within 20% of real time', () => {
    // 16 voices, 3 osc x 16 unison with two warps each, 2 driven ladders, 16 matrix slots
    const osc = (o: 'a' | 'b' | 'c'): Partial<Record<ParamKey, number>> => ({
      [`osc.${o}.enable`]: 1,
      [`osc.${o}.unison`]: 16,
      [`osc.${o}.warp1_mode`]: 3,
      [`osc.${o}.warp1_amount`]: 0.4,
      [`osc.${o}.warp2_mode`]: 7,
      [`osc.${o}.warp2_amount`]: 0.3,
      [`osc.${o}.balance`]: 0.5,
    });
    const filter = (f: 1 | 2): Partial<Record<ParamKey, number>> => ({ [`filter.${f}.enable`]: 1, [`filter.${f}.type`]: 11, [`filter.${f}.drive`]: 0.5, [`filter.${f}.res`]: 0.4 });
    const mod: Route[] = [
      ['LFO 1', 'filter.1.cutoff', 0.3],
      ['LFO 2', 'filter.2.cutoff', 0.3],
      ['Env 2', 'osc.a.wt_pos', 0.5],
      ['Env 2', 'osc.b.wt_pos', 0.4],
      ['Env 3', 'osc.c.wt_pos', 0.3],
      ['LFO 3', 'osc.a.warp1_amount', 0.2],
      ['LFO 4', 'osc.b.warp1_amount', 0.2],
      ['Velocity', 'osc.a.level', 0.2],
      ['Note', 'filter.1.res', 0.1],
      ['Mod Wheel', 'osc.c.warp2_amount', 0.3],
      ['Rand 1', 'osc.a.pan', 0.2],
      ['Rand 2', 'osc.b.pan', 0.2],
      ['Macro 1', 'filter.2.res', 0.2],
      ['LFO 5', 'osc.c.level', 0.2],
      ['Env 4', 'osc.a.detune', 0.2],
      ['LFO 6', 'osc.b.detune', 0.2],
    ];
    const heavy = load({ ...osc('a'), ...osc('b'), ...osc('c'), ...filter(1), ...filter(2), 'mix.filter_routing': 1 }, 16, { secs: 2, mod });
    console.log(`heavy patch (no FX): ${pct(heavy)}`);
    expect(heavy).toBeLessThan(0.2);
  });

  it('heavy patch with full FX stays within 30% of real time', () => {
    const osc = (o: 'a' | 'b' | 'c'): Partial<Record<ParamKey, number>> => ({
      [`osc.${o}.enable`]: 1,
      [`osc.${o}.unison`]: 16,
      [`osc.${o}.warp1_mode`]: 3,
      [`osc.${o}.warp1_amount`]: 0.4,
      [`osc.${o}.warp2_mode`]: 7,
      [`osc.${o}.warp2_amount`]: 0.3,
      [`osc.${o}.balance`]: 0.5,
      [`osc.${o}.send1`]: 0.3,
    });
    const filter = (f: 1 | 2): Partial<Record<ParamKey, number>> => ({ [`filter.${f}.enable`]: 1, [`filter.${f}.type`]: 11, [`filter.${f}.drive`]: 0.5, [`filter.${f}.res`]: 0.4 });
    const mod: Route[] = [
      ['LFO 1', 'filter.1.cutoff', 0.3],
      ['LFO 2', 'filter.2.cutoff', 0.3],
      ['Env 2', 'osc.a.wt_pos', 0.5],
      ['Env 2', 'osc.b.wt_pos', 0.4],
      ['Env 3', 'osc.c.wt_pos', 0.3],
      ['LFO 3', 'osc.a.warp1_amount', 0.2],
      ['LFO 4', 'osc.b.warp1_amount', 0.2],
      ['Velocity', 'osc.a.level', 0.2],
      ['Note', 'filter.1.res', 0.1],
      ['Mod Wheel', 'osc.c.warp2_amount', 0.3],
      ['Rand 1', 'osc.a.pan', 0.2],
      ['Rand 2', 'osc.b.pan', 0.2],
      ['Macro 1', 'fx.reverb.1.size', 0.2],
      ['LFO 5', 'fx.delay.1.feedback', 0.2],
      ['Env 4', 'fx.distortion.1.drive', 0.2],
      ['LFO 6', 'fx.filter.1.cutoff', 0.2],
    ];
    // Main: every effect but the shifter and splitter (with a 2.6 s hall in the convolver); Bus 1: a reverb
    const main: [number, number][] = [[0, 0], [1, 0], [2, 0], [3, 0], [4, 0], [5, 0], [6, 0], [7, 0], [8, 0], [9, 0], [11, 0], [12, 0]];
    const fx: FxSetup = { chains: [[0, main], [1, [[7, 1]]]], irs: [[0, 2]] };
    const heavy = load({ ...osc('a'), ...osc('b'), ...osc('c'), ...filter(1), ...filter(2), 'mix.filter_routing': 1, 'fx.compressor.1.mode': 1 }, 16, { secs: 2, mod, fx });
    console.log(`heavy patch (full FX): ${pct(heavy)}`);
    expect(heavy).toBeLessThan(0.3);
  });

  it('reports what each effect costs', () => {
    const costs: string[] = [];
    const base = load({ 'osc.a.unison': 4 }, 4, { secs: 1 });
    ['hyper', 'distortion', 'flanger', 'phaser', 'chorus', 'delay', 'compressor', 'reverb', 'eq', 'filter', 'bode', 'convolve', 'utility', 'splitter'].forEach((name, t) => {
      const fx: FxSetup = { chains: [[0, [[t, 0]]]], irs: name === 'convolve' ? [[0, 2]] : [] };
      const c = load({ 'osc.a.unison': 4 }, 4, { secs: 1, fx }) - base;
      costs.push(`${name} ${pct(c)}`);
    });
    console.log(`per effect: ${costs.join(', ')}`);
  });
});
