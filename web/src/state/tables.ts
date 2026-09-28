// The wavetable each oscillator plays. The store keeps the raw frames (for
// display and export) and the mip-mapped data (for the engine), so a
// restarted engine can be given its tables again straight away.

import { CONST } from '../gen/protocol';
import { tools } from '../tools/client';
import { parseWav, writeWavetable } from '../tables/wav';

export interface OscTable {
  name: string;
  /** e.g. "factory:Basic Shapes" or "file:growl.wav" */
  source: string;
  /** count × 2048 samples. */
  frames: Float32Array;
  count: number;
  /** count × frameStride floats, as the engine reads them. */
  mips: Float32Array;
}

export interface TableSink {
  loadTable(osc: number, mips: Float32Array, frames: number): void;
}

type Sub = (osc: number, t: OscTable) => void;

export class TableStore {
  readonly osc: (OscTable | null)[] = Array.from({ length: CONST.oscCount }, () => null);
  private readonly subs = new Set<Sub>();
  private sink: TableSink | null = null;
  private factory: Promise<{ name: string; frames: number }[]> | null = null;

  onChange(fn: Sub): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  /** Where tables go once they're built (the engine host). */
  attach(sink: TableSink | null): void {
    this.sink = sink;
  }

  /** Send every table to the engine again (after it restarted). */
  resync(): void {
    this.osc.forEach((t, i) => t && this.sink?.loadTable(i, t.mips, t.count));
  }

  factoryList(): Promise<{ name: string; frames: number }[]> {
    return (this.factory ??= tools().call({ op: 'factoryList' }));
  }

  async loadFactory(osc: number, name: string): Promise<void> {
    const list = await this.factoryList();
    const index = list.findIndex((t) => t.name === name);
    if (index < 0) throw new Error(`no factory table "${name}"`);
    const { frames, count } = await tools().call({ op: 'factory', index });
    await this.set(osc, { name, source: `factory:${name}`, frames, count });
  }

  /** Install raw frames (count × 2048) on an oscillator. */
  async set(osc: number, t: { name: string; source: string; frames: Float32Array; count: number }): Promise<void> {
    const count = Math.max(1, Math.min(CONST.maxFrames, t.count));
    const frames = t.frames.subarray(0, count * CONST.frameLen);
    const mips = await tools().call({ op: 'mips', frames: frames.slice(), count });
    const table: OscTable = { name: t.name, source: t.source, frames: frames.slice(), count, mips };
    this.osc[osc] = table;
    this.sink?.loadTable(osc, mips, count);
    for (const fn of this.subs) fn(osc, table);
  }

  /** Import a wavetable WAV. Files without a clm chunk are read as 2048-sample frames. */
  async importWav(osc: number, name: string, buf: ArrayBuffer): Promise<void> {
    const wav = parseWav(buf);
    const size = wav.clm?.frameSize ?? CONST.frameLen;
    const count = Math.max(1, Math.min(CONST.maxFrames, Math.floor(wav.samples.length / size)));
    let frames: Float32Array;
    if (size === CONST.frameLen) {
      frames = new Float32Array(count * CONST.frameLen);
      frames.set(wav.samples.subarray(0, frames.length));
    } else {
      frames = new Float32Array(count * CONST.frameLen);
      for (let f = 0; f < count; f++) {
        const cyc = wav.samples.slice(f * size, (f + 1) * size);
        frames.set(await tools().call({ op: 'resample', cycle: cyc }), f * CONST.frameLen);
      }
    }
    await this.set(osc, { name: name.replace(/\.wav$/i, ''), source: `file:${name}`, frames, count });
  }

  /** The oscillator's table as a wavetable WAV. */
  exportWav(osc: number): Blob | null {
    const t = this.osc[osc];
    if (!t) return null;
    return new Blob([writeWavetable(t.frames, CONST.frameLen, 1)], { type: 'audio/wav' });
  }
}
