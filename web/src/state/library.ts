// The preset library: the factory presets (built in) and the user's own,
// kept in IndexedDB with the wavetables and impulse responses they use,
// stored once each by content hash. Where IndexedDB isn't available (some
// private windows) the library lives in memory for the session.

import { FACTORY } from '../presets/factory';
import { exportFile, importFile, migrate, type Patch, type PatchMeta } from './patch';

export interface Entry {
  /** "factory:<name>" or "user:<uuid>" */
  id: string;
  patch: Patch;
  factory: boolean;
  /** ms since the epoch; 0 for factory presets */
  modified: number;
}

interface PatchRecord {
  id: string;
  patch: Patch;
  modified: number;
}

interface AssetRecord {
  hash: string;
  data: ArrayBuffer;
}

/** The user's changes to a factory preset's metadata (only its rating, for now). */
interface PrefRecord {
  id: string;
  rating: number;
}

type StoreName = 'patches' | 'assets' | 'prefs';

/** Where the library keeps its records. */
export interface Backend {
  all<T>(store: StoreName): Promise<T[]>;
  get<T>(store: StoreName, key: string): Promise<T | undefined>;
  put(store: StoreName, value: object): Promise<void>;
  delete(store: StoreName, key: string): Promise<void>;
}

export class MemoryBackend implements Backend {
  private readonly stores: Record<StoreName, Map<string, object>> = { patches: new Map(), assets: new Map(), prefs: new Map() };
  private key(store: StoreName, v: object): string {
    return (store === 'assets' ? (v as AssetRecord).hash : (v as PatchRecord).id) as string;
  }
  async all<T>(store: StoreName): Promise<T[]> {
    return [...this.stores[store].values()] as T[];
  }
  async get<T>(store: StoreName, key: string): Promise<T | undefined> {
    return this.stores[store].get(key) as T | undefined;
  }
  async put(store: StoreName, value: object): Promise<void> {
    this.stores[store].set(this.key(store, value), structuredClone(value));
  }
  async delete(store: StoreName, key: string): Promise<void> {
    this.stores[store].delete(key);
  }
}

const req = <T>(r: IDBRequest<T>) =>
  new Promise<T>((ok, fail) => {
    r.onsuccess = () => ok(r.result);
    r.onerror = () => fail(r.error);
  });

export class IdbBackend implements Backend {
  private constructor(private readonly db: IDBDatabase) {}

  static async open(name: string, idb: IDBFactory = indexedDB): Promise<IdbBackend> {
    const r = idb.open(name, 1);
    r.onupgradeneeded = () => {
      const db = r.result;
      if (!db.objectStoreNames.contains('patches')) db.createObjectStore('patches', { keyPath: 'id' });
      if (!db.objectStoreNames.contains('assets')) db.createObjectStore('assets', { keyPath: 'hash' });
      if (!db.objectStoreNames.contains('prefs')) db.createObjectStore('prefs', { keyPath: 'id' });
    };
    return new IdbBackend(await req(r));
  }

  private store(name: StoreName, mode: IDBTransactionMode): IDBObjectStore {
    return this.db.transaction(name, mode).objectStore(name);
  }
  all<T>(store: StoreName): Promise<T[]> {
    return req(this.store(store, 'readonly').getAll()) as Promise<T[]>;
  }
  get<T>(store: StoreName, key: string): Promise<T | undefined> {
    return req(this.store(store, 'readonly').get(key)) as Promise<T | undefined>;
  }
  async put(store: StoreName, value: object): Promise<void> {
    await req(this.store(store, 'readwrite').put(value));
  }
  async delete(store: StoreName, key: string): Promise<void> {
    await req(this.store(store, 'readwrite').delete(key));
  }
}

// ------------------------------------------------------------------ search

export type TagState = 'all' | 'any' | 'not';

export interface Query {
  /** Words that must all appear in the name, author, category, tags or notes. */
  text: string;
  /** Tags the preset must have (all), may have (at least one of the "any"), or mustn't. */
  tags: Record<string, TagState>;
  category: string;
  minRating: number;
  source: 'all' | 'factory' | 'user';
  sort: 'name' | 'category' | 'rating' | 'modified';
}

export function emptyQuery(): Query {
  return { text: '', tags: {}, category: '', minRating: 0, source: 'all', sort: 'category' };
}

const CATEGORY_ORDER = ['Init', 'Bass', 'Lead', 'Pad', 'Pluck', 'Keys', 'FX', 'Drums'];
const catRank = (c: string) => {
  const i = CATEGORY_ORDER.indexOf(c);
  return i < 0 ? CATEGORY_ORDER.length : i;
};

export function search(entries: readonly Entry[], q: Query): Entry[] {
  const words = q.text.toLowerCase().split(/\s+/).filter(Boolean);
  const all = Object.keys(q.tags).filter((t) => q.tags[t] === 'all');
  const any = Object.keys(q.tags).filter((t) => q.tags[t] === 'any');
  const not = Object.keys(q.tags).filter((t) => q.tags[t] === 'not');
  const out = entries.filter((e) => {
    const m = e.patch.meta;
    if (q.source !== 'all' && e.factory !== (q.source === 'factory')) return false;
    if (q.category && m.category !== q.category) return false;
    if (m.rating < q.minRating) return false;
    const tags = new Set(m.tags.map((t) => t.toLowerCase()));
    if (!all.every((t) => tags.has(t))) return false;
    if (any.length && !any.some((t) => tags.has(t))) return false;
    if (not.some((t) => tags.has(t))) return false;
    if (words.length) {
      const hay = [m.name, m.author, m.category, m.notes, ...m.tags].join(' ').toLowerCase();
      if (!words.every((w) => hay.includes(w))) return false;
    }
    return true;
  });
  const byName = (a: Entry, b: Entry) => a.patch.meta.name.localeCompare(b.patch.meta.name);
  const sorts: Record<Query['sort'], (a: Entry, b: Entry) => number> = {
    name: byName,
    category: (a, b) => catRank(a.patch.meta.category) - catRank(b.patch.meta.category) || a.patch.meta.category.localeCompare(b.patch.meta.category) || (a.factory === b.factory ? 0 : a.factory ? -1 : 1) || byName(a, b),
    rating: (a, b) => b.patch.meta.rating - a.patch.meta.rating || byName(a, b),
    modified: (a, b) => b.modified - a.modified || byName(a, b),
  };
  return out.sort(sorts[q.sort]);
}

/** Every tag in use, most used first. */
export function tagCloud(entries: readonly Entry[]): { tag: string; count: number }[] {
  const n = new Map<string, number>();
  for (const e of entries) for (const t of e.patch.meta.tags) n.set(t.toLowerCase(), (n.get(t.toLowerCase()) ?? 0) + 1);
  return [...n].map(([tag, count]) => ({ tag, count })).sort((a, b) => b.count - a.count || a.tag.localeCompare(b.tag));
}

// ----------------------------------------------------------------- library

export class Library {
  private user: Entry[] = [];
  private factory: Entry[] = [];
  private readonly subs = new Set<() => void>();
  /** Whether the browser promised not to evict the library (null: not asked yet). */
  persisted: boolean | null = null;

  private constructor(
    private readonly db: Backend,
    /** false when IndexedDB was unavailable and the library lives in memory. */
    readonly durable: boolean,
  ) {}

  static async open(name = 'birdsynth'): Promise<Library> {
    let lib: Library;
    try {
      if (typeof indexedDB === 'undefined') throw new Error('no IndexedDB');
      lib = new Library(await IdbBackend.open(name), true);
    } catch {
      lib = new Library(new MemoryBackend(), false);
    }
    await lib.load();
    return lib;
  }

  /** A library over a given backend (tests). */
  static async over(db: Backend, durable = true): Promise<Library> {
    const lib = new Library(db, durable);
    await lib.load();
    return lib;
  }

  private async load(): Promise<void> {
    const prefs = new Map((await this.db.all<PrefRecord>('prefs')).map((p) => [p.id, p]));
    this.factory = FACTORY.map((p) => {
      const id = `factory:${p.meta.name}`;
      const patch = structuredClone(p);
      patch.meta.rating = prefs.get(id)?.rating ?? 0;
      return { id, patch, factory: true, modified: 0 };
    });
    const recs = await this.db.all<PatchRecord>('patches');
    this.user = [];
    for (const r of recs) {
      try {
        this.user.push({ id: r.id, patch: migrate(r.patch), factory: false, modified: r.modified });
      } catch {
        // a record from a newer build: leave it alone
      }
    }
    this.emit();
  }

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  private emit(): void {
    for (const fn of this.subs) fn();
  }

  get entries(): Entry[] {
    return [...this.factory, ...this.user];
  }

  entry(id: string): Entry | undefined {
    return this.factory.find((e) => e.id === id) ?? this.user.find((e) => e.id === id);
  }

  /** Ask the browser to keep the library (Safari evicts site data otherwise). */
  async persist(): Promise<boolean> {
    try {
      this.persisted = (await navigator.storage?.persist?.()) ?? false;
    } catch {
      this.persisted = false;
    }
    return this.persisted;
  }

  /** Save a patch and its assets: over `id` if it's a user preset, else as a new one. */
  async save(patch: Patch, assets: Map<string, ArrayBuffer>, id?: string): Promise<Entry> {
    for (const [hash, data] of assets) if (!(await this.db.get('assets', hash))) await this.db.put('assets', { hash, data } satisfies AssetRecord);
    const keep = id?.startsWith('user:') && this.user.some((e) => e.id === id);
    const rec: PatchRecord = { id: keep ? id! : `user:${crypto.randomUUID()}`, patch: structuredClone(patch), modified: Date.now() };
    await this.db.put('patches', rec);
    const entry: Entry = { id: rec.id, patch: rec.patch, factory: false, modified: rec.modified };
    const i = this.user.findIndex((e) => e.id === rec.id);
    if (i >= 0) this.user[i] = entry;
    else this.user.push(entry);
    this.emit();
    return entry;
  }

  async remove(id: string): Promise<void> {
    if (!id.startsWith('user:')) return;
    await this.db.delete('patches', id);
    this.user = this.user.filter((e) => e.id !== id);
    this.emit();
  }

  /** Change a preset's metadata (factory presets keep only a rating). */
  async setMeta(id: string, meta: Partial<PatchMeta>): Promise<void> {
    const e = this.entry(id);
    if (!e) return;
    if (e.factory) {
      if (meta.rating === undefined) return;
      e.patch.meta.rating = meta.rating;
      await this.db.put('prefs', { id, rating: meta.rating } satisfies PrefRecord);
    } else {
      e.patch.meta = { ...e.patch.meta, ...meta, tags: [...(meta.tags ?? e.patch.meta.tags)] };
      e.modified = Date.now();
      await this.db.put('patches', { id, patch: e.patch, modified: e.modified } satisfies PatchRecord);
    }
    this.emit();
  }

  async asset(hash: string): Promise<ArrayBuffer | undefined> {
    return (await this.db.get<AssetRecord>('assets', hash))?.data;
  }

  /** A preset as one file, its assets embedded. */
  async exportEntry(id: string): Promise<string> {
    const e = this.entry(id);
    if (!e) throw new Error(`no preset ${id}`);
    const assets = new Map<string, ArrayBuffer>();
    for (const h of hashesOf(e.patch)) {
      const d = await this.asset(h);
      if (d) assets.set(h, d);
    }
    return exportFile(e.patch, assets);
  }

  /** Add a preset file to the library. */
  async importText(text: string): Promise<Entry> {
    const { patch, assets } = importFile(text);
    return this.save(patch, assets);
  }
}

/** The asset hashes a patch refers to. */
export function hashesOf(p: Patch): string[] {
  const out = new Set<string>();
  for (const t of p.tables) if (t?.hash) out.add(t.hash);
  for (const r of p.irs) if (r?.hash) out.add(r.hash);
  return [...out];
}
