// WAV reading and writing for wavetables, including the `clm ` chunk that
// Serum (and Surge, and birdsynth) use to mark a file's frame size:
//   <!>2048 01000000 wavetable (vendor)
// The four digits after "<!>" are the frame size. The first flag digit is
// the frame interpolation (0 none, 1 crossfade, 2-4 spectral); the second is
// Xfer's factory flag, which we always write as 0.

export interface Clm {
  frameSize: number;
  interp: number;
  flags: string;
}

export interface Wav {
  sampleRate: number;
  channels: number;
  /** Mono (channels averaged), -1..1. */
  samples: Float32Array;
  clm: Clm | null;
}

const text = (dv: DataView, at: number, n: number) => {
  let s = '';
  for (let i = 0; i < n; i++) s += String.fromCharCode(dv.getUint8(at + i));
  return s;
};

export function parseClm(s: string): Clm | null {
  const m = /<!>(\d{3,4})\s?([0-9]{0,8})/.exec(s);
  if (!m) return null;
  const frameSize = Number(m[1]);
  const flags = m[2] ?? '';
  return { frameSize, interp: flags.length ? Number(flags[0]) : 0, flags };
}

export function parseWav(buf: ArrayBuffer): Wav {
  const dv = new DataView(buf);
  if (buf.byteLength < 12 || text(dv, 0, 4) !== 'RIFF' || text(dv, 8, 4) !== 'WAVE') throw new Error('not a WAV file');
  let fmt: { format: number; channels: number; rate: number; bits: number } | null = null;
  let data: { at: number; len: number } | null = null;
  let clm: Clm | null = null;
  let at = 12;
  while (at + 8 <= buf.byteLength) {
    const id = text(dv, at, 4);
    const len = dv.getUint32(at + 4, true);
    const body = at + 8;
    const end = Math.min(body + len, buf.byteLength);
    if (id === 'fmt ') {
      let format = dv.getUint16(body, true);
      if (format === 0xfffe && len >= 26) format = dv.getUint16(body + 24, true); // WAVE_FORMAT_EXTENSIBLE
      fmt = { format, channels: dv.getUint16(body + 2, true), rate: dv.getUint32(body + 4, true), bits: dv.getUint16(body + 14, true) };
    } else if (id === 'data') {
      data = { at: body, len: end - body };
    } else if (id === 'clm ') {
      clm = parseClm(text(dv, body, end - body));
    }
    at = body + len + (len & 1); // chunks are word-aligned
  }
  if (!fmt || !data) throw new Error('WAV file has no fmt or data chunk');
  const { format, channels, rate, bits } = fmt;
  const bytes = bits / 8;
  const frames = Math.floor(data.len / (bytes * channels));
  const out = new Float32Array(frames);
  const read = (p: number): number => {
    if (format === 3 && bits === 32) return dv.getFloat32(p, true);
    if (format === 3 && bits === 64) return dv.getFloat64(p, true);
    if (format !== 1) throw new Error(`unsupported WAV format ${format}`);
    switch (bits) {
      case 8:
        return (dv.getUint8(p) - 128) / 128;
      case 16:
        return dv.getInt16(p, true) / 32768;
      case 24: {
        const v = dv.getUint8(p) | (dv.getUint8(p + 1) << 8) | (dv.getInt8(p + 2) << 16);
        return v / 8388608;
      }
      case 32:
        return dv.getInt32(p, true) / 2147483648;
      default:
        throw new Error(`unsupported ${bits}-bit PCM`);
    }
  };
  for (let i = 0; i < frames; i++) {
    let s = 0;
    for (let c = 0; c < channels; c++) s += read(data.at + (i * channels + c) * bytes);
    out[i] = channels === 1 ? s : s / channels;
  }
  return { sampleRate: rate, channels, samples: out, clm };
}

/** Write frames (count × frameSize, mono) as a 32-bit float WAV with a clm chunk. */
export function writeWavetable(frames: Float32Array, frameSize = 2048, interp = 1, vendor = 'birdsynth'): ArrayBuffer {
  let clm = `<!>${String(frameSize).padStart(4, '0')} ${interp}0000000 wavetable (${vendor})`;
  if (clm.length % 2) clm += '\0';
  const fmtLen = 16;
  const dataLen = frames.length * 4;
  const total = 4 + (8 + fmtLen) + (8 + clm.length) + (8 + dataLen);
  const buf = new ArrayBuffer(8 + total);
  const dv = new DataView(buf);
  let at = 0;
  const str = (s: string) => {
    for (let i = 0; i < s.length; i++) dv.setUint8(at++, s.charCodeAt(i));
  };
  const u32 = (v: number) => {
    dv.setUint32(at, v, true);
    at += 4;
  };
  const u16 = (v: number) => {
    dv.setUint16(at, v, true);
    at += 2;
  };
  str('RIFF');
  u32(total);
  str('WAVE');
  str('fmt ');
  u32(fmtLen);
  u16(3); // IEEE float
  u16(1);
  u32(44_100);
  u32(44_100 * 4);
  u16(4);
  u16(32);
  str('clm ');
  u32(clm.length);
  str(clm);
  str('data');
  u32(dataLen);
  for (let i = 0; i < frames.length; i++) {
    dv.setFloat32(at, frames[i], true);
    at += 4;
  }
  return buf;
}
