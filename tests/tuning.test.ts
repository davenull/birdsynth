// Tuning files: Scala scales and keyboard maps, AnaMark .tun.

import { describe, expect, it } from 'vitest';
import { parseKbm, parseScl, parseTun, tableFromScale, TuningError } from '../web/src/tuning/tuning';

const ET12 = `! 12-TET.scl
!
12 tone equal temperament
 12
!
 100.0
 200.
 300.0 cents
 400.0
 500.0
 600.0
 700.0
 800.0
 900.0
 1000.0
 1100.0
 2/1
`;

const JUST = `Just major, 7 notes
7
9/8
5/4
4/3
3/2
5/3
15/8
2
`;

describe('tuning files', () => {
  it('reads a 12-TET scale back as standard tuning', () => {
    const t = tableFromScale(parseScl(ET12));
    for (let n = 0; n < 128; n++) expect(t[n]).toBeCloseTo(n, 4);
  });

  it('maps a 7-note scale onto consecutive keys from middle C', () => {
    const s = parseScl(JUST);
    expect(s.description).toBe('Just major, 7 notes');
    const t = tableFromScale(s);
    expect(t[60]).toBeCloseTo(60, 5);
    expect(t[62]).toBeCloseTo(60 + 12 * Math.log2(5 / 4), 5); // key 62 is degree 2
    expect(t[67]).toBeCloseTo(72, 5); // a period (2/1) seven keys up
    expect(t[53]).toBeCloseTo(48, 5);
  });

  it('follows a keyboard map: white keys only, A at 432 Hz', () => {
    const kbm = `! white keys play the just scale, black keys are silent
12
0
127
60
69
432.0
7
0
x
1
x
2
3
x
4
x
5
x
6
`;
    const km = parseKbm(kbm);
    expect(km.map).toEqual([0, null, 1, null, 2, 3, null, 4, null, 5, null, 6]);
    const t = tableFromScale(parseScl(JUST), km);
    expect(t[69]).toBeCloseTo(69 + 12 * Math.log2(432 / 440), 5);
    // E (degree 2, 5/4) against A (degree 5, 5/3): a just fourth down... from A: 5/4 ÷ 5/3 = 3/4
    expect(t[64] - t[69]).toBeCloseTo(12 * Math.log2(3 / 4), 5);
    expect(t[72] - t[60]).toBeCloseTo(12, 5); // the pattern repeats at degree 7 (2/1)
    expect(t[61]).toBe(61); // silent keys stay where they were
  });

  it('reads .tun files, preferring [Exact Tuning]', () => {
    const tun = `; AnaMark tuning
[Tuning]
note 69=6950
[Exact Tuning]
BaseFreq=8.1757989156437073336
note 69=6900.5
note 70= 7000
`;
    const { table } = parseTun(tun);
    expect(table[69]).toBeCloseTo(69.005, 5);
    expect(table[70]).toBeCloseTo(70, 5);
    expect(table[10]).toBe(10); // unlisted notes stay standard
    expect(parseTun('[Tuning]\nnote 0=100\n').table[0]).toBeCloseTo(1, 5);
  });

  it('says what is wrong with a broken file', () => {
    expect(() => parseScl('desc\n3\n100.0\n')).toThrow(/3 notes but lists 1/);
    expect(() => parseScl('desc\n2\n100.0\nabc\n')).toThrow(/line 4/);
    expect(() => parseScl('desc\n1\n-3/2\n')).toThrow(TuningError);
    expect(() => parseKbm('12\n0\n127\n')).toThrow(/7 lines/);
    expect(() => parseTun('[Info]\nName=x\n')).toThrow(/no \[Tuning\]/);
  });
});
