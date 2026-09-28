// Patches: capture → JSON → apply gives the same sound, bit for bit (by
// hash of the engine's output); older formats migrate.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { FLAG, PARAMS, PARAM_ID } from '../web/src/gen/params';
import { SOURCE } from '../web/src/state/matrix';
import { applyPatch, capture, migrate, PATCH_VERSION } from '../web/src/state/patch';
import { FACTORY } from '../web/src/presets/factory';
import { toPlain } from '../web/src/state/param-math';
import { audition, loudness, render, stores } from './patch-kit';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

describe('patches', () => {
  it('save → reload → render is bit-identical', () => {
    const a = stores();
    // a busy patch: random modulatable parameters, routings, shapes, an FX chain
    let seed = 7;
    const rnd = () => ((seed = (seed * 1664525 + 1013904223) >>> 0) / 4294967296);
    const modulatable = PARAMS.filter((p) => p.flags & FLAG.mod && !p.key.startsWith('fx.') && !p.key.startsWith('master.'));
    for (let i = 0; i < 80; i++) {
      const p = modulatable[Math.floor(rnd() * modulatable.length)];
      a.bank.set(p.id, rnd());
    }
    a.bank.set(PARAM_ID['osc.b.enable'], 1);
    a.bank.set(PARAM_ID['filter.1.enable'], 1);
    a.matrix.add(SOURCE['LFO 1'], PARAM_ID['filter.1.cutoff'], 0.4);
    a.matrix.add(SOURCE['Env 2'], PARAM_ID['osc.a.wt_pos'], 0.7);
    a.matrix.update(1, { curve: 0.3, bipolar: true });
    a.lfo.set(0, 'curve', [
      { x: 0, y: -1, c: 0.2 },
      { x: 0.5, y: 1, c: -0.4 },
    ]);
    a.remap.set(0, [
      { x: 0, y: 0, c: 0 },
      { x: 0.3, y: 0.8, c: 0.2 },
      { x: 1, y: 1, c: 0 },
    ]);
    a.bank.set(PARAM_ID['osc.a.warp1_mode'], 12 / 61);
    a.bank.set(PARAM_ID['osc.a.warp1_amount'], 0.6);
    a.fx.add(0, 5);
    a.fx.add(0, 7);
    a.bank.set(PARAM_ID['fx.delay.1.feedback'], 0.6);

    const json = JSON.stringify(capture(a));
    const b = stores();
    const warnings = applyPatch(b, migrate(JSON.parse(json)));
    expect(warnings).toEqual([]);
    expect(render(b)).toBe(render(a));
    // and a different patch sounds different (the hash sees something)
    b.bank.set(PARAM_ID['filter.1.cutoff'], 0.1);
    expect(render(b)).not.toBe(render(a));
  });

  it('migrates the pre-release format', () => {
    const raw = JSON.parse(fs.readFileSync(path.join(ROOT, 'tests/fixtures/patches/v0-lead.json'), 'utf8'));
    const p = migrate(raw);
    expect(p.version).toBe(PATCH_VERSION);
    expect(p.meta.name).toBe('Early Lead');
    // osc A's filter switch was on: it routes through the filters; osc B's was off: straight to main
    expect(p.params['osc.a.route']).toBe(0);
    expect(p.params['osc.b.route']).toBeCloseTo(1 / 3, 6);
    expect(p.params['osc.a.filter']).toBeUndefined();
    expect(p.tables[1]?.source).toBe('factory:Basic Shapes');
    const t = stores();
    expect(applyPatch(t, p)).toEqual([]);
    expect(t.matrix.slots[0]?.dest).toBe(PARAM_ID['filter.1.cutoff']);
  });

  it('refuses patches from a newer build and reports unknown parts', () => {
    expect(() => migrate({ format: 'birdsynth-patch', version: PATCH_VERSION + 1 })).toThrow(/newer/);
    const p = capture(stores());
    p.params['osc.z.level'] = 0.5;
    p.matrix.push({ slot: 3, source: 'Moon Phase', aux: 'None', dest: 'osc.a.level', amount: 1, curve: 0, output: 1, bipolar: false, bypass: false });
    const w = applyPatch(stores(), migrate(JSON.parse(JSON.stringify(p))));
    expect(w).toHaveLength(2);
  });

  it('every factory preset builds, applies cleanly and sits at the library level', () => {
    expect(FACTORY.length).toBeGreaterThanOrEqual(30);
    const names = new Set<string>();
    const vol = PARAMS[PARAM_ID['master.volume']];
    for (const p of FACTORY) {
      expect(names.has(p.meta.name), `duplicate ${p.meta.name}`).toBe(false);
      names.add(p.meta.name);
      const t = stores();
      expect(applyPatch(t, migrate(JSON.parse(JSON.stringify(p)))), p.meta.name).toEqual([]);
      // Levels: a held triad (drums: one hit) reaches -12 LUFS momentary, unless
      // its peaks reach -3 dBFS first (plucks and drums), so presets swap at even loudness.
      const { l, r } = audition(t, p);
      const { lufs, peak } = loudness(l, r);
      const pk = 20 * Math.log10(peak);
      expect(Number.isFinite(lufs) && Number.isFinite(pk), p.meta.name).toBe(true);
      const trim = Math.min(-12 - lufs, -3 - pk);
      const want = (toPlain(vol, t.bank.get(vol.id)) + trim).toFixed(1);
      expect(Math.abs(trim), `${p.meta.name}: ${lufs.toFixed(1)} LUFS, peak ${pk.toFixed(1)} dBFS; set master.volume to ${want}`).toBeLessThan(1);
    }
  });
});
