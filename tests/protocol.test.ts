import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { PARAMS } from '../web/src/gen/params';
import { CmdWriter, HEADER_BYTES, OP } from '../web/src/gen/protocol';
import { format, parse, snap, toNorm, toPlain } from '../web/src/state/param-math';
import { parseMidi, type MidiSink } from '../web/src/input/midi';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

describe('schema', () => {
  it('generated files are up to date', () => {
    expect(() => execFileSync(process.execPath, ['tools/gen-schema.mjs', '--check'], { cwd: ROOT, stdio: 'pipe' })).not.toThrow();
  });
});

describe('CmdWriter', () => {
  it('lays out headers and payloads as the schema says', () => {
    const w = new CmdWriter(16);
    w.setParam(1234, 7, 0.5);
    w.noteOn(0, 60, 2, 0.75, 99);
    const b = w.bytes();
    const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
    expect(dv.getUint16(0, true)).toBe(OP.SetParam);
    expect(dv.getUint32(4, true)).toBe(8);
    expect(dv.getFloat64(8, true)).toBe(1234);
    expect(dv.getUint16(HEADER_BYTES, true)).toBe(7);
    expect(dv.getFloat32(HEADER_BYTES + 4, true)).toBe(0.5);
    const n = HEADER_BYTES + 8;
    expect(dv.getUint16(n, true)).toBe(OP.NoteOn);
    expect(dv.getUint32(n + 4, true)).toBe(16);
    expect(dv.getUint8(n + HEADER_BYTES)).toBe(60);
    expect(dv.getUint8(n + HEADER_BYTES + 1)).toBe(2);
    expect(dv.getFloat32(n + HEADER_BYTES + 4, true)).toBe(0.75);
    expect(dv.getUint32(n + HEADER_BYTES + 8, true)).toBe(99);
    expect(b.length).toBe(n + HEADER_BYTES + 16);
  });

  it('splits big batches between commands', () => {
    const w = new CmdWriter();
    for (let i = 0; i < 100; i++) w.setParam(0, i, 0);
    const parts = w.take(24 * 10 + 5);
    expect(parts.length).toBe(10);
    expect(parts.every((p) => p.byteLength % 24 === 0 && p.byteLength <= 245)).toBe(true);
    expect(w.length).toBe(0);
  });
});

describe('param math', () => {
  it('round-trips plain values', () => {
    for (const p of PARAMS) {
      for (const n of [0, 0.25, 0.5, 0.75, 1]) {
        const s = snap(p, n);
        const back = toNorm(p, toPlain(p, s));
        expect(Math.abs(back - s), p.key).toBeLessThan(1e-9);
      }
    }
  });

  it('formats and parses what it shows', () => {
    const vol = PARAMS.find((p) => p.key === 'master.volume')!;
    expect(format(vol, vol.def)).toBe('-6.0 dB');
    expect(format(vol, 0)).toBe('-inf dB');
    const pan = PARAMS.find((p) => p.key === 'osc.a.pan')!;
    expect(format(pan, 0.5)).toBe('C');
    expect(format(pan, 0.25)).toBe('L50');
    expect(parse(pan, 'r25')).toBeCloseTo(0.625, 6);
    const att = PARAMS.find((p) => p.key === 'env.1.attack')!;
    expect(format(att, att.def)).toBe('0.50 ms');
    expect(toPlain(att, parse(att, '2 s')!)).toBeCloseTo(2000, 3);
    const semi = PARAMS.find((p) => p.key === 'osc.a.semi')!;
    expect(format(semi, 1)).toBe('+12 st');
    expect(parse(semi, '-7')).toBeCloseTo(toNorm(semi, -7), 9);
  });
});

describe('MIDI parsing', () => {
  function sink() {
    const log: string[] = [];
    const s: MidiSink = {
      noteOn: (n, v, c) => log.push(`on ${n} ${v.toFixed(2)} ${c}`),
      noteOff: (n, c) => log.push(`off ${n} ${c}`),
      pitchBend: (c, v) => log.push(`bend ${c} ${v.toFixed(3)}`),
      controller: (c, cc, v) => log.push(`cc ${c} ${cc} ${v.toFixed(2)}`),
      channelPressure: (c, v) => log.push(`at ${c} ${v.toFixed(2)}`),
      polyPressure: (n, c, v) => log.push(`pat ${n} ${c} ${v.toFixed(2)}`),
    };
    return { s, log };
  }

  it('handles note on/off, velocity-0 offs and running status', () => {
    const { s, log } = sink();
    parseMidi([0x91, 60, 127, 64, 64, 0x81, 60, 0, 0x91, 64, 0], s);
    expect(log).toEqual(['on 60 1.00 1', 'on 64 0.50 1', 'off 60 1', 'off 64 1']);
  });

  it('scales pitch bend to -1..1 around the centre', () => {
    const { s, log } = sink();
    parseMidi([0xe0, 0, 64, 0xe0, 0x7f, 0x7f, 0xe0, 0, 0], s);
    expect(log).toEqual(['bend 0 0.000', 'bend 0 1.000', 'bend 0 -1.000']);
  });

  it('skips sysex and realtime bytes', () => {
    const { s, log } = sink();
    parseMidi([0xf0, 1, 2, 3, 0xf7, 0xb0, 1, 127, 0xf8, 0xd2, 100], s);
    expect(log).toEqual(['cc 0 1 1.00', 'at 2 0.79']);
  });
});
