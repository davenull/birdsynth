// The tools worker's requests for the wavetable editor, through the same
// handler the worker runs, on the real tools.wasm in Node.

import { describe, expect, it } from 'vitest';
import { handle, type Exports } from '../web/src/tools/handle';
import type { ToolsReq, ToolsResults } from '../web/src/tools/protocol';
import { toolsWasm } from './engine/harness';

const FL = 2048;
function call<K extends ToolsReq['op']>(req: Extract<ToolsReq, { op: K }>): ToolsResults[K] {
  return handle(toolsWasm() as unknown as Exports, req).result as ToolsResults[K];
}

const saw = () => Float32Array.from({ length: FL }, (_, i) => (2 * i) / FL - 1);

describe('tools: wavetable editor requests', () => {
  it('runs formulas and reports errors with their position', () => {
    const frames = new Float32Array(3 * FL);
    const r = call({ op: 'formula', src: 'lerp(saw(w), sin(w * tau), y)', frames, count: 3, apply: new Uint8Array([1, 1, 1]), selected: new Uint8Array(3), seed: 1 });
    expect(r.error).toBeNull();
    expect(r.frames[512]).toBeCloseTo(-0.5, 5);
    expect(r.frames[2 * FL + 512]).toBeCloseTo(1, 5);
    const bad = call({ op: 'formula', src: 'sin(x', frames, count: 3, apply: new Uint8Array([1, 1, 1]), selected: new Uint8Array(3), seed: 1 });
    expect(bad).toMatchObject({ error: "expected ')' or ',', found the end", pos: 5 });
    expect(call({ op: 'formulaCheck', src: 'x * 2' })).toEqual({ error: null, pos: 0 });
    expect(call({ op: 'formulaCheck', src: 'x × 2' })).toMatchObject({ pos: 2 });
  });

  it('processes, morphs, sorts and makes PWM', () => {
    const t = new Float32Array(2 * FL);
    t.set(saw());
    t.set(saw().map((v) => v * 0.25), FL);
    const n = call({ op: 'wtProcess', kind: 0, a: 0, b: 0, frames: t, count: 2 });
    expect(Math.max(...n.subarray(FL).map(Math.abs))).toBeCloseTo(1, 5);
    expect(() => call({ op: 'wtProcess', kind: 99, a: 0, b: 0, frames: t, count: 2 })).toThrow(/unknown process/);
    const m = call({ op: 'wtMorph', keys: t, count: 2, target: 5, mode: 1 });
    expect(m.length).toBe(5 * FL);
    const pwm = call({ op: 'wtPwm', frame: saw(), count: 16 });
    expect(pwm.length).toBe(16 * FL);
    const sine = Float32Array.from({ length: FL }, (_, i) => Math.sin((2 * Math.PI * i) / FL));
    const order = call({ op: 'wtSort', frames: Float32Array.from([...saw(), ...sine]), count: 2 });
    expect([...order]).toEqual([1, 0]);
    const { mag, phase } = call({ op: 'wtAnalyze', frame: sine });
    expect(mag[1]).toBeCloseTo(1, 4);
    expect(phase[1]).toBeCloseTo(0, 3);
    const back = call({ op: 'wtSynthesize', mag, phase });
    expect(back[512]).toBeCloseTo(1, 4);
  });

  it('imports recordings: pitch within a cent, frame counts as expected', () => {
    const sr = 48_000;
    const hz = 220;
    const audio = Float32Array.from({ length: sr }, (_, i) => Math.sin((2 * Math.PI * hz * i) / sr) * 0.8 + 0.2 * Math.sin((2 * Math.PI * 2 * hz * i) / sr));
    const f = call({ op: 'pitch', audio, sr });
    expect(Math.abs(1200 * Math.log2(f / hz))).toBeLessThan(1);
    const c = call({ op: 'import', mode: 0, audio, sr, arg: sr / f, max: 256 });
    expect(c.count).toBe(220);
    expect(c.frames.length).toBe(220 * FL);
    expect(call({ op: 'import', mode: 3, audio, sr, arg: 1024, max: 256 }).count).toBe(46);
    expect(call({ op: 'import', mode: 2, audio, sr, arg: 0, max: 64 }).count).toBe(64);
    expect(call({ op: 'pitch', audio: new Float32Array(sr), sr })).toBe(0);
  });
});
