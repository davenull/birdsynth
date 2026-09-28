// MIDI byte parsing, shared by Web MIDI input and __synth.midiIn injection.

export interface MidiSink {
  noteOn(note: number, velocity: number, channel: number): void;
  noteOff(note: number, channel: number, velocity: number): void;
  pitchBend(channel: number, value: number): void;
  controller(channel: number, cc: number, value: number): void;
  channelPressure(channel: number, value: number): void;
  polyPressure(note: number, channel: number, value: number): void;
}

const DATA_BYTES: Record<number, number> = { 0x80: 2, 0x90: 2, 0xa0: 2, 0xb0: 2, 0xc0: 1, 0xd0: 1, 0xe0: 2 };

/**
 * Parse one or more MIDI messages (running status supported). System
 * messages are skipped. Velocities and values are scaled to 0..1, pitch
 * bend to -1..1.
 */
export function parseMidi(bytes: ArrayLike<number>, sink: MidiSink): void {
  let status = 0;
  let i = 0;
  while (i < bytes.length) {
    const b = bytes[i];
    if (b >= 0xf0) {
      // system messages: skip sysex payloads and anything else we don't use
      if (b === 0xf0) {
        while (i < bytes.length && bytes[i] !== 0xf7) i++;
      }
      i++;
      if (b < 0xf8) status = 0;
      continue;
    }
    if (b & 0x80) {
      status = b;
      i++;
    }
    if (!status) {
      i++;
      continue;
    }
    const kind = status & 0xf0;
    const ch = status & 0x0f;
    const need = DATA_BYTES[kind];
    if (i + need > bytes.length) break;
    const d1 = bytes[i] & 0x7f;
    const d2 = need > 1 ? bytes[i + 1] & 0x7f : 0;
    i += need;
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
