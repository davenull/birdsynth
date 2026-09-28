<!-- One FX chain as a list: power, name (click to edit), move, remove; splitters show their band chains. -->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID } from '../../gen/params';
  import { FX_TYPES, SPLITTER, bandChain, refName, refParam, type FxRef } from '../../state/fx';
  import { toPlain } from '../../state/param-math';
  import type { Synth } from '../../synth';
  import Toggle from '../primitives/Toggle.svelte';
  import Self from './ModuleList.svelte';

  let { chain, selected, onselect, depth = 0 }: { chain: number; selected: FxRef | null; onselect: (r: FxRef) => void; depth?: number } = $props();
  const synth = getContext<Synth>('synth');
  let items = $state<FxRef[]>([]);
  let modes = $state<Record<number, number>>({});
  onMount(() => {
    const read = () => {
      items = synth.fx.chains[chain].map((r) => ({ ...r }));
      const m: Record<number, number> = {};
      for (const r of items) {
        if (r.type === SPLITTER) {
          const info = PARAMS[PARAM_ID[refParam(r, 'mode')]];
          m[r.inst] = toPlain(info, synth.bank.get(info.id));
        }
      }
      modes = m;
    };
    read();
    const offs = [synth.fx.subscribe(read), ...[0, 1, 2, 3].map((i) => synth.bank.subscribe(PARAM_ID[refParam({ type: SPLITTER, inst: i }, 'mode')], read))];
    return () => offs.forEach((f) => f());
  });
  const addable = $derived(FX_TYPES.map((t, i) => ({ ...t, i })).filter((t) => !(depth > 0 && t.i === SPLITTER)));
  const same = (a: FxRef | null, b: FxRef) => !!a && a.type === b.type && a.inst === b.inst;
  function add(type: number): void {
    const r = synth.fx.add(chain, type);
    if (r) onselect(r);
  }
</script>

<ol class="list" class:nested={depth > 0}>
  {#each items as r, i (`${r.type}.${r.inst}`)}
    <li class:selected={same(selected, r)}>
      <div class="row">
        <Toggle param={refParam(r, 'enable')} label="" power />
        <button class="name" onclick={() => onselect(r)} aria-label={`Edit ${refName(r)}`}>{refName(r)}</button>
        <button class="icon" aria-label={`Move ${refName(r)} up`} disabled={i === 0} onclick={() => synth.fx.move(chain, i, i - 1)}>↑</button>
        <button class="icon" aria-label={`Move ${refName(r)} down`} disabled={i === items.length - 1} onclick={() => synth.fx.move(chain, i, i + 1)}>↓</button>
        <button class="icon" aria-label={`Remove ${refName(r)}`} onclick={() => synth.fx.remove(chain, i)}>✕</button>
      </div>
      {#if r.type === SPLITTER}
        {@const bands = modes[r.inst] === 1 ? 3 : 2}
        {@const names = modes[r.inst] === 2 ? ['MID', 'SIDE'] : bands === 3 ? ['LOW', 'MID', 'HIGH'] : ['LOW', 'HIGH']}
        <div class="bands">
          {#each names as band, b (band)}
            <div class="band"><span>{band}</span><Self chain={bandChain(r.inst, b)} {selected} {onselect} depth={depth + 1} /></div>
          {/each}
        </div>
      {/if}
    </li>
  {/each}
  <li class="add">
    <select aria-label={depth ? 'Add an effect to this band' : 'Add an effect'} value="" onchange={(e) => {
      const v = (e.currentTarget as HTMLSelectElement).value;
      (e.currentTarget as HTMLSelectElement).value = '';
      if (v !== '') add(Number(v));
    }}>
      <option value="">+ Add effect…</option>
      {#each addable as t (t.key)}<option value={t.i} disabled={synth.fx.freeInstance(t.i) < 0}>{t.name}</option>{/each}
    </select>
  </li>
</ol>

<style>
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 3px;
  }
  .nested {
    padding-left: 8px;
    border-left: 1px solid var(--line);
  }
  li.selected > .row {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, var(--panel-2));
  }
  .row {
    display: grid;
    grid-template-columns: auto 1fr auto auto auto;
    align-items: center;
    gap: 4px;
    padding: 2px 4px;
    border: 1px solid var(--line);
    border-radius: 4px;
    background: var(--panel-2);
  }
  .name {
    text-align: left;
    font: 600 11px var(--font-ui);
    color: var(--text);
    background: none;
    border: 0;
    cursor: pointer;
    padding: 2px;
  }
  .icon {
    font: 11px var(--font-ui);
    color: var(--text-dim);
    background: none;
    border: 1px solid transparent;
    border-radius: 3px;
    padding: 0 4px;
    cursor: pointer;
  }
  .icon:hover:not(:disabled) {
    border-color: var(--line);
    color: var(--text);
  }
  .icon:disabled {
    opacity: 0.3;
  }
  .bands {
    display: grid;
    gap: 3px;
    margin: 3px 0 3px 6px;
  }
  .band > span {
    font: 600 9px var(--font-ui);
    letter-spacing: 0.08em;
    color: var(--text-dim);
  }
  .add select {
    width: 100%;
    font: 11px var(--font-ui);
    color: var(--text-dim);
    background: transparent;
    border: 1px dashed var(--line);
    border-radius: 4px;
    padding: 3px;
  }
</style>
