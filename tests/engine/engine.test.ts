// P0 gates checked against the real release engine.wasm.

import { describe, expect, it } from 'vitest';
import { PARAMS, PARAM_ID } from '../../web/src/gen/params';
import { ABI_HASH, DEBUG, TEL } from '../../web/src/gen/protocol';
import { toNorm, toPlain } from '../../web/src/state/param-math';
import { Engine, loadMulti, loadRecording, loadSpectral, module, pitch, testRecording } from './harness';

describe('engine.wasm', () => {
  it('imports nothing and matches the spec hash', () => {
    expect(WebAssembly.Module.imports(module())).toEqual([]);
    expect(new Engine().ex.wt_abi_hash() >>> 0).toBe(ABI_HASH);
  });

  it('maps every parameter exactly like the TypeScript side', () => {
    const e = new Engine();
    expect(e.ex.wt_param_count()).toBe(PARAMS.length);
    for (const p of PARAMS) {
      for (let k = 0; k <= 20; k++) {
        const n = Math.fround(k / 20);
        const rust = e.ex.wt_param_plain(p.id, n);
        const ts = toPlain(p, n);
        if (ts === -Infinity) expect(rust, p.key).toBe(-Infinity);
        else expect(Math.abs(rust - ts), `${p.key} at ${n}`).toBeLessThanOrEqual(Math.max(1e-6, Math.abs(ts) * 2e-6));
      }
    }
  });

  for (const sr of [44_100, 48_000]) {
    it(`plays A4 at 440.00 ± 0.01 Hz at ${sr} Hz`, () => {
      const e = new Engine(sr);
      e.w.noteOn(0, 69, 0, 1, 1);
      const { l } = e.render(sr * 3);
      expect(Math.abs(pitch(l.subarray(sr * 0.1), sr) - 440)).toBeLessThan(0.01);
    });
  }

  it('follows the oscillator pitch parameters', () => {
    const e = new Engine();
    const set = (key: string, plain: number) => {
      const p = PARAMS[PARAM_ID[key as keyof typeof PARAM_ID]];
      e.w.setParam(0, p.id, toNorm(p, plain));
    };
    set('osc.a.octave', -1);
    set('osc.a.semi', 7);
    set('osc.a.fine', 25);
    e.w.noteOn(0, 69, 0, 1, 1);
    const { l } = e.render(48_000 * 2);
    const want = 440 * 2 ** ((-12 + 7 + 0.25) / 12);
    expect(Math.abs(pitch(l.subarray(4800), 48_000) - want)).toBeLessThan(0.01);
  });

  it('never allocates while rendering 60 s of random notes', () => {
    const e = new Engine();
    let seed = 1;
    const rand = () => ((seed = (seed * 1103515245 + 12345) >>> 0) / 2 ** 32);
    const held: [number, number][] = [];
    let id = 1;
    const step = () => {
      // every 50 ms: maybe release a note, maybe start one
      if (held.length && rand() < 0.5) {
        const [note, nid] = held.splice(Math.floor(rand() * held.length), 1)[0];
        e.w.noteOff(0, note, 0, 0, nid);
      }
      if (rand() < 0.7) {
        const note = 36 + Math.floor(rand() * 48);
        held.push([note, id]);
        e.w.noteOn(0, note, 0, 0.3 + rand() * 0.7, id++);
      }
    };
    for (let i = 0; i < 20; i++) {
      step();
      e.render(2400); // warm up: fill every voice slot and the event queue path
    }
    const before = e.ex.wt_alloc_count();
    for (let i = 0; i < 1200; i++) {
      step();
      const { l } = e.render(2400);
      if (i % 100 === 0) expect(l.every(Number.isFinite)).toBe(true);
    }
    expect(e.ex.wt_alloc_count() - before).toBe(0);
    expect(e.tel()[TEL.queueDrops]).toBe(0);
  });

  it('never allocates playing recordings: sample, multisample, granular, spectral', () => {
    const e = new Engine();
    const set = (key: string, plain: number) => {
      const p = PARAMS.find((q) => q.key === key)!;
      e.w.setParam(0, p.id, toNorm(p, plain));
    };
    const rec = testRecording(48_000, 2);
    loadRecording(e, 0, rec, 48_000);
    loadMulti(e, 1, 1);
    loadRecording(e, 2, rec, 48_000);
    loadSpectral(e, 2, rec, 48_000);
    set('osc.a.type', 1);
    set('osc.b.enable', 1);
    set('osc.b.type', 2);
    set('osc.c.enable', 1);
    set('osc.c.type', 3);
    set('osc.a.unison', 4);
    let seed = 3;
    const rand = () => ((seed = (seed * 1103515245 + 12345) >>> 0) / 2 ** 32);
    let id = 1;
    const held: [number, number][] = [];
    const step = (i: number) => {
      if (held.length && rand() < 0.5) {
        const [note, nid] = held.splice(Math.floor(rand() * held.length), 1)[0];
        e.w.noteOff(0, note, 0, 0, nid);
      }
      if (rand() < 0.7) {
        const note = 36 + Math.floor(rand() * 48);
        held.push([note, id]);
        e.w.noteOn(0, note, 0, 0.3 + rand() * 0.7, id++);
      }
      // swap osc C between granular and spectral now and then
      if (i % 40 === 0) set('osc.c.type', (i / 40) % 2 ? 4 : 3);
    };
    for (let i = 0; i < 40; i++) {
      step(i);
      e.render(2400);
    }
    const before = e.ex.wt_alloc_count();
    for (let i = 40; i < 440; i++) {
      step(i);
      const { l } = e.render(2400);
      if (i % 50 === 0) expect(l.every(Number.isFinite)).toBe(true);
    }
    expect(e.ex.wt_alloc_count() - before).toBe(0);
    expect(e.tel()[TEL.tableErrors]).toBe(0);
  });

  it('keeps its views when memory grows', () => {
    const e = new Engine();
    const rab = (e.ex.memory as unknown as { toResizableBuffer(): ArrayBuffer }).toResizableBuffer();
    const out = new Float32Array(rab, e.ex.wt_out_ptr(0), 128);
    e.w.noteOn(0, 60, 0, 1, 1);
    e.render(256);
    e.ex.memory.grow(16);
    expect(out.length).toBe(128);
    e.render(128);
    expect(out.some((v) => v !== 0)).toBe(true);
  });

  it('reports the panic message when it traps', () => {
    const e = new Engine();
    e.w.debug(0, DEBUG.Trap, 0);
    expect(() => e.apply()).toThrow(WebAssembly.RuntimeError);
    const n = e.ex.wt_panic_len();
    const msg = String.fromCharCode(...new Uint8Array(e.ex.memory.buffer, e.ex.wt_panic_ptr(), n));
    expect(msg).toMatch(/debug trap requested by the host/);
    expect(msg).toMatch(/engine\.rs/);
  });

  it('renders 8 voices far faster than real time', () => {
    const e = new Engine();
    for (let i = 0; i < 8; i++) e.w.noteOn(0, 40 + i * 5, 0, 1, i + 1);
    e.render(48_000); // warm up (lets V8 optimize past its baseline tier)
    const secs = 10;
    const t0 = performance.now();
    e.render(48_000 * secs);
    const ms = performance.now() - t0;
    const load = ms / (secs * 1000);
    console.log(`8 voices: ${(load * 100).toFixed(2)}% of real time`);
    expect(load).toBeLessThan(0.05);
  });
});
