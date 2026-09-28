// Multisamples for the oscillators' Multisample type: a factory instrument
// (played by the tools' physical models) or an SFZ file with its samples.
// Either way the engine gets zone headers and the zones' audio in one
// asset (crates/engine/src/osc/multi.rs); the store keeps it for a
// restarted engine, and for saving with a patch.

import { findSample, parseSfz } from '../sfz/sfz';
import { tools } from '../tools/client';
import { decode } from './recordings';

export const ZONE_FLOATS = 16;
export const MAX_ZONES = 128;

export interface ZoneInfo {
  sample: string;
  lokey: number;
  hikey: number;
  lovel: number;
  hivel: number;
  root: number;
}

export interface MultiSet {
  name: string;
  /** "factory:<name>" or "sfz:<file name>" */
  source: string;
  zones: ZoneInfo[];
  /** Zone headers, then audio: what the engine loads. */
  data: Float32Array;
  warnings: string[];
}

export interface MultiSink {
  loadMulti(osc: number, m: { zones: number; data: Float32Array } | null): void;
}

export class MultiStore {
  readonly osc: (MultiSet | null)[] = [null, null, null];
  private sink: MultiSink | null = null;
  private rate = 48_000;
  private readonly subs = new Set<(osc: number) => void>();
  private list: Promise<string[]> | null = null;

  subscribe(fn: (osc: number) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  attach(sink: MultiSink | null, sampleRate: number): void {
    this.sink = sink;
    this.rate = sampleRate;
  }

  resync(): void {
    this.osc.forEach((m, o) => this.sink?.loadMulti(o, m ? { zones: m.zones.length, data: m.data } : null));
  }

  factoryList(): Promise<string[]> {
    return (this.list ??= tools().call({ op: 'multiList' }));
  }

  set(o: number, m: MultiSet | null): void {
    this.osc[o] = m;
    this.sink?.loadMulti(o, m ? { zones: m.zones.length, data: m.data } : null);
    for (const fn of this.subs) fn(o);
  }

  async loadFactory(o: number, name: string): Promise<void> {
    const list = await this.factoryList();
    const index = list.indexOf(name);
    if (index < 0) throw new Error(`no factory multisample "${name}"`);
    const { data, zones } = await tools().call({ op: 'multiFactory', index, sr: this.rate });
    this.set(o, { name, source: `factory:${name}`, zones: zoneInfo(data, zones, name), data, warnings: [] });
  }

  /** An SFZ file and its samples (the files picked or dropped with it). */
  async loadSfz(o: number, sfz: { name: string; text(): Promise<string> }, files: { name: string; path?: string; arrayBuffer(): Promise<ArrayBuffer> }[]): Promise<MultiSet> {
    const parsed = parseSfz(await sfz.text());
    const warnings = [...parsed.warnings];
    const names = files.map((f) => f.path ?? f.name);
    const decoded = new Map<string, { channels: Float32Array[]; rate: number }>();
    const regions = parsed.regions.slice(0, MAX_ZONES);
    if (parsed.regions.length > MAX_ZONES) warnings.push(`only the first ${MAX_ZONES} of ${parsed.regions.length} regions are used`);
    const parts: { info: ZoneInfo; header: number[]; audio: Float32Array[] }[] = [];
    for (const r of regions) {
      const at = findSample(r.sample, names);
      if (at === null) {
        warnings.push(`missing sample ${r.sample}`);
        continue;
      }
      if (!decoded.has(at)) {
        const f = files[names.indexOf(at)];
        const buf = await decode(await f.arrayBuffer());
        decoded.set(at, { rate: buf.sampleRate, channels: Array.from({ length: Math.min(2, buf.numberOfChannels) }, (_, c) => buf.getChannelData(c).slice()) });
      }
      const d = decoded.get(at)!;
      const from = Math.max(0, Math.min(r.offset, d.channels[0].length - 4));
      const to = Math.max(from + 4, Math.min(r.end ?? d.channels[0].length, d.channels[0].length));
      const audio = d.channels.map((c) => c.subarray(from, to));
      const frames = to - from;
      const ls = r.loopStart === null ? 0 : r.loopStart - from;
      const le = r.loopEnd === null ? frames : r.loopEnd - from;
      parts.push({
        info: { sample: r.sample, lokey: r.lokey, hikey: r.hikey, lovel: r.lovel, hivel: r.hivel, root: r.root },
        header: [0, frames, audio.length, d.rate, r.root, r.lokey, r.hikey, r.lovel, r.hivel, r.loopMode, Math.max(0, ls), Math.min(frames, le), r.gain, r.pan, r.oneShot ? 1 : 0, 0],
        audio,
      });
    }
    if (!parts.length) throw new Error(warnings[0] ?? 'the SFZ has no regions');
    const head = parts.length * ZONE_FLOATS;
    const total = parts.reduce((a, p) => a + p.header[1] * p.audio.length, 0);
    const data = new Float32Array(head + total);
    let off = 0;
    parts.forEach((p, z) => {
      p.header[0] = off;
      data.set(p.header, z * ZONE_FLOATS);
      for (const c of p.audio) {
        data.set(c, head + off);
        off += c.length;
      }
    });
    const m: MultiSet = { name: sfz.name.replace(/\.sfz$/i, ''), source: `sfz:${sfz.name}`, zones: parts.map((p) => p.info), data, warnings };
    this.set(o, m);
    return m;
  }
}

/** The zones described by a packed multisample's headers. */
export function zoneInfo(data: Float32Array, zones: number, name: string): ZoneInfo[] {
  return Array.from({ length: zones }, (_, z) => {
    const h = data.subarray(z * ZONE_FLOATS, (z + 1) * ZONE_FLOATS);
    return { sample: `${name} ${z + 1}`, root: h[4], lokey: h[5], hikey: h[6], lovel: h[7], hivel: h[8] };
  });
}
