// SFZ fixtures map the way their text says.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { findSample, noteNumber, parseSfz } from '../web/src/sfz/sfz';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

describe('SFZ', () => {
  it('reads note names', () => {
    expect([noteNumber('c4'), noteNumber('C#4'), noteNumber('db4'), noteNumber('a4'), noteNumber('c-1'), noteNumber('60'), noteNumber('h2')]).toEqual([60, 61, 61, 69, 0, 60, null]);
  });

  it('maps the layered fixture: inheritance, defines, keys, loops', () => {
    const s = parseSfz(fs.readFileSync(path.join(ROOT, 'tests/fixtures/sfz/layers.sfz'), 'utf8'));
    expect(s.regions).toHaveLength(4);
    const [a, b, c, d] = s.regions;
    expect(a).toMatchObject({ sample: 'Samples/Piano Soft/C3 soft.wav', lokey: 48, hikey: 52, lovel: 1, hivel: 63, root: 48, loopMode: 0, oneShot: false });
    expect(a.gain).toBeCloseTo(10 ** (-6 / 20), 6); // the global volume, through the define
    expect(b).toMatchObject({ lokey: 53, hikey: 59, root: 55.12 });
    expect(c).toMatchObject({ sample: 'Samples/Piano Soft/C3 hard.wav', lokey: 48, hikey: 48, root: 48, lovel: 64, hivel: 127, gain: 1 });
    expect(d).toMatchObject({ lokey: 53, hikey: 127, root: 69 - 12, loopMode: 4, loopStart: 1000, loopEnd: 48_000, pan: -0.5 });
    expect(s.warnings).toContain('a region without sample= was skipped');
    expect(s.ignored).toMatchObject({ ampeg_release: 5, '<curve>': 1 }); // every region under <global> carries it
  });

  it('finds sample files by path or by name', () => {
    const files = ['kit/Samples/Piano Soft/C3 soft.wav', 'other/c3 HARD.WAV'];
    expect(findSample('Samples/Piano Soft/C3 soft.wav', files)).toBe(files[0]);
    expect(findSample('Samples/Piano Soft/C3 hard.wav', files)).toBe(files[1]);
    expect(findSample('missing.wav', files)).toBeNull();
  });
});
