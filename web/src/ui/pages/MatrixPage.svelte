<!--
  MATRIX page: every routing as a row. Source, curve, amount, destination,
  unipolar/bipolar, aux source and output, with the destination's live
  value. Rows can be bypassed, reordered and removed.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { FLAG, PARAMS, type ParamInfo } from '../../gen/params';
  import { SOURCES, TEL, TEL_COUNT } from '../../gen/protocol';
  import { format } from '../../state/param-math';
  import { SOURCE, type ModSlot, type SourceName } from '../../state/matrix';
  import type { Synth } from '../../synth';
  import { SOURCE_GROUPS, sourceColor } from '../mod/sources';
  import { onFrame } from '../frame';

  const synth = getContext<Synth>('synth');
  const m = synth.matrix;

  /** Modulatable parameters, grouped by section ("Osc A", "Filter 1", ...). */
  const DEST_GROUPS = (() => {
    const out = new Map<string, ParamInfo[]>();
    for (const p of PARAMS) {
      if (!(p.flags & FLAG.mod)) continue;
      const section = p.name.endsWith(p.short) ? p.name.slice(0, p.name.length - p.short.length).trim() : p.group;
      const list = out.get(section) ?? [];
      list.push(p);
      out.set(section, list);
    }
    return [...out];
  })();

  let rows = $state<{ i: number; s: ModSlot }[]>([]);
  const read = () => (rows = m.slots.flatMap((s, i) => (s ? [{ i, s: { ...s } }] : [])));
  onMount(() => {
    read();
    return m.subscribe(read);
  });

  // live values of each row's destination (the newest voice's)
  let live = $state<Record<number, number>>({});
  onMount(() =>
    onFrame(() => {
      const t = synth.host?.tel;
      if (!t) return;
      const out: Record<number, number> = {};
      for (let d = 0; d < TEL_COUNT.modDest; d++) {
        const id = t[TEL.modDest + d];
        if (id >= 0) out[id] = t[TEL.modValue + d];
      }
      live = out;
    }),
  );

  let newSource = $state(SOURCE['LFO 1']);
  let newDest = $state(DEST_GROUPS[0][1][0].id);
  const pct = (v: number) => `${v >= 0 ? '+' : ''}${Math.round(v * 100)}%`;
  const num = (e: Event) => Number((e.currentTarget as HTMLInputElement | HTMLSelectElement).value);
</script>

<section class="panel matrix" data-explain="matrix" aria-label="Modulation matrix">
  <header>
    <h2>MODULATION MATRIX</h2>
    <span class="count">{rows.length} of {m.slots.length} slots</span>
    <div class="add">
      <select aria-label="New routing source" value={newSource} onchange={(e) => (newSource = num(e))}>
        {#each SOURCE_GROUPS as g (g.name)}
          <optgroup label={g.name}>{#each g.sources as n (n)}<option value={SOURCE[n as SourceName]}>{n}</option>{/each}</optgroup>
        {/each}
      </select>
      <span>→</span>
      <select aria-label="New routing destination" value={newDest} onchange={(e) => (newDest = num(e))}>
        {#each DEST_GROUPS as [g, ps] (g)}
          <optgroup label={g}>{#each ps as p (p.id)}<option value={p.id}>{p.name}</option>{/each}</optgroup>
        {/each}
      </select>
      <button onclick={() => m.add(newSource, newDest)}>Add</button>
    </div>
  </header>
  <div class="table" role="table" aria-label="Routings">
    <div class="tr head" role="row">
      <span role="columnheader">#</span><span role="columnheader">Source</span><span role="columnheader">Curve</span><span role="columnheader">Amount</span>
      <span role="columnheader">Destination</span><span role="columnheader">Type</span><span role="columnheader">Aux</span><span role="columnheader">Output</span>
      <span role="columnheader">Value</span><span role="columnheader"></span>
    </div>
    {#if !rows.length}
      <p class="empty">No routings yet. Drag an ENV, LFO or macro handle onto a knob, or add one above.</p>
    {/if}
    {#each rows as { i, s } (i)}
      {@const n = i + 1}
      {@const v = live[s.dest]}
      <div class="tr" role="row" class:bypass={s.bypass} style:--c={sourceColor(s.source)} aria-label={`Slot ${n}: ${SOURCES[s.source]} to ${PARAMS[s.dest].name}`}>
        <span class="idx">{n}</span>
        <select aria-label={`Slot ${n} source`} value={s.source} onchange={(e) => m.update(i, { source: num(e) })}>
          {#each SOURCE_GROUPS as g (g.name)}
            <optgroup label={g.name}>{#each g.sources as sn (sn)}<option value={SOURCE[sn as SourceName]}>{sn}</option>{/each}</optgroup>
          {/each}
        </select>
        <label class="slider"><input type="range" min="-100" max="100" aria-label={`Slot ${n} curve`} value={Math.round(s.curve * 100)} oninput={(e) => m.update(i, { curve: Math.fround(num(e) / 100) })} ondblclick={() => m.update(i, { curve: 0 })} /><span>{Math.round(s.curve * 100)}</span></label>
        <label class="slider amount"><input type="range" min="-100" max="100" aria-label={`Slot ${n} amount`} value={Math.round(s.amount * 100)} oninput={(e) => m.update(i, { amount: Math.fround(num(e) / 100) })} /><span>{pct(s.amount)}</span></label>
        <select aria-label={`Slot ${n} destination`} value={s.dest} onchange={(e) => m.update(i, { dest: num(e) })}>
          {#each DEST_GROUPS as [g, ps] (g)}
            <optgroup label={g}>{#each ps as p (p.id)}<option value={p.id}>{p.name}</option>{/each}</optgroup>
          {/each}
        </select>
        <button class="type" aria-label={`Slot ${n} ${s.bipolar ? 'bipolar' : 'unipolar'}`} onclick={() => m.update(i, { bipolar: !s.bipolar })}>{s.bipolar ? '± Bi' : '+ Uni'}</button>
        <select aria-label={`Slot ${n} aux source`} value={s.aux} onchange={(e) => m.update(i, { aux: num(e) })}>
          <option value={0}>None</option>
          {#each SOURCE_GROUPS as g (g.name)}
            <optgroup label={g.name}>{#each g.sources as sn (sn)}<option value={SOURCE[sn as SourceName]}>{sn}</option>{/each}</optgroup>
          {/each}
        </select>
        <label class="slider"><input type="range" min="0" max="100" aria-label={`Slot ${n} output`} value={Math.round(s.output * 100)} oninput={(e) => m.update(i, { output: Math.fround(num(e) / 100) })} /><span>{Math.round(s.output * 100)}%</span></label>
        <span class="value" title={v === undefined ? '' : format(PARAMS[s.dest], v)}>
          <span class="bar" style:width={`${Math.round((v ?? PARAMS[s.dest].def) * 100)}%`}></span>
        </span>
        <span class="acts">
          <button aria-label={`Slot ${n} bypass`} aria-pressed={s.bypass} title="Bypass" onclick={() => m.update(i, { bypass: !s.bypass })}>⏻</button>
          <button aria-label={`Move slot ${n} up`} title="Move up" disabled={i === 0} onclick={() => m.move(i, i - 1)}>↑</button>
          <button aria-label={`Move slot ${n} down`} title="Move down" disabled={i === m.slots.length - 1} onclick={() => m.move(i, i + 1)}>↓</button>
          <button aria-label={`Remove slot ${n}`} title="Remove" onclick={() => m.remove(i)}>✕</button>
        </span>
      </div>
    {/each}
  </div>
</section>

<style>
  .matrix {
    display: grid;
    grid-template-rows: auto 1fr;
    gap: 8px;
    height: 100%;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  h2 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.1em;
  }
  .count {
    font: 10.5px var(--font-num);
    color: var(--text-dim);
  }
  .add {
    margin-left: auto;
    display: flex;
    gap: 6px;
    align-items: center;
  }
  select,
  button {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 2px 4px;
    min-width: 0;
  }
  button {
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .table {
    overflow-y: auto;
    min-height: 0;
    display: grid;
    align-content: start;
    gap: 3px;
  }
  .tr {
    display: grid;
    grid-template-columns: 26px 130px 110px 150px 1fr 60px 120px 110px 90px 104px;
    align-items: center;
    gap: 8px;
    padding: 3px 6px;
    border-left: 3px solid var(--c, transparent);
    border-radius: 4px;
    background: var(--panel-2);
  }
  .tr.head {
    background: none;
    border-color: transparent;
    font-size: 9.5px;
    letter-spacing: 0.08em;
    color: var(--text-dim);
    text-transform: uppercase;
    position: sticky;
    top: 0;
  }
  .tr.bypass {
    opacity: 0.45;
  }
  .idx {
    font: 10px var(--font-num);
    color: var(--text-dim);
  }
  .slider {
    display: grid;
    grid-template-columns: 1fr 38px;
    align-items: center;
    gap: 4px;
    font: 10px var(--font-num);
    color: var(--text-dim);
  }
  .slider input {
    width: 100%;
    accent-color: var(--c);
  }
  .type {
    font-size: 10px;
  }
  .value {
    height: 8px;
    border-radius: 4px;
    background: var(--glass);
    overflow: hidden;
  }
  .bar {
    display: block;
    height: 100%;
    background: var(--c);
  }
  .acts {
    display: flex;
    gap: 3px;
  }
  .acts button {
    padding: 1px 6px;
  }
  .empty {
    margin: 12px 4px;
    font-size: 12px;
    color: var(--text-dim);
  }
</style>
