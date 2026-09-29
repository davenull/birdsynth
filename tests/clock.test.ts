// MIDI clock-in: real-time bytes reach the sink, and 24 ticks a beat give the tempo.

import { describe, expect, it } from 'vitest';
import { parseMidi, type MidiSink } from '../web/src/input/midi';
import { MidiClock } from '../web/src/input/clock';

function sink(log: (string | number)[][]): MidiSink {
  return {
    noteOn: (n, v, c) => log.push(['on', n, v, c]),
    noteOff: (n, c) => log.push(['off', n, c]),
    pitchBend: () => {},
    controller: () => {},
    channelPressure: () => {},
    polyPressure: () => {},
    realtime: (s, t) => log.push(['rt', s, t]),
  };
}

describe('MIDI clock', () => {
  it('passes real-time bytes through, even inside a running message', () => {
    const log: (string | number)[][] = [];
    parseMidi([0xfa, 0x90, 60, 0xf8, 100, 0xfc], sink(log), 5);
    expect(log).toEqual([
      ['rt', 0xfa, 5],
      ['rt', 0xf8, 5],
      ['on', 60, 100 / 127, 0],
      ['rt', 0xfc, 5],
    ]);
  });

  it('finds the tempo from a beat of ticks, despite jitter', () => {
    const c = new MidiClock();
    const perTick = 60_000 / (128 * 24);
    let bpm: number | null = null;
    let seen = 0;
    for (let i = 0; i < 24 * 8; i++) {
      const r = c.tick(i * perTick + (i % 2 ? 0.8 : -0.8));
      if (r !== null) {
        bpm = r;
        seen++;
      }
    }
    expect(bpm).not.toBeNull();
    expect(Math.abs(bpm! - 128)).toBeLessThan(0.1);
    expect(seen, 'a steady clock reports once').toBe(1);
    // the tempo changes: reported again within a beat
    const t0 = 24 * 8 * perTick;
    const slower = 60_000 / (100 * 24);
    let now: number | null = null;
    for (let i = 1; i <= 26; i++) now = c.tick(t0 + i * slower) ?? now;
    expect(Math.abs(now! - 100)).toBeLessThan(0.1);
  });
});
