<!--
  A dropdown bound to an enum parameter. `max` hides options past that
  index; `group` sorts options into labelled sections; `join` shows several
  options as one (picking it keeps a value already among them, so the real
  choice is kept for when they're shown apart again).
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
    join = null,
  }: { param: ParamKey; label?: string; max?: number; wide?: boolean; group?: (index: number) => string; join?: { of: readonly number[]; label: string } | null } = $props();

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
  const joined = (i: number) => !!join && join.of.includes(i);
  /** A joined option stands for its first member. */
  const shown = $derived(joined(value) ? join!.of[0] : value);
  const pick = (v: number) => {
    if (joined(v) && joined(value)) return; // already one of them: keep which
    bank.set(info.id, toNorm(info, v));
  };
</script>

<label class="select" class:wide data-explain={param} title={info.explain}>
  <span class="label">{label ?? info.short}</span>
  <select aria-label={info.name} value={shown} onchange={(e) => pick(Number((e.currentTarget as HTMLSelectElement).value))}>
    {#if group}
      {#each sections as [g, idx] (g)}
        <optgroup label={g}>{#each idx as i (i)}{#if !(joined(i) && i !== join!.of[0])}<option value={i}>{joined(i) ? join!.label : options[i]}</option>{/if}{/each}</optgroup>
      {/each}
    {:else}
      {#each options as name, i (i)}
        {#if (max === undefined || i <= max) && !(joined(i) && i !== join!.of[0])}
          <option value={i}>{joined(i) ? join!.label : name}</option>
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
