// The engine's modulation matrix against the TypeScript reference model
// (web/src/state/matrix.ts): 1,000 random routings, each checked through
// the telemetry the UI reads (destination ids and resolved values).

import { describe, expect, it } from 'vitest';
import { FLAG, PARAMS, PARAM_ID, type ParamKey } from '../../web/src/gen/params';
import { SOURCES, TEL, TEL_COUNT } from '../../web/src/gen/protocol';
import { SOURCE, evaluate, slotFlags, type ModSlot, type SourceName } from '../../web/src/state/matrix';
import { toNorm } from '../../web/src/state/param-math';
import { Engine } from './harness';

/** A seeded generator so a failure repeats. */
function rng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

describe('modulation matrix', () => {
  it('matches the reference model over 1,000 random routings', () => {
    const e = new Engine(48_000);
    const set = (key: ParamKey, plain: number) => e.w.setParam(0, PARAM_ID[key], toNorm(PARAMS[PARAM_ID[key]], plain));
    // steady, known sources while one note is held
    const sustain = [0.9, 0.3, 0.65, 0.8];
    sustain.forEach((s, i) => {
      set(`env.${i + 1}.attack` as ParamKey, 0);
      set(`env.${i + 1}.decay` as ParamKey, 1);
      set(`env.${i + 1}.sustain` as ParamKey, s);
    });
    const macros = [0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 1];
    macros.forEach((m, i) => set(`macro.${i + 1}.value` as ParamKey, m));
    e.w.noteOn(0, 72, 0, 0.75, 1);
    e.w.controller(0, 0, 1, 0.4);
    e.w.channelPressure(0, 0, 0.55);
    e.w.polyPressure(0, 72, 0, 0.35, 1);
    e.w.pitchBend(0, 0, -0.5);
    e.render(24_000); // envelopes reach sustain; smoothing and the bend settle

    const f = Math.fround;
    const known = new Map<number, number>([
      ...sustain.map((s, i) => [SOURCE[`Env ${i + 1}` as SourceName], f(s)] as [number, number]),
      ...macros.map((m, i) => [SOURCE[`Macro ${i + 1}` as SourceName], f(m)] as [number, number]),
      [SOURCE.Velocity, 0.75],
      [SOURCE.Note, f(f(72 - 60) / 60)],
      [SOURCE['Mod Wheel'], f(0.4)],
      [SOURCE['Pitch Bend'], -0.5],
      [SOURCE.Aftertouch, f(0.55)],
      [SOURCE['Poly AT'], f(0.35)],
      [SOURCE['Release Vel'], 0],
      [SOURCE['Voice Index'], f(1 / 31)],
      [SOURCE['Active Voices'], f(1 / 32)],
      [SOURCE.Fixed, 1],
      [SOURCE['MPE X'], 0],
    ]);
    const sources = [...known.keys()];
    // destinations that can't feed back into those sources
    const dests = PARAMS.filter((p) => p.flags & FLAG.mod && !/^(env|lfo|macro|voice|global|master|mix)\./.test(p.key)).map((p) => p.id);

    const r = rng(0xb1d5);
    const pick = <T>(xs: readonly T[]) => xs[Math.floor(r() * xs.length)];
    let checked = 0;
    for (let trial = 0; trial < 1000; trial++) {
      e.w.clearMod(0);
      const slots: (ModSlot | null)[] = Array(64).fill(null);
      const n = 1 + Math.floor(r() * 6);
      for (let k = 0; k < n; k++) {
        const slot: ModSlot = {
          source: pick(sources),
          aux: r() < 0.5 ? 0 : pick(sources),
          dest: r() < 0.3 && k > 0 ? slots.find(Boolean)!.dest : pick(dests), // some share a destination
          amount: f(r() * 2 - 1),
          curve: r() < 0.5 ? 0 : f(r() * 2 - 1),
          output: r() < 0.5 ? 1 : f(r()),
          bipolar: r() < 0.5,
          bypass: r() < 0.1,
        };
        const i = Math.floor(r() * 64);
        slots[i] = slot;
      }
      slots.forEach((s, i) => s && e.w.setModSlot(0, i, s.source, s.aux, slotFlags(s), s.dest, s.amount, s.curve, s.output));
      e.render(32);
      const tel = e.tel();
      const want = evaluate(slots, (s) => known.get(s) ?? NaN);
      const got = new Map<number, number>();
      for (let d = 0; d < TEL_COUNT.modDest; d++) {
        const id = tel[TEL.modDest + d];
        if (id >= 0) got.set(id, tel[TEL.modValue + d]);
      }
      expect([...got.keys()].sort(), `trial ${trial}: destinations`).toEqual([...want.keys()].sort());
      for (const [dest, off] of want) {
        const expected = Math.min(1, Math.max(0, PARAMS[dest].def + off));
        const actual = got.get(dest)!;
        if (Math.abs(actual - expected) > 2e-5) {
          throw new Error(`trial ${trial}: ${PARAMS[dest].key} is ${actual}, the model says ${expected} (slots ${JSON.stringify(slots.flatMap((s, i) => (s ? [{ i, ...s, source: SOURCES[s.source], aux: SOURCES[s.aux] }] : [])))})`);
        }
        checked++;
      }
      expect(tel[TEL.modSlots]).toBe(slots.filter((s) => s && !s.bypass).length);
    }
    expect(checked).toBeGreaterThan(1500);
  });
});
