// The wavetable editor's model: every operation changes the right frames,
// reaches the engine (one frame at a time while drawing, the whole table
// when its shape changes), and undoes.

import { beforeAll, describe, expect, it } from 'vitest';
import { WtEditor, IMPORT_MODES, PROCESS } from '../web/src/editor/model';
import { TableStore } from '../web/src/state/tables';
import { useTools } from '../web/src/tools/client';
import { handle, type Exports } from '../web/src/tools/handle';
import type { ToolsReq } from '../web/src/tools/protocol';
import { toolsWasm } from './engine/harness';

const FL = 2048;

beforeAll(() => {
  const ex = toolsWasm() as unknown as Exports;
  useTools({ call: async (req: ToolsReq) => handle(ex, req).result as never });
});

function rig() {
  const sent: { kind: 'table' | 'frame'; osc: number; index?: number; frames?: number }[] = [];
  const tables = new TableStore();
  tables.attach({
    loadTable: (osc, _m, frames) => sent.push({ kind: 'table', osc, frames }),
    updateFrame: (osc, index) => sent.push({ kind: 'frame', osc, index }),
  });
  return { tables, sent, ed: new WtEditor(tables) };
}

const peak = (f: Float32Array) => f.reduce((m, v) => Math.max(m, Math.abs(v)), 0);

describe('wavetable editor', () => {
  it('draws, sends only the changed frame, and undoes a stroke as one step', async () => {
    const { tables, sent, ed } = rig();
    await tables.set(0, { name: 't', source: 'factory:Saw', frames: new Float32Array(3 * FL), count: 3 });
    ed.open(0);
    ed.select(1);
    sent.length = 0;
    ed.beginStroke();
    ed.segment(0, 0.5, 100, 0.5);
    ed.segment(100, 0.5, 200, -0.5);
    ed.endStroke();
    await ed.settled();
    expect(ed.frame(1)[50]).toBeCloseTo(0.5, 6);
    expect(ed.frame(1)[150]).toBeCloseTo(0, 2);
    expect(sent.every((s) => s.kind === 'frame' && s.index === 1)).toBe(true);
    expect(tables.osc[0]!.frames[FL + 50]).toBeCloseTo(0.5, 6);
    expect(tables.osc[0]!.source).toBe('edit:t'); // edited: patches must carry it now
    ed.undo();
    await ed.settled();
    expect(ed.frame(1)[50]).toBe(0);
    expect(tables.osc[0]!.frames[FL + 50]).toBe(0);
    ed.redo();
    expect(ed.frame(1)[50]).toBeCloseTo(0.5, 6);
  });

  it('runs formulas on the selection or everything, and reports errors', async () => {
    const { tables, ed } = rig();
    await tables.set(0, { name: 't', source: 'x', frames: new Float32Array(4 * FL), count: 4 });
    ed.open(0);
    ed.select(1);
    ed.select(2, 'toggle');
    expect(await ed.formula('sin(w * tau) * (q + 1)', 'selection')).toBeNull();
    expect(peak(ed.frame(0))).toBe(0);
    expect(peak(ed.frame(1))).toBeCloseTo(2, 4);
    expect(peak(ed.frame(2))).toBeCloseTo(3, 4);
    expect(await ed.formula('sin(', 'all')).toEqual({ error: 'the formula ends too soon', pos: 4 });
    expect(await ed.formula('y', 'all')).toBeNull();
    expect(ed.frame(3)[9]).toBe(1);
    ed.undo();
    expect(ed.frame(3)[9]).toBe(0);
    expect(peak(ed.frame(2))).toBeCloseTo(3, 4);
  });

  it('processes, reshapes the table, and undoes every kind of change', async () => {
    const { tables, sent, ed } = rig();
    const saw = Float32Array.from({ length: FL }, (_, i) => (2 * i) / FL - 1);
    const frames = new Float32Array(2 * FL);
    frames.set(saw);
    frames.set(saw.map((v) => v * 0.5), FL);
    await tables.set(0, { name: 't', source: 'x', frames, count: 2 });
    ed.open(0);
    await ed.process(PROCESS.normalizeEach, 'all');
    expect(peak(ed.frame(1))).toBeCloseTo(1, 5);
    // morph to 8 frames: the whole table goes to the engine again
    sent.length = 0;
    await ed.morph(1, 8);
    await ed.settled();
    expect(ed.count).toBe(8);
    expect(sent.at(-1)).toMatchObject({ kind: 'table', frames: 8 });
    ed.select(3);
    ed.addFrame(true);
    expect(ed.count).toBe(9);
    expect(ed.current).toBe(4);
    ed.select(0);
    ed.select(2, 'range');
    ed.removeFrames();
    expect(ed.count).toBe(6);
    ed.moveFrame(0, 5);
    expect(ed.current).toBe(5);
    ed.reverse();
    await ed.sortByBrightness();
    await ed.pwm(16);
    expect(ed.count).toBe(16);
    // all the way back
    const steps = [16, 6, 6, 6, 6, 9, 8, 2];
    for (const n of steps) {
      expect(ed.count).toBe(n);
      ed.undo();
    }
    expect(ed.count).toBe(2);
    expect(peak(ed.frame(1))).toBeCloseTo(0.5, 5);
    expect(ed.canUndo).toBe(false);
    await ed.settled();
    expect(tables.osc[0]!.count).toBe(2);
  });

  it('keeps at least one frame and at most 256', async () => {
    const { tables, ed } = rig();
    await tables.set(0, { name: 't', source: 'x', frames: new Float32Array(FL), count: 1 });
    ed.open(0);
    ed.removeFrames();
    expect(ed.count).toBe(1);
    await ed.pwm(256);
    ed.addFrame(true);
    expect(ed.count).toBe(256);
  });

  it('imports a recording', async () => {
    const { tables, ed } = rig();
    await tables.set(0, { name: 't', source: 'x', frames: new Float32Array(FL), count: 1 });
    ed.open(0);
    const sr = 48_000;
    const audio = Float32Array.from({ length: sr / 2 }, (_, i) => Math.sin((2 * Math.PI * 300 * i) / sr));
    const hz = await ed.pitch(audio, sr);
    expect(Math.abs(1200 * Math.log2(hz / 300))).toBeLessThan(1);
    await ed.importAudio(audio, sr, IMPORT_MODES.constant, sr / hz);
    expect(ed.count).toBe(150);
    await ed.importAudio(audio, sr, IMPORT_MODES.fft, 512, 20);
    expect(ed.count).toBe(20);
    ed.undo();
    expect(ed.count).toBe(150);
  });
});
