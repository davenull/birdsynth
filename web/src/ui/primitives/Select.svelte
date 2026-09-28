<!--
  A dropdown bound to an enum parameter. `max` hides options past that
  index; `group` sorts options into labelled sections.
-->
<script lang="ts">
  import { getContext } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { toNorm, toPlain } from '../../state/param-math';
  import type { ParamBank } from '../../state/bank';

  let {
    param,
    label,
    max,
    wide = false,
    group,
  }: { param: ParamKey; label?: string; max?: number; wide?: boolean; group?: (index: number) => string } = $props();

  const bank = getContext<ParamBank>('bank');
  const info = $derived(PARAMS[PARAM_ID[param]]);
  const options = $derived(info.curve.kind === 'enum' ? info.curve.options : []);
  /** Options by section, in order of first appearance. */
  const sections = $derived.by(() => {
    if (!group) return [];
    const out = new Map<string, number[]>();
    options.forEach((_, i) => {
      if (max !== undefined && i > max) return;
      const g = group(i);
      const list = out.get(g) ?? [];
      list.push(i);
      out.set(g, list);
    });
    return [...out];
  });
  let value = $state(0);
  $effect(() => {
    value = toPlain(info, bank.get(info.id));
    return bank.subscribe(info.id, (v) => (value = toPlain(info, v)));
  });
</script>

<label class="select" class:wide data-explain={param} title={info.explain}>
  <span class="label">{label ?? info.short}</span>
  <select aria-label={info.name} value={value} onchange={(e) => bank.set(info.id, toNorm(info, Number((e.currentTarget as HTMLSelectElement).value)))}>
    {#if group}
      {#each sections as [g, idx] (g)}
        <optgroup label={g}>{#each idx as i (i)}<option value={i}>{options[i]}</option>{/each}</optgroup>
      {/each}
    {:else}
      {#each options as name, i (i)}
        {#if max === undefined || i <= max}
          <option value={i}>{name}</option>
        {/if}
      {/each}
    {/if}
  </select>
</label>

<style>
  .select {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .label {
    font-size: 10px;
    color: var(--text-dim);
  }
  select {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 2px 4px;
    max-width: 118px;
  }
  .wide select {
    max-width: none;
  }
  select:focus-visible {
    outline: 1px solid var(--accent);
  }
</style>
