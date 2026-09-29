// MIDI byte parsing, shared by Web MIDI input and __synth.midiIn injection.

export interface MidiSink {
  noteOn(note: number, velocity: number, channel: number): void;
  noteOff(note: number, channel: number, velocity: number): void;
  pitchBend(channel: number, value: number): void;
  controller(channel: number, cc: number, value: number): void;
  channelPressure(channel: number, value: number): void;
  polyPressure(note: number, channel: number, value: number): void;
  /** System real-time: clock (0xF8), start (0xFA), continue (0xFB), stop (0xFC). */
  realtime?(status: number, time: number): void;
}

const DATA_BYTES: Record<number, number> = { 0x80: 2, 0x90: 2, 0xa0: 2, 0xb0: 2, 0xc0: 1, 0xd0: 1, 0xe0: 2 };

/**
 * Parse one or more MIDI messages (running status supported). Real-time
 * messages go to the sink's `realtime` (with `time`, ms); other system
 * messages are skipped. Velocities and values are scaled to 0..1, pitch
 * bend to -1..1.
 */
export function parseMidi(bytes: ArrayLike<number>, sink: MidiSink, time = performance.now()): void {
  let status = 0;
  let i = 0;
  while (i < bytes.length) {
    const b = bytes[i];
    if (b >= 0xf8) {
      sink.realtime?.(b, time);
      i++;
      continue;
    }
    if (b >= 0xf0) {
      // system common messages and sysex: skipped (sysex to its end), and they cancel running status
      if (b === 0xf0) {
        while (i < bytes.length && bytes[i] !== 0xf7) i++;
      }
      i++;
      status = 0;
      continue;
    }
    if (b & 0x80) {
      status = b;
      i++;
      continue;
    }
    if (!status) {
      i++;
      continue;
    }
    // a data byte: gather the message's data (real-time bytes can sit between them)
    const need = DATA_BYTES[status & 0xf0];
    let d1 = 0;
    let d2 = 0;
    let got = 0;
    while (got < need && i < bytes.length) {
      const x = bytes[i];
      if (x >= 0xf8) {
        sink.realtime?.(x, time);
        i++;
        continue;
      }
      if (x & 0x80) break;
      if (got === 0) d1 = x;
      else d2 = x;
      got++;
      i++;
    }
    // cut short (by a new status byte, or the end): dropped
    if (got < need) continue;
    const kind = status & 0xf0;
    const ch = status & 0x0f;
    switch (kind) {
      case 0x90:
        if (d2 > 0) sink.noteOn(d1, d2 / 127, ch);
        else sink.noteOff(d1, ch, 0);
        break;
      case 0x80:
        sink.noteOff(d1, ch, d2 / 127);
        break;
      case 0xa0:
        sink.polyPressure(d1, ch, d2 / 127);
        break;
      case 0xb0:
        sink.controller(ch, d1, d2 / 127);
        break;
      case 0xd0:
        sink.channelPressure(ch, d1 / 127);
        break;
      case 0xe0: {
        const v = (d2 << 7) | d1; // 14-bit, centre 8192
        sink.pitchBend(ch, v >= 8192 ? (v - 8192) / 8191 : (v - 8192) / 8192);
        break;
      }
      // 0xc0 program change: unused for now
    }
  }
}
