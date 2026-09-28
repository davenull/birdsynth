import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { parseClm, parseWav, writeWavetable } from '../web/src/tables/wav';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

describe('wavetable WAV', () => {
  it('round-trips frames bit for bit with a clm chunk', () => {
    const frames = new Float32Array(3 * 2048).map((_, i) => Math.sin(i * 0.013) * (1 + (i % 7) * 1e-7));
    const wav = parseWav(writeWavetable(frames, 2048, 1));
    expect(wav.clm).toEqual({ frameSize: 2048, interp: 1, flags: '10000000' });
    expect(wav.samples.length).toBe(frames.length);
    for (let i = 0; i < frames.length; i++) expect(Object.is(wav.samples[i], frames[i])).toBe(true);
  });

  it('reads Serum and Surge clm strings', () => {
    expect(parseClm('<!>2048 01000000 wavetable (www.xferrecords.com)')).toEqual({ frameSize: 2048, interp: 0, flags: '01000000' });
    expect(parseClm('<!>1024 10000000 wavetable Surge-XT\0')).toMatchObject({ frameSize: 1024, interp: 1 });
    expect(parseClm('no marker here')).toBeNull();
  });

  it('reads 16-bit stereo PCM as a mono mix', () => {
    // build a tiny 16-bit stereo file by hand
    const n = 4;
    const buf = new ArrayBuffer(44 + n * 4);
    const dv = new DataView(buf);
    const str = (at: number, s: string) => [...s].forEach((c, i) => dv.setUint8(at + i, c.charCodeAt(0)));
    str(0, 'RIFF');
    dv.setUint32(4, 36 + n * 4, true);
    str(8, 'WAVE');
    str(12, 'fmt ');
    dv.setUint32(16, 16, true);
    dv.setUint16(20, 1, true);
    dv.setUint16(22, 2, true);
    dv.setUint32(24, 48000, true);
    dv.setUint32(28, 48000 * 4, true);
    dv.setUint16(32, 4, true);
    dv.setUint16(34, 16, true);
    str(36, 'data');
    dv.setUint32(40, n * 4, true);
    for (let i = 0; i < n; i++) {
      dv.setInt16(44 + i * 4, 16384, true);
      dv.setInt16(46 + i * 4, -16384 * (i % 2), true);
    }
    const w = parseWav(buf);
    expect(w.channels).toBe(2);
    expect(Array.from(w.samples)).toEqual([0.25, 0, 0.25, 0]);
  });

  it('tools.wasm builds factory tables and mips through its ABI', async () => {
    const bytes = fs.readFileSync(path.join(ROOT, 'web/wasm/tools.wasm'));
    const mod = new WebAssembly.Module(bytes);
    expect(WebAssembly.Module.imports(mod)).toEqual([]);
    const ex = new WebAssembly.Instance(mod, {}).exports as unknown as Record<string, (...a: number[]) => number> & { memory: WebAssembly.Memory };
    expect(ex.tl_factory_count()).toBe(21);
    const frames = ex.tl_factory_frames(0);
    const fl = ex.tl_frame_len();
    const fs2 = ex.tl_frame_stride();
    const src = ex.tl_alloc(frames * fl * 4);
    expect(ex.tl_factory_build(0, src)).toBe(frames);
    const dst = ex.tl_alloc(frames * fs2 * 4);
    ex.tl_mips(src, frames, dst);
    const mips = new Float32Array(ex.memory.buffer, dst, frames * fs2);
    expect(mips.every(Number.isFinite)).toBe(true);
    // level 0 of frame 0 matches the raw frame (a sine, which needs no band-limiting)
    const raw = new Float32Array(ex.memory.buffer, src, fl);
    for (let i = 0; i < fl; i += 97) expect(Math.abs(mips[i] - raw[i])).toBeLessThan(1e-4);
  });
});
