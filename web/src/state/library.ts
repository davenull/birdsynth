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

/**
 * The working state, kept between visits: one record in the prefs store, id
 * "session". (Builds of 2026-09-28 21:25–21:40 kept it in a store of its own,
 * "session", id "current", which needed a database upgrade; a record found
 * there is moved over.)
 */
interface SessionRecord {
  id: string;
  patch: Patch;
  /** The preset it came from ("" for none), and whether it had unsaved changes. */
  presetId: string;
  dirty: boolean;
  /** ms since the epoch */
  saved: number;
}

export interface Session {
  patch: Patch;
  presetId: string;
  dirty: boolean;
  saved: number;
}

/** The session record's id in the prefs store. */
const SESSION = 'session';

type StoreName = 'patches' | 'assets' | 'prefs' | 'session';

/** Where the library keeps its records. */
export interface Backend {
  all<T>(store: StoreName): Promise<T[]>;
  keys(store: StoreName): Promise<string[]>;
  get<T>(store: StoreName, key: string): Promise<T | undefined>;
  put(store: StoreName, value: object): Promise<void>;
  delete(store: StoreName, key: string): Promise<void>;
}

export class MemoryBackend implements Backend {
  private readonly stores: Record<StoreName, Map<string, object>> = { patches: new Map(), assets: new Map(), prefs: new Map(), session: new Map() };
  private key(store: StoreName, v: object): string {
    return (store === 'assets' ? (v as AssetRecord).hash : (v as PatchRecord).id) as string;
  }
  async all<T>(store: StoreName): Promise<T[]> {
    return [...this.stores[store].values()] as T[];
  }
  async keys(store: StoreName): Promise<string[]> {
    return [...this.stores[store].keys()];
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

  /**
   * Open the library's database at whatever version this browser has, never
   * asking for an upgrade: an upgrade has to wait for every other tab with the
   * database open (an older build's never lets go), and until then the
   * library can't be read at all. A new database starts at version 1 with the
   * three stores; version 2 (briefly, above) added a "session" store, which
   * is read if it's there.
   */
  static async open(name: string, idb: IDBFactory = indexedDB): Promise<IdbBackend> {
    const r = idb.open(name);
    r.onupgradeneeded = () => {
      const db = r.result;
      if (!db.objectStoreNames.contains('patches')) db.createObjectStore('patches', { keyPath: 'id' });
      if (!db.objectStoreNames.contains('assets')) db.createObjectStore('assets', { keyPath: 'hash' });
      if (!db.objectStoreNames.contains('prefs')) db.createObjectStore('prefs', { keyPath: 'id' });
    };
    const db = await req(r);
    // should a later build ever need an upgrade, step aside for it rather than block it
    db.onversionchange = () => db.close();
    return new IdbBackend(db);
  }

  private has(name: StoreName): boolean {
    return this.db.objectStoreNames.contains(name);
  }
  private store(name: StoreName, mode: IDBTransactionMode): IDBObjectStore {
    return this.db.transaction(name, mode).objectStore(name);
  }
  async all<T>(store: StoreName): Promise<T[]> {
    return this.has(store) ? ((await req(this.store(store, 'readonly').getAll())) as T[]) : [];
  }
  async keys(store: StoreName): Promise<string[]> {
    return this.has(store) ? ((await req(this.store(store, 'readonly').getAllKeys())) as string[]) : [];
  }
  async get<T>(store: StoreName, key: string): Promise<T | undefined> {
    return this.has(store) ? ((await req(this.store(store, 'readonly').get(key))) as T | undefined) : undefined;
  }
  async put(store: StoreName, value: object): Promise<void> {
    await req(this.store(store, 'readwrite').put(value));
  }
  async delete(store: StoreName, key: string): Promise<void> {
    if (this.has(store)) await req(this.store(store, 'readwrite').delete(key));
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
    category: (a, b) =>
      catRank(a.patch.meta.category) - catRank(b.patch.meta.category) ||
      a.patch.meta.category.localeCompare(b.patch.meta.category) ||
      (a.factory === b.factory ? 0 : a.factory ? -1 : 1) ||
      byName(a, b),
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
  /** Writes run one at a time, so a clean-up never sees a preset half saved. */
  private queue: Promise<unknown> = Promise.resolve();
  /** The asset hashes stored (loaded on first need). */
  private known: Set<string> | null = null;
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

  private serial<T>(fn: () => Promise<T>): Promise<T> {
    const run = this.queue.then(fn, fn);
    this.queue = run.catch(() => {});
    return run;
  }

  /** Store the assets not stored yet (each once, by hash). `data` can be a function, so only new ones are copied. */
  private async putAssets(assets: Map<string, ArrayBuffer | (() => ArrayBuffer)>): Promise<void> {
    this.known ??= new Set(await this.db.keys('assets'));
    for (const [hash, data] of assets) {
      if (this.known.has(hash)) continue;
      await this.db.put('assets', { hash, data: typeof data === 'function' ? data() : data } satisfies AssetRecord);
      this.known.add(hash);
    }
  }

  /**
   * Delete the assets nothing refers to any more (deleted presets, earlier
   * states of the session). What's in use is read from the stored records,
   * not this tab's lists, so another tab's presets are safe; if any record
   * can't be read, nothing is deleted.
   */
  private async collect(): Promise<number> {
    const used = new Set<string>();
    try {
      for (const r of await this.db.all<PatchRecord>('patches')) for (const h of hashesOf(migrate(r.patch))) used.add(h);
      for (const s of [await this.db.get<SessionRecord>('prefs', SESSION), await this.db.get<SessionRecord>('session', 'current')]) if (s) for (const h of hashesOf(migrate(s.patch))) used.add(h);
    } catch {
      return 0;
    }
    this.known = new Set(await this.db.keys('assets'));
    let n = 0;
    for (const h of [...this.known]) {
      if (used.has(h)) continue;
      await this.db.delete('assets', h);
      this.known.delete(h);
      n++;
    }
    return n;
  }

  /** Clear out assets nothing uses (at startup; also after a delete or a reset). */
  collectGarbage(): Promise<number> {
    return this.serial(() => this.collect());
  }

  /** Save a patch and its assets: over `id` if it's a user preset, else as a new one. */
  save(patch: Patch, assets: Map<string, ArrayBuffer>, id?: string): Promise<Entry> {
    return this.serial(() => this.saveNow(patch, assets, id));
  }

  private async saveNow(patch: Patch, assets: Map<string, ArrayBuffer>, id?: string): Promise<Entry> {
    await this.putAssets(assets);
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

  remove(id: string): Promise<void> {
    if (!id.startsWith('user:')) return Promise.resolve();
    return this.serial(async () => {
      await this.db.delete('patches', id);
      this.user = this.user.filter((e) => e.id !== id);
      this.emit();
      await this.collect();
    });
  }

  // ---------------------------------------------------------------- session

  /** The working state saved last time, or null (a record in the short-lived session store is moved to prefs). */
  loadSession(): Promise<Session | null> {
    return this.serial(async () => {
      let r = await this.db.get<SessionRecord>('prefs', SESSION);
      const legacy = await this.db.get<SessionRecord>('session', 'current');
      if (legacy) {
        if (!r || legacy.saved > r.saved) {
          r = { ...legacy, id: SESSION };
          await this.db.put('prefs', r);
        }
        await this.db.delete('session', 'current');
      }
      if (!r) return null;
      return { patch: migrate(r.patch), presetId: r.presetId, dirty: r.dirty, saved: r.saved };
    });
  }

  /** Keep the working state: its assets (the new ones), then the record. */
  saveSession(patch: Patch, assets: Map<string, ArrayBuffer | (() => ArrayBuffer)>, presetId: string, dirty: boolean): Promise<void> {
    return this.serial(async () => {
      await this.putAssets(assets);
      await this.db.put('prefs', { id: SESSION, patch, presetId, dirty, saved: Date.now() } satisfies SessionRecord);
    });
  }

  /** Forget the working state (after a reset, the next visit starts fresh). */
  clearSession(): Promise<void> {
    return this.serial(async () => {
      await this.db.delete('prefs', SESSION);
      await this.db.delete('session', 'current');
      await this.collect();
    });
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
  for (const r of p.recordings ?? []) if (r?.hash) out.add(r.hash);
  for (const m of p.multis ?? []) if (m?.hash) out.add(m.hash);
  return [...out];
}
