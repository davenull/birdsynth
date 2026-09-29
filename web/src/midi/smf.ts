// Standard MIDI Files (types 0 and 1): the notes, in beats, for a clip.
// Tempo changes don't matter here (a clip is in beats); note-ons with
// velocity 0 are note-offs; a note still held at the end runs to it.

export interface SmfNote {
  start: number;
  length: number;
  key: number;
  velocity: number;
  channel: number;
}

export interface Smf {
  format: number;
  ppq: number;
  notes: SmfNote[];
  /** Beats to the end of the last event. */
  length: number;
  /** The first time signature's beats per bar (4 when none). */
  beatsPerBar: number;
  name: string;
}

export class SmfError extends Error {}

export function parseSmf(buf: ArrayBuffer): Smf {
  const v = new DataView(buf);
  let at = 0;
  const u32 = () => {
    const x = v.getUint32(at);
    at += 4;
    return x;
  };
  const u16 = () => {
    const x = v.getUint16(at);
    at += 2;
    return x;
  };
  const tag = () => String.fromCharCode(v.getUint8(at), v.getUint8(at + 1), v.getUint8(at + 2), v.getUint8(at + 3));
  if (buf.byteLength < 14 || tag() !== 'MThd') throw new SmfError('not a MIDI file (no MThd header)');
  at += 4;
  const hlen = u32();
  const format = u16();
  const tracks = u16();
  const division = u16();
  if (division & 0x8000) throw new SmfError('SMPTE-timed MIDI files are not supported');
  const ppq = division || 480;
  at = 8 + hlen;
  const notes: SmfNote[] = [];
  let end = 0;
  let beatsPerBar = 0;
  let name = '';
  for (let t = 0; t < tracks && at + 8 <= buf.byteLength; t++) {
    if (tag() !== 'MTrk') {
      // skip unknown chunks
      at += 4;
      const skip = u32();
      at += skip;
      t--;
      continue;
    }
    at += 4;
    const len = u32();
    const stop = Math.min(buf.byteLength, at + len);
    let tick = 0;
    let status = 0;
    const open = new Map<number, { start: number; velocity: number }[]>();
    const vlq = () => {
      let x = 0;
      for (let i = 0; i < 4; i++) {
        const b = v.getUint8(at++);
        x = (x << 7) | (b & 0x7f);
        if (!(b & 0x80)) break;
      }
      return x;
    };
    while (at < stop) {
      tick += vlq();
      let b = v.getUint8(at);
      if (b & 0x80) {
        at++;
        if (b < 0xf0) status = b;
      } else {
        b = status; // running status
      }
      if (b === 0xff) {
        const type = v.getUint8(at++);
        const n = vlq();
        if (type === 0x03 && !name) name = new TextDecoder().decode(new Uint8Array(buf, at, n));
        if (type === 0x58 && !beatsPerBar && n >= 2) beatsPerBar = v.getUint8(at) * (4 / 2 ** v.getUint8(at + 1));
        at += n;
        continue;
      }
      if (b === 0xf0 || b === 0xf7) {
        at += vlq();
        continue;
      }
      const kind = b & 0xf0;
      const ch = b & 0x0f;
      const d1 = v.getUint8(at++);
      const d2 = kind === 0xc0 || kind === 0xd0 ? 0 : v.getUint8(at++);
      const key = ch * 128 + d1;
      if (kind === 0x90 && d2 > 0) {
        const list = open.get(key) ?? [];
        list.push({ start: tick, velocity: d2 / 127 });
        open.set(key, list);
      } else if (kind === 0x80 || (kind === 0x90 && d2 === 0)) {
        const on = open.get(key)?.shift();
        if (on) notes.push({ start: on.start / ppq, length: Math.max(1, tick - on.start) / ppq, key: d1, velocity: on.velocity, channel: ch });
      }
      end = Math.max(end, tick);
    }
    for (const [key, list] of open) for (const on of list) notes.push({ start: on.start / ppq, length: Math.max(1, end - on.start) / ppq, key: key % 128, velocity: on.velocity, channel: Math.floor(key / 128) });
    at = stop;
  }
  notes.sort((a, b) => a.start - b.start || a.key - b.key);
  return { format, ppq, notes, length: end / ppq, beatsPerBar: beatsPerBar || 4, name };
}

/** Write notes as a type-0 MIDI file (for tests and for exporting a clip). */
export function writeSmf(notes: SmfNote[], ppq = 480, bpm = 120): ArrayBuffer {
  const ev: { tick: number; bytes: number[] }[] = [];
  for (const n of notes) {
    const s = Math.round(n.start * ppq);
    ev.push({ tick: s, bytes: [0x90 | n.channel, n.key, Math.max(1, Math.round(n.velocity * 127))] });
    ev.push({ tick: s + Math.max(1, Math.round(n.length * ppq)), bytes: [0x80 | n.channel, n.key, 0] });
  }
  ev.sort((a, b) => a.tick - b.tick || (a.bytes[0] & 0xf0) - (b.bytes[0] & 0xf0));
  const us = Math.round(60_000_000 / bpm);
  const body: number[] = [0, 0xff, 0x51, 3, (us >> 16) & 255, (us >> 8) & 255, us & 255];
  const vlq = (x: number) => {
    const out = [x & 0x7f];
    while ((x >>= 7)) out.unshift((x & 0x7f) | 0x80);
    return out;
  };
  let last = 0;
  for (const e of ev) {
    body.push(...vlq(e.tick - last), ...e.bytes);
    last = e.tick;
  }
  body.push(0, 0xff, 0x2f, 0);
  const head = [0x4d, 0x54, 0x68, 0x64, 0, 0, 0, 6, 0, 0, 0, 1, (ppq >> 8) & 255, ppq & 255];
  const trk = [0x4d, 0x54, 0x72, 0x6b, (body.length >>> 24) & 255, (body.length >> 16) & 255, (body.length >> 8) & 255, body.length & 255];
  return new Uint8Array([...head, ...trk, ...body]).buffer;
}
