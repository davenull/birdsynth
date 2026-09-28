<!-- A button bound to a bool parameter. -->
<script lang="ts">
  import { getContext } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import type { ParamBank } from '../../state/bank';

  let { param, label, color = 'var(--accent)', power = false }: { param: ParamKey; label?: string; color?: string; power?: boolean } = $props();

  const bank = getContext<ParamBank>('bank');
  const info = $derived(PARAMS[PARAM_ID[param]]);
  let on = $state(false);
  $effect(() => {
    on = bank.get(info.id) >= 0.5;
    return bank.subscribe(info.id, (v) => (on = v >= 0.5));
  });
</script>

<button
  class="toggle"
  class:on
  class:power
  style:--on-color={color}
  aria-pressed={on}
  aria-label={info.name}
  title={info.explain}
  data-explain={param}
  onclick={() => bank.set(info.id, on ? 0 : 1)}
>
  {#if power}<span class="led" aria-hidden="true"></span>{/if}{label ?? info.short}
</button>

<style>
  .toggle {
    font: 600 10px var(--font-ui);
    letter-spacing: 0.04em;
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 3px 7px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    white-space: nowrap;
  }
  .toggle.on {
    color: var(--text);
    border-color: var(--on-color);
    background: color-mix(in srgb, var(--on-color) 18%, var(--panel-2));
  }
  .led {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--knob-track);
  }
  .on .led {
    background: var(--on-color);
    box-shadow: 0 0 6px var(--on-color);
  }
  .toggle:focus-visible {
    outline: 1px solid var(--on-color);
  }
</style>
