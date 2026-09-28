<!--
  FX page: the three racks (Main, Bus 1, Bus 2). Pick a rack, add and order
  its effects on the left, edit the selected one on the right. Rack presets
  store the whole rack (the order and every module's settings).
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { RACK_NAMES, MAIN, BUS1, SPLITTER, bandChain, moduleParams, refParam, type FxRef } from '../../state/fx';
  import { PARAM_ID } from '../../gen/params';
  import type { Synth } from '../../synth';
  import Select from '../primitives/Select.svelte';
  import ModuleList from '../fx/ModuleList.svelte';
  import ModulePanel from '../fx/ModulePanel.svelte';

  const synth = getContext<Synth>('synth');
  let rack = $state(MAIN);
  let selected = $state<FxRef | null>(null);
  // keep the selection valid when modules go away
  onMount(() =>
    synth.fx.subscribe(() => {
      if (selected && !synth.fx.used(selected.type, selected.inst)) selected = null;
    }),
  );

  // rack presets: factory ones, plus the user's in local storage
  const STORE = 'birdsynth.rackPresets';
  type Snap = ReturnType<Synth['fx']['snapshot']>;
  const FACTORY: Record<string, (s: Synth) => void> = {
    Empty: (s) => s.fx.set(rack, []),
    'Wide Pad': (s) => {
      s.fx.set(rack, []);
      const h = s.fx.add(rack, 0);
      const c = s.fx.add(rack, 4);
      const r = s.fx.add(rack, 7);
      if (h) s.fx.applyPreset(h, { hyper_mix: 0.4, dim_mix: 0.5, size: 0.7 });
      if (c) s.fx.applyPreset(c, { rate: 0.4, depth: 0.4, mix: 0.4 });
      if (r) s.fx.applyPreset(r, { algo: 3, size: 0.8, decay: 5, mix: 0.35 });
    },
    'Lo-Fi': (s) => {
      s.fx.set(rack, []);
      const d = s.fx.add(rack, 1);
      const e = s.fx.add(rack, 8);
      const dl = s.fx.add(rack, 5);
      if (d) s.fx.applyPreset(d, { mode: 8, drive: 0.4 });
      if (e) s.fx.applyPreset(e, { low_type: 2, low_freq: 300, high_type: 2, high_freq: 4000 });
      if (dl) s.fx.applyPreset(dl, { bpm: 1, sync_l: 7, link: 1, feedback: 0.4, freq: 900, width: 0.4, mix: 0.25 });
    },
    'Glue and Space': (s) => {
      s.fx.set(rack, []);
      const cp = s.fx.add(rack, 6);
      const e = s.fx.add(rack, 8);
      const r = s.fx.add(rack, 7);
      if (cp) s.fx.applyPreset(cp, { threshold: -18, ratio: 2, attack: 30, release: 200, knee: 9, gain: 3 });
      if (e) s.fx.applyPreset(e, { low_gain: 2, high_gain: 2 });
      if (r) s.fx.applyPreset(r, { algo: 1, decay: 1.8, mix: 0.18 });
    },
  };
  let user = $state<Record<string, Snap>>({});
  onMount(() => {
    try {
      user = JSON.parse(localStorage.getItem(STORE) ?? '{}');
    } catch {
      user = {};
    }
  });
  function rackSnapshot(): Snap {
    // just this rack (and, for the main rack or buses, their splitters' bands)
    const s = synth.fx.snapshot();
    const keep = new Set<number>([rack]);
    for (const r of s.chains[rack]) if (r.type === 13) for (let b = 0; b < 3; b++) keep.add(bandChain(r.inst, b));
    return { chains: s.chains.map((c, i) => (keep.has(i) ? c : [])), params: s.params };
  }
  function save(): void {
    const name = prompt('Name this rack preset');
    if (!name) return;
    user = { ...user, [name]: rackSnapshot() };
    try {
      localStorage.setItem(STORE, JSON.stringify(user));
    } catch {
      // storage full or blocked: the preset lasts for this session only
    }
  }
  /** Load a preset into the current rack, each module on a free instance with the saved settings. */
  function load(name: string): void {
    if (FACTORY[name]) return FACTORY[name](synth);
    const s = user[name];
    if (!s) return;
    const src = s.chains.findIndex((c, i) => i < 3 && c.length);
    synth.fx.set(rack, []);
    if (src < 0) return;
    const copy = (from: FxRef, to: FxRef) => {
      for (const p of moduleParams(from)) {
        const v = s.params[p.key];
        const id = PARAM_ID[refParam(to, p.local)];
        if (v !== undefined && id !== undefined) synth.bank.set(id, v);
      }
    };
    for (const r of s.chains[src]) {
      const n = synth.fx.add(rack, r.type);
      if (!n) continue;
      copy(r, n);
      if (r.type === SPLITTER) {
        for (let b = 0; b < 3; b++) {
          for (const br of s.chains[bandChain(r.inst, b)] ?? []) {
            const nb = synth.fx.add(bandChain(n.inst, b), br.type);
            if (nb) copy(br, nb);
          }
        }
      }
    }
  }
</script>

<div class="fx-page">
  <section class="panel racks" data-explain="fx" aria-label="FX racks">
    <div class="tabs" role="tablist" aria-label="Rack">
      {#each RACK_NAMES as name, i (name)}
        <button role="tab" aria-selected={rack === i} class:on={rack === i} onclick={() => (rack = i)}>{name}</button>
      {/each}
    </div>
    {#if rack !== MAIN}
      <div class="route"><Select param={rack === BUS1 ? 'rack.bus1_to' : 'rack.bus2_to'} wide /></div>
    {/if}
    <div class="presets">
      <select aria-label="Rack preset" value="" onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        (e.currentTarget as HTMLSelectElement).value = '';
        if (v) load(v);
      }}>
        <option value="">Rack preset…</option>
        <optgroup label="Factory">{#each Object.keys(FACTORY) as n (n)}<option value={n}>{n}</option>{/each}</optgroup>
        {#if Object.keys(user).length}<optgroup label="Yours">{#each Object.keys(user) as n (n)}<option value={n}>{n}</option>{/each}</optgroup>{/if}
      </select>
      <button onclick={save}>Save…</button>
    </div>
    {#key rack}
      <div class="list"><ModuleList chain={rack} {selected} onselect={(r) => (selected = r)} /></div>
    {/key}
    <p class="note">{rack === MAIN ? 'Main: the sources routed to the filters or to Main, plus any bus sent here.' : 'A bus hears each source’s send to it; its output goes to the Main rack or straight to the master.'}</p>
  </section>
  <div class="editor">
    {#if selected}
      {#key `${selected.type}.${selected.inst}`}<ModulePanel r={selected} />{/key}
    {:else}
      <section class="panel empty"><p>Add an effect to the rack, then click its name to edit it.</p></section>
    {/if}
  </div>
</div>

<style>
  .fx-page {
    display: grid;
    grid-template-columns: 300px 1fr;
    gap: 8px;
    height: 100%;
    min-height: 0;
  }
  .racks {
    display: grid;
    grid-template-rows: auto auto auto 1fr auto;
    gap: 6px;
    min-height: 0;
  }
  .tabs {
    display: flex;
    gap: 3px;
  }
  .tabs button {
    flex: 1;
    font: 600 10px var(--font-ui);
    letter-spacing: 0.06em;
    color: var(--text-dim);
    background: transparent;
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 3px 0;
    cursor: pointer;
  }
  .tabs button.on {
    color: var(--text);
    border-color: var(--fx);
  }
  .presets {
    display: flex;
    gap: 4px;
  }
  .presets select {
    flex: 1;
  }
  .presets select,
  .presets button {
    font: 11px var(--font-ui);
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 2px 6px;
  }
  .presets button {
    cursor: pointer;
  }
  .list {
    overflow-y: auto;
    min-height: 0;
  }
  .note {
    margin: 0;
    font-size: 10px;
    color: var(--text-faint);
  }
  .editor {
    min-height: 0;
  }
  .empty {
    height: 100%;
    display: grid;
    place-items: center;
    color: var(--text-dim);
    font-size: 12px;
  }
</style>
