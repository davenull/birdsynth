// The recordings the oscillators' Sample, Granular and Spectral types play:
// one per oscillator, with its slices (found at the transients). What the
// engine gets depends on the oscillator's type: the recording packed with
// its halved copies (Sample, Granular), or its spectral analysis
// (Spectral), built by the tools worker when first needed and kept, so a
// restarted engine gets it back at once. A Spectral oscillator can also
// play a picture (a PNG read as a spectrogram) instead of a recording.

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';
import { tools } from '../tools/client';
import type { ParamBank } from './bank';
import { toPlain } from './param-math';

export const TYPE = { wavetable: 0, sample: 1, multi: 2, granular: 3, spectral: 4 } as const;
/** Longest recording kept, in seconds. */
export const MAX_SECONDS = 60;

export interface Recording {
  name: string;
  rate: number;
  /** One or two channels of equal length. */
  channels: Float32Array[];
  /** Slice starts, in frames. */
  slices: number[];
}

/** A picture played as a spectrogram. */
export interface Picture {
  name: string;
  /** The PNG (or other image) file, kept for saving with a patch. */
  bytes: ArrayBuffer;
  seconds: number;
}

export interface RecordingSink {
  loadRecording(osc: number, r: { channels: number; levels: number; slices: number; frames: number; rate: number; data: Float32Array } | null): void;
  loadSpectral(osc: number, a: { frames: number; rate: number; data: Float32Array } | null): void;
}

type Analysis = { frames: number; rate: number; data: Float32Array };

export class RecordingStore {
  readonly osc: (Recording | null)[] = [null, null, null];
  readonly picture: (Picture | null)[] = [null, null, null];
  private packed: ({ data: Float32Array; levels: number } | null)[] = [null, null, null];
  private analysis: (Analysis | null)[] = [null, null, null];
  /** What the engine has now, per oscillator. */
  private sent: [string, string][] = [
    ['', ''],
    ['', ''],
    ['', ''],
  ];
  private sink: RecordingSink | null = null;
  private readonly subs = new Set<(osc: number) => void>();
  private serial = [0, 0, 0];

  constructor(private readonly bank: ParamBank) {
    for (let o = 0; o < 3; o++) bank.subscribe(PARAM_ID[`osc.${'abc'[o]}.type` as ParamKey], () => void this.sync(o));
  }

  subscribe(fn: (osc: number) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  private emit(o: number): void {
    for (const fn of this.subs) fn(o);
  }

  attach(sink: RecordingSink | null): void {
    this.sink = sink;
    this.sent = [
      ['', ''],
      ['', ''],
      ['', ''],
    ];
  }

  resync(): void {
    this.sent = [
      ['', ''],
      ['', ''],
      ['', ''],
    ];
    for (let o = 0; o < 3; o++) void this.sync(o);
  }

  type(o: number): number {
    const p = PARAMS[PARAM_ID[`osc.${'abc'[o]}.type` as ParamKey]];
    return toPlain(p, this.bank.get(p.id));
  }

  /** The analysis the Spectral type is playing (for the display), if any. */
  spectrum(o: number): Analysis | null {
    return this.analysis[o];
  }

  /** Install a recording on an oscillator (slices found now unless given). */
  async set(o: number, r: Recording | null, findSlices = true): Promise<void> {
    const n = ++this.serial[o];
    if (r) {
      const max = Math.round(MAX_SECONDS * r.rate);
      const chans = r.channels.slice(0, 2).map((c) => (c.length > max ? c.slice(0, max) : c));
      r = { ...r, channels: chans };
      if (findSlices && !r.slices.length) {
        const found = await tools().call({ op: 'onsets', audio: mono(r), sr: r.rate });
        if (n !== this.serial[o]) return;
        r.slices = [0, ...[...found].filter((f) => f > r!.rate * 0.02)];
      }
    }
    this.osc[o] = r;
    this.picture[o] = null;
    this.packed[o] = null;
    this.analysis[o] = null;
    this.sent[o] = ['', ''];
    this.emit(o);
    await this.sync(o);
  }

  /** New slice points (from the sample editor). */
  async setSlices(o: number, slices: number[]): Promise<void> {
    const r = this.osc[o];
    if (!r) return;
    r.slices = [...slices].sort((a, b) => a - b);
    this.packed[o] = null;
    this.sent[o][0] = '';
    this.emit(o);
    await this.sync(o);
  }

  /** Play a picture as a spectrogram (the Spectral type). */
  async setPicture(o: number, pic: Picture, analysis: Analysis): Promise<void> {
    ++this.serial[o];
    this.picture[o] = pic;
    this.analysis[o] = analysis;
    this.sent[o][1] = '';
    this.emit(o);
    await this.sync(o);
  }

  /** Send what the oscillator's type needs, building it if it isn't built yet. */
  async sync(o: number): Promise<void> {
    const sink = this.sink;
    if (!sink) return;
    const n = this.serial[o];
    const t = this.type(o);
    const r = this.osc[o];
    const key = r ? `${r.name}:${r.channels[0].length}:${r.slices.length}` : '';
    if (t === TYPE.sample || t === TYPE.granular) {
      if (!r) {
        if (this.sent[o][0] !== 'none') sink.loadRecording(o, null);
        this.sent[o][0] = 'none';
        return;
      }
      if (this.sent[o][0] === key) return;
      this.packed[o] ??= await tools().call({ op: 'recPrepare', channels: r.channels.map((c) => c.slice()), slices: Float32Array.from(r.slices) });
      if (n !== this.serial[o] || !this.packed[o]) return;
      sink.loadRecording(o, { channels: r.channels.length, levels: this.packed[o]!.levels, slices: r.slices.length, frames: r.channels[0].length, rate: r.rate, data: this.packed[o]!.data });
      this.sent[o][0] = key;
    } else if (t === TYPE.spectral) {
      const pkey = this.picture[o] ? `pic:${this.picture[o]!.name}:${this.analysis[o]?.frames}` : key;
      if (!pkey) {
        if (this.sent[o][1] !== 'none') sink.loadSpectral(o, null);
        this.sent[o][1] = 'none';
        return;
      }
      if (this.sent[o][1] === pkey) return;
      if (!this.analysis[o] && r) {
        const a = await tools().call({ op: 'specAnalyze', audio: mono(r), sr: r.rate });
        if (n !== this.serial[o]) return;
        this.analysis[o] = { frames: a.frames, rate: r.rate, data: a.data };
        this.emit(o);
      }
      const a = this.analysis[o];
      if (!a) return;
      sink.loadSpectral(o, a);
      this.sent[o][1] = pkey;
    }
  }

  /** Decode an audio file (any format the browser reads) onto an oscillator. */
  async importFile(o: number, file: { name: string; arrayBuffer(): Promise<ArrayBuffer> }): Promise<void> {
    const buf = await decode(await file.arrayBuffer());
    const channels = Array.from({ length: Math.min(2, buf.numberOfChannels) }, (_, c) => buf.getChannelData(c).slice());
    await this.set(o, { name: file.name.replace(/\.[^.]+$/, ''), rate: buf.sampleRate, channels, slices: [] });
  }
}

/** The recording's channels averaged. */
export function mono(r: Recording): Float32Array {
  if (r.channels.length === 1) return r.channels[0].slice();
  const [a, b] = r.channels;
  return Float32Array.from(a, (v, i) => 0.5 * (v + b[i]));
}

/** Decode audio without resampling it (the engine plays any rate). */
export async function decode(bytes: ArrayBuffer): Promise<AudioBuffer> {
  // an offline context at 48 kHz decodes; browsers resample to the context's rate,
  // so decode with a context at the file's own rate when we can find it (WAV)
  const rate = wavRate(bytes) ?? 48_000;
  const ctx = new OfflineAudioContext(1, 1, Math.min(384_000, Math.max(8000, rate)));
  return ctx.decodeAudioData(bytes.slice(0));
}

/** The sample rate in a WAV header, if it is one. */
function wavRate(b: ArrayBuffer): number | null {
  if (b.byteLength < 28) return null;
  const v = new DataView(b);
  if (v.getUint32(0, false) !== 0x52494646 || v.getUint32(8, false) !== 0x57415645) return null;
  let at = 12;
  while (at + 8 <= b.byteLength) {
    const id = v.getUint32(at, false);
    const len = v.getUint32(at + 4, true);
    if (id === 0x666d7420 && at + 16 <= b.byteLength) return v.getUint32(at + 12, true);
    at += 8 + len + (len & 1);
  }
  return null;
}
