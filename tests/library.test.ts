// The preset library (IndexedDB, through fake-indexeddb here), its search,
// and undo/redo.

import { IDBFactory } from 'fake-indexeddb';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PARAM_ID } from '../web/src/gen/params';
import { FACTORY } from '../web/src/presets/factory';
import { History } from '../web/src/state/history';
import { IdbBackend, Library, emptyQuery, search, tagCloud, type Entry } from '../web/src/state/library';
import { SOURCE } from '../web/src/state/matrix';
import { capture, emptyMeta, hashBytes } from '../web/src/state/patch';
import { stores } from './patch-kit';

describe('library', () => {
  it('keeps user presets and their assets across sessions, once per hash', async () => {
    const idb = new IDBFactory();
    const lib = await Library.over(await IdbBackend.open('t', idb));
    expect(lib.entries.length).toBe(FACTORY.length);
    const t = stores();
    t.bank.set(PARAM_ID['filter.1.enable'], 1);
    const frames = new Float32Array(2048 * 2).map((_, i) => Math.sin(i / 50));
    const hash = await hashBytes(frames);
    const p = capture(t, { ...emptyMeta('Mine'), tags: ['dark', 'bass'], category: 'Bass', rating: 4 });
    p.tables[0] = { name: 'drawn', source: 'file:drawn.wav', count: 2, hash };
    const a = await lib.save(p, new Map([[hash, frames.buffer]]));
    const b = await lib.save({ ...p, meta: { ...p.meta, name: 'Mine 2' } }, new Map([[hash, frames.buffer]]));
    expect(a.id).not.toBe(b.id);
    expect(a.id.startsWith('user:')).toBe(true);
    // saving over a user preset keeps its id
    const a2 = await lib.save({ ...p, meta: { ...p.meta, notes: 'edited' } }, new Map(), a.id);
    expect(a2.id).toBe(a.id);
    await lib.setMeta('factory:Reese', { rating: 5 });

    const again = await Library.over(await IdbBackend.open('t', idb));
    expect(again.entries.length).toBe(FACTORY.length + 2);
    expect(again.entry(a.id)?.patch.meta.notes).toBe('edited');
    expect(again.entry('factory:Reese')?.patch.meta.rating).toBe(5);
    const back = new Float32Array((await again.asset(hash))!);
    expect(back).toEqual(frames);

    // a single-file export carries its assets; importing it makes a new preset
    const text = await again.exportEntry(a.id);
    const idb2 = new IDBFactory();
    const other = await Library.over(await IdbBackend.open('u', idb2));
    const imported = await other.importText(text);
    expect(imported.patch.meta.name).toBe('Mine');
    expect(imported.patch.params).toEqual(again.entry(a.id)!.patch.params);
    expect(new Float32Array((await other.asset(hash))!)).toEqual(frames);

    await again.remove(b.id);
    expect((await Library.over(await IdbBackend.open('t', idb))).entries.length).toBe(FACTORY.length + 1);
    // factory presets can't be removed
    await again.remove('factory:Reese');
    expect(again.entry('factory:Reese')).toBeDefined();
  });

  it('keeps the session between visits and clears out assets nothing uses', async () => {
    const idb = new IDBFactory();
    const lib = await Library.over(await IdbBackend.open('s', idb));
    expect(await lib.loadSession()).toBeNull();
    const t = stores();
    t.bank.set(PARAM_ID['filter.1.cutoff'], 0.3);
    const table = (f: (i: number) => number) => new Float32Array(2048).map((_, i) => f(i));
    const [tableA, tableB, tableC] = [table((i) => Math.sin(i / 30)), table((i) => Math.cos(i / 20)), table(() => 0.25)];
    const [ha, hb, hc] = await Promise.all([tableA, tableB, tableC].map((x) => hashBytes(x)));
    const withTable = (h: string) => {
      const p = capture(t, emptyMeta('Working'));
      p.tables[1] = { name: 'mine', source: 'file:mine.wav', count: 1, hash: h };
      return p;
    };
    // a user preset uses table A; the session uses table B
    const kept = await lib.save(withTable(ha), new Map([[ha, tableA.buffer]]));
    let copies = 0;
    const lazyB = new Map([[hb, () => (copies++, tableB.buffer)]]);
    await lib.saveSession(withTable(hb), lazyB, kept.id, true);
    await lib.saveSession(withTable(hb), lazyB, kept.id, true);
    expect(copies, 'an asset already stored is not copied again').toBe(1);

    // the next visit
    const again = await Library.over(await IdbBackend.open('s', idb));
    const s = await again.loadSession();
    expect(s).toMatchObject({ presetId: kept.id, dirty: true });
    expect(s!.patch.params['filter.1.cutoff']).toBeCloseTo(0.3);
    expect(new Float32Array((await again.asset(hb))!)).toEqual(tableB);

    // the session moves on to another table: nothing uses B now
    await again.saveSession(withTable(hc), new Map([[hc, () => tableC.buffer]]), '', false);
    expect(await again.collectGarbage()).toBe(1);
    expect(await again.asset(hb)).toBeUndefined();
    expect(await again.asset(ha), 'a preset still uses A').toBeDefined();
    expect(await again.asset(hc)).toBeDefined();
    // a reset forgets the session and what only it used
    await again.clearSession();
    expect(await again.loadSession()).toBeNull();
    expect(await again.asset(hc)).toBeUndefined();
    expect(await again.asset(ha)).toBeDefined();
    // deleting the preset frees its table too
    await again.remove(kept.id);
    expect(await again.asset(ha)).toBeUndefined();
  });

  it('opens a library from before sessions, and leaves a session it can’t read alone', async () => {
    const idb = new IDBFactory();
    // the version-1 database: presets, assets and prefs, no session store
    await new Promise<void>((ok, fail) => {
      const r = idb.open('old', 1);
      r.onupgradeneeded = () => {
        const db = r.result;
        db.createObjectStore('patches', { keyPath: 'id' }).put({ id: 'user:1', patch: capture(stores(), emptyMeta('Old one')), modified: 1 });
        db.createObjectStore('assets', { keyPath: 'hash' }).put({ hash: 'abc', data: new ArrayBuffer(8) });
        db.createObjectStore('prefs', { keyPath: 'id' });
      };
      r.onsuccess = () => (r.result.close(), ok());
      r.onerror = () => fail(r.error);
    });
    const backend = await IdbBackend.open('old', idb);
    const lib = await Library.over(backend);
    expect(lib.entry('user:1')?.patch.meta.name).toBe('Old one');
    expect(await lib.loadSession()).toBeNull();
    // a session saved by a newer build: loading it fails, and the clean-up deletes nothing
    await backend.put('session', { id: 'current', patch: { ...capture(stores(), emptyMeta('Future')), version: 999 }, presetId: '', dirty: false, saved: 1 });
    await expect(lib.loadSession()).rejects.toThrow(/newer/);
    expect(await lib.collectGarbage()).toBe(0);
    expect(await lib.asset('abc')).toBeDefined();
  });

  it('searches by words, tags (all, any, not), category, rating and source', () => {
    const e = (name: string, category: string, tags: string[], rating = 0, factory = true): Entry => ({
      id: `${factory ? 'factory' : 'user'}:${name}`,
      patch: { ...capture(stores()), meta: { ...emptyMeta(name), category, tags, rating } },
      factory,
      modified: 0,
    });
    const all = [e('Deep Sub', 'Bass', ['sub', 'dark']), e('Glass', 'Pad', ['bright', 'wide'], 3), e('Night Drive', 'Lead', ['dark', 'wide'], 5, false), e('Wub', 'Bass', ['wobble', 'dark'], 2)];
    const q = emptyQuery();
    const names = (qq: Partial<typeof q>) => search(all, { ...q, ...qq }).map((x) => x.patch.meta.name);
    expect(names({})).toEqual(['Deep Sub', 'Wub', 'Night Drive', 'Glass']); // by category, then name
    expect(names({ text: 'dri nig' })).toEqual(['Night Drive']);
    expect(names({ text: 'bass' })).toEqual(['Deep Sub', 'Wub']);
    expect(names({ tags: { dark: 'all', wide: 'all' } })).toEqual(['Night Drive']);
    expect(names({ tags: { sub: 'any', wobble: 'any' } })).toEqual(['Deep Sub', 'Wub']);
    expect(names({ tags: { dark: 'not' } })).toEqual(['Glass']);
    expect(names({ tags: { dark: 'all', wobble: 'not' } })).toEqual(['Deep Sub', 'Night Drive']);
    expect(names({ category: 'Bass', minRating: 1 })).toEqual(['Wub']);
    expect(names({ source: 'user' })).toEqual(['Night Drive']);
    expect(names({ sort: 'rating' })).toEqual(['Night Drive', 'Glass', 'Wub', 'Deep Sub']);
    expect(tagCloud(all)[0]).toEqual({ tag: 'dark', count: 3 });
  });

  it('falls back to memory when IndexedDB is missing', async () => {
    const lib = await Library.open('x');
    expect(lib.durable).toBe(false);
    const e = await lib.save(capture(stores()), new Map());
    expect(lib.entry(e.id)).toBeDefined();
  });
});

describe('undo', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('makes a gesture one step, restores every store and knows when the preset is clean', () => {
    const t = stores();
    const h = new History(t, 400);
    const cut = PARAM_ID['filter.1.cutoff'];
    expect(h.canUndo).toBe(false);
    for (let i = 0; i <= 50; i++) {
      t.bank.set(cut, 0.5 + i / 200); // a knob drag
      vi.advanceTimersByTime(16);
    }
    expect(h.dirty).toBe(true);
    vi.advanceTimersByTime(400);
    expect(h.depth).toBe(1);
    t.matrix.add(SOURCE['LFO 1'], cut, 0.3);
    t.fx.add(0, 5);
    vi.advanceTimersByTime(400);
    expect(h.depth).toBe(2);
    h.undo();
    expect(t.matrix.used).toBe(0);
    expect(t.fx.chains[0].length).toBe(0);
    expect(t.bank.get(cut)).toBeCloseTo(0.75, 6);
    h.undo();
    expect(t.bank.get(cut)).toBeCloseTo(new (t.bank.constructor as new () => typeof t.bank)().get(cut), 6);
    expect(h.dirty).toBe(false);
    h.redo();
    h.redo();
    expect(t.matrix.used).toBe(1);
    expect(t.fx.chains[0].length).toBe(1);
    // a change after an undo drops the redo steps
    h.undo();
    t.bank.set(cut, 0.1);
    h.commit();
    expect(h.canRedo).toBe(false);
    h.markClean();
    expect(h.dirty).toBe(false);
    // undo doesn't record itself
    h.undo();
    vi.advanceTimersByTime(1000);
    expect(h.canRedo).toBe(true);
  });
});
