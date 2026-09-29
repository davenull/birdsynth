// The explainer's content: every data-explain key the UI can render has
// notes, and every "try this" change runs against the real stores.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { PARAMS, PARAM_ID, type ParamKey } from '../web/src/gen/params';
import { CONTENT, explain } from '../web/src/explain/content';
import { TOURS } from '../web/src/explain/tours';
import { ParamBank } from '../web/src/state/bank';
import { FX_TYPES, FxRacks } from '../web/src/state/fx';
import { ModMatrix } from '../web/src/state/matrix';
import { TuningStore } from '../web/src/state/tuning';
import type { Synth } from '../web/src/synth';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function svelteFiles(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? svelteFiles(path.join(dir, e.name)) : e.name.endsWith('.svelte') ? [path.join(dir, e.name)] : []));
}

/** What each template in the markup can expand to. */
const EXPANSIONS: Record<string, string[]> = {
  '`osc.${L}`': ['osc.a', 'osc.b', 'osc.c'],
  '`filter.${n}`': ['filter.1', 'filter.2'],
  '`mix.${s.key}`': ['mix.osc.a', 'mix.osc.b', 'mix.osc.c', 'mix.sub', 'mix.noise'],
  '`${s.key}`': ['osc.a', 'osc.b', 'osc.c', 'sub', 'noise'],
  "k === 'f1' ? 'filter.1' : 'filter.2'": ['filter.1', 'filter.2'],
  '`fx.${FX_TYPES[r.type].key}`': FX_TYPES.map((t) => `fx.${t.key}`),
  'n.explain': ['osc.a', 'osc.b', 'osc.c', 'sub', 'noise', 'filter.1', 'filter.2', 'flow.amp', 'flow.buses', 'fx', 'master'], // the FLOW page's nodes
  param: [], // knobs, selects and toggles: parameter keys, checked below
};

describe('explainer', () => {
  it('has notes for every data-explain key in the UI', () => {
    const missing: string[] = [];
    const unknown: string[] = [];
    for (const f of svelteFiles(path.join(ROOT, 'web/src'))) {
      const src = fs.readFileSync(f, 'utf8');
      for (const m of src.matchAll(/(?<=\s)data-explain=(?:"([^"]+)"|\{(.+?)\}(?=[\s>/]))/g)) {
        const keys = m[1] ? [m[1]] : EXPANSIONS[m[2].trim()];
        if (!keys) unknown.push(`${path.relative(ROOT, f)}: data-explain={${m[2]}}`);
        for (const k of keys ?? []) if (!explain(k)) missing.push(`${path.relative(ROOT, f)}: ${k}`);
      }
    }
    expect(unknown, 'add new templates to EXPANSIONS').toEqual([]);
    expect(missing).toEqual([]);
    // every parameter explains itself (knobs use the parameter's key)
    expect(PARAMS.filter((p) => !p.explain).map((p) => p.key)).toEqual([]);
  });

  it('runs every "try this" against the stores', async () => {
    const bank = new ParamBank();
    const store = new Map<string, string>();
    const fake = {
      bank,
      setParam: (key: ParamKey, v: number) => {
        if (PARAM_ID[key] === undefined) throw new Error(`no parameter ${key}`);
        bank.set(PARAM_ID[key], v);
      },
      fx: new FxRacks(bank),
      matrix: new ModMatrix(),
      tuning: new TuningStore({ getItem: (k) => store.get(k) ?? null, setItem: (k, v) => void store.set(k, v), removeItem: (k) => void store.delete(k) }),
    } as unknown as Synth;
    let n = 0;
    for (const [key, e] of Object.entries(CONTENT)) {
      expect(e.title, key).toBeTruthy();
      expect(e.text.length, key).toBeGreaterThan(40);
      for (const t of e.tries ?? []) {
        await t.run(fake);
        n++;
      }
    }
    expect(n).toBeGreaterThan(20);
    expect(fake.matrix.used).toBeGreaterThan(0);
  });

  it('points tours at parts that exist', () => {
    for (const t of TOURS) {
      for (const s of t.steps) if (s.target) expect(explain(s.target), `${t.id}: ${s.title}`).not.toBeNull();
    }
  });
});
