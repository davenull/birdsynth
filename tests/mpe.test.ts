// MPE on the host: with MPE on, each member channel's bend, CC 74 and
// pressure go to the notes on that channel only; channel 1 stays global.

import { describe, expect, it } from 'vitest';
import { PARAM_ID } from '../web/src/gen/params';
import { parseMidi } from '../web/src/input/midi';
import { Synth } from '../web/src/synth';

/** A synth whose engine is a recorder of the commands it's sent. */
function recorded(): { synth: Synth; log: unknown[][] } {
  const synth = new Synth();
  const log: unknown[][] = [];
  const w = new Proxy({}, { get: (_, name) => (...args: unknown[]) => log.push([name, ...args.slice(1)]) });
  (synth as unknown as { host: unknown }).host = { send: (fn: (w: unknown) => void) => fn(w), tel: new Float32Array(1024) };
  (synth as unknown as { resume: () => void }).resume = () => {};
  return { synth, log };
}

const bend14 = (v: number) => {
  const x = Math.round(8192 + v * (v >= 0 ? 8191 : 8192));
  return [x & 0x7f, x >> 7];
};

describe('MPE', () => {
  it('sends each channel its own expression, and channel 1 to everything', () => {
    const { synth, log } = recorded();
    synth.bank.set(PARAM_ID['voice.mpe'], 1);
    // the controller sets channel 3's bend before its note, as MPE controllers do
    parseMidi([0xe2, ...bend14(0.5), 0x92, 64, 100, 0x91, 60, 100], synth);
    const on = log.filter((c) => c[0] === 'noteOn').map((c) => ({ note: c[1], ch: c[2], id: c[4] }));
    expect(on.map((n) => [n.note, n.ch])).toEqual([
      [64, 2],
      [60, 1],
    ]);
    const idOf = (note: number) => on.find((n) => n.note === note)!.id;
    const ex = () => log.filter((c) => c[0] === 'noteExpression').map((c) => [c[1], +(c[2] as number).toFixed(3), c[3]]);
    // each note starts with its channel's state: 64 bent by half, 60 not
    expect(ex()).toContainEqual([0, 0.5, idOf(64)]);
    expect(ex()).toContainEqual([0, 0, idOf(60)]);
    log.length = 0;
    parseMidi([0xe1, ...bend14(-0.25), 0xb1, 74, 127, 0xd1, 64], synth);
    expect(ex()).toEqual([
      [0, -0.25, idOf(60)],
      [1, 1, idOf(60)],
      [2, +(64 / 127).toFixed(3), idOf(60)],
    ]);
    expect(log.some((c) => c[0] === 'pitchBend' || c[0] === 'channelPressure')).toBe(false);
    expect(synth.wheels.bend).toBe(0);
    // channel 1 is the master channel: a global bend
    log.length = 0;
    parseMidi([0xe0, ...bend14(1)], synth);
    expect(log.filter((c) => c[0] === 'pitchBend')).toHaveLength(1);
    expect(ex()).toEqual([]);
  });

  it('leaves channels alone with MPE off', () => {
    const { synth, log } = recorded();
    parseMidi([0x91, 60, 100, 0xe1, ...bend14(0.5), 0xd1, 30], synth);
    expect(log.filter((c) => c[0] === 'noteExpression')).toEqual([]);
    expect(log.filter((c) => c[0] === 'pitchBend' || c[0] === 'channelPressure')).toHaveLength(2);
  });
});
