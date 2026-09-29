// MIDI files become clips: the fixture parses as written, and a clip made
// from it plays each note on its frame.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { PARAMS, PARAM_ID } from '../web/src/gen/params';
import { parseSmf, writeSmf } from '../web/src/midi/smf';
import { toNorm } from '../web/src/state/param-math';
import { ClipStore } from '../web/src/state/seq';
import { Engine } from './engine/harness';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const phrase = () => {
  const b = fs.readFileSync(path.join(ROOT, 'tests/fixtures/midi/phrase.mid'));
  return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
};

describe('MIDI files', () => {
  it('reads the fixture: format 1, running status, velocity-0 note-offs, a note left open', () => {
    const s = parseSmf(phrase());
    expect(s).toMatchObject({ format: 1, ppq: 96, length: 4, beatsPerBar: 3, name: 'Lead' });
    expect(s.notes.map((n) => [n.start, n.length, n.key, Math.round(n.velocity * 127), n.channel])).toEqual([
      [0, 1, 60, 100, 0],
      [1, 0.5, 64, 80, 0],
      [1.5, 1.5, 67, 127, 0],
      [3, 1, 72, 64, 2],
    ]);
  });

  it('writes what it reads', () => {
    const s = parseSmf(phrase());
    expect(parseSmf(writeSmf(s.notes)).notes).toEqual(s.notes);
    expect(() => parseSmf(new ArrayBuffer(20))).toThrow(/not a MIDI file/);
  });

  it('imports into a clip of whole bars', () => {
    const c = new ClipStore();
    c.importSmf(2, parseSmf(phrase()));
    // 4 beats of 3/4: two bars
    expect(c.clips[2].length).toBe(6);
    expect(c.clips[2].notes.map((n) => [n.start, n.key])).toEqual([
      [0, 60],
      [1, 64],
      [1.5, 67],
      [3, 72],
    ]);
  });

  it('plays a clip made from it on the right frames', () => {
    const e = new Engine(48_000);
    const set = (k: string, v: number) => {
      const p = PARAMS[PARAM_ID[k as keyof typeof PARAM_ID]];
      e.w.setParam(0, p.id, toNorm(p, v));
    };
    set('osc.a.phase', 90); // each note's first sample shows
    set('env.1.attack', 0);
    set('env.1.release', 0.5);
    set('clip.enable', 1);
    const notes = parseSmf(phrase()).notes;
    // shorten each note a little so there's a gap to find the next start
    const tail = notes.flatMap((n) => [n.start, n.length * 0.9, n.key, n.velocity, 1, 0]);
    e.w.setClip(0, 0, notes.length, 4, tail);
    e.render(24_000);
    e.w.transport(0, 1);
    const { l } = e.render(96_000); // 4 beats at 120
    const starts: number[] = [];
    let quiet = 64;
    l.forEach((v, i) => {
      if (Math.abs(v) > 1e-6) {
        if (quiet >= 64) starts.push(i);
        quiet = 0;
      } else quiet++;
    });
    // the transport started on the render boundary: beat b is frame b × 24,000 (plus the first block's alignment)
    const first = starts[0];
    expect(starts.map((f) => (f - first) / 24_000)).toEqual([0, 1, 1.5, 3]);
  });
});
