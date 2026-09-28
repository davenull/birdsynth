// The tuning the whole synth plays in: standard, or a table from a .scl
// (with an optional .kbm) or a .tun file. Like MIDI learn it belongs to the
// setup, not to a patch, so it's kept in localStorage and survives preset
// changes and reloads.

import { parseKbm, parseScl, parseTun, standardTable, tableFromScale, TuningError } from '../tuning/tuning';

export interface TuningSink {
  setTuning(table: Float32Array): void;
}

const KEY = 'birdsynth.tuning';

export class TuningStore {
  name = 'Standard (12-TET)';
  /** null: standard tuning. */
  table: Float32Array | null = null;
  private sink: TuningSink | null = null;
  private readonly subs = new Set<() => void>();

  constructor(private readonly storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null = typeof localStorage === 'undefined' ? null : localStorage) {
    try {
      const raw = JSON.parse(this.storage?.getItem(KEY) ?? 'null') as { name: string; table: number[] } | null;
      if (raw && Array.isArray(raw.table) && raw.table.length === 128 && raw.table.every(Number.isFinite)) {
        this.name = raw.name;
        this.table = Float32Array.from(raw.table);
      }
    } catch {
      // unreadable: standard tuning
    }
  }

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  attach(sink: TuningSink | null): void {
    this.sink = sink;
  }

  resync(): void {
    this.sink?.setTuning(this.table ?? standardTable());
  }

  set(name: string, table: Float32Array | null): void {
    this.name = table ? name : 'Standard (12-TET)';
    this.table = table;
    try {
      if (table) this.storage?.setItem(KEY, JSON.stringify({ name, table: Array.from(table) }));
      else this.storage?.removeItem(KEY);
    } catch {
      // storage blocked: the tuning lasts for the session
    }
    this.resync();
    for (const fn of this.subs) fn();
  }

  reset(): void {
    this.set('', null);
  }

  /** Use dropped or picked files: a .tun, or a .scl with an optional .kbm. */
  async load(files: { name: string; text(): Promise<string> }[]): Promise<void> {
    const by = (ext: string) => files.find((f) => f.name.toLowerCase().endsWith(ext));
    const tun = by('.tun');
    const scl = by('.scl');
    const kbm = by('.kbm');
    if (tun) {
      const { name, table } = parseTun(await tun.text());
      return this.set(name || tun.name.replace(/\.tun$/i, ''), table);
    }
    if (!scl) throw new TuningError(kbm ? 'a .kbm needs a .scl scale to map (drop or pick both together)' : 'pick a .scl (with an optional .kbm) or a .tun file');
    const scale = parseScl(await scl.text());
    const map = kbm ? parseKbm(await kbm.text()) : undefined;
    const name = `${scale.description || scl.name.replace(/\.scl$/i, '')}${kbm ? ` · ${kbm.name.replace(/\.kbm$/i, '')}` : ''}`;
    this.set(name, tableFromScale(scale, map));
  }
}
