// The tuning store keeps its table across sessions and sends it to the engine,
// which plays it: a .scl through the real engine, checked by pitch.

import { describe, expect, it } from 'vitest';
import { TuningStore } from '../web/src/state/tuning';
import { Engine, pitch } from './engine/harness';

function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
}
const file = (name: string, text: string) => ({ name, text: async () => text });

describe('tuning store', () => {
  it('loads a scale, remembers it, and the engine plays in it', async () => {
    const store = memory();
    const t = new TuningStore(store);
    const e = new Engine(48_000);
    t.attach({ setTuning: (table) => e.w.setTuning(0, table.length, table) });
    // quarter-comma meantone-ish: a 5/4 major third on key 64
    await t.load([file('thirds.scl', 'pure thirds\n12\n100.0\n200.0\n300.0\n5/4\n500.0\n600.0\n700.0\n800.0\n900.0\n1000.0\n1100.0\n2/1\n')]);
    expect(t.name).toBe('pure thirds');
    e.render(24_000);
    e.w.noteOn(0, 64, 0, 0.8, 1);
    e.render(4096);
    const f = pitch(e.render(16_384).l, 48_000);
    expect(f).toBeCloseTo(261.6255653 * 1.25, 1);
    const again = new TuningStore(store);
    expect(again.name).toBe('pure thirds');
    expect(again.table![64]).toBeCloseTo(60 + 12 * Math.log2(1.25), 4);
    again.reset();
    expect(new TuningStore(store).table).toBeNull();
  });

  it('asks for the scale when only a keyboard map is given', async () => {
    await expect(new TuningStore(memory()).load([file('a.kbm', '0\n0\n127\n60\n69\n440\n0\n')])).rejects.toThrow(/needs a \.scl/);
  });
});
