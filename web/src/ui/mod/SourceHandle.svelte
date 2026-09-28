<!-- A grip to drag a modulation source onto any knob. -->
<script lang="ts">
  import { getContext } from 'svelte';
  import { SOURCES } from '../../gen/protocol';
  import type { Synth } from '../../synth';
  import { sourceColor } from './sources';
  import { startModDrag } from './drag';

  let { source, label = '' }: { source: number; label?: string } = $props();
  const synth = getContext<Synth>('synth');
  const name = $derived(SOURCES[source] as string);
  const color = $derived(sourceColor(source));
</script>

<button
  class="handle"
  style:--c={color}
  aria-label={`Drag ${name} onto a control to modulate it`}
  title={`Drag ${name} onto a knob to modulate it`}
  data-source={name}
  onpointerdown={(e) => startModDrag(e, source, name, color, synth.matrix)}
>
  <svg width="10" height="12" viewBox="0 0 10 12" aria-hidden="true">
    {#each [0, 1, 2] as r (r)}{#each [0, 1] as c (c)}<circle cx={2.5 + c * 5} cy={2 + r * 4} r="1.3" />{/each}{/each}
  </svg>
  {#if label}<span>{label}</span>{/if}
</button>

<style>
  .handle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font: 600 10px var(--font-ui);
    letter-spacing: 0.04em;
    color: var(--c);
    background: color-mix(in srgb, var(--c) 12%, var(--panel-2));
    border: 1px solid color-mix(in srgb, var(--c) 55%, var(--line));
    border-radius: 10px;
    padding: 1px 7px 1px 5px;
    cursor: grab;
    touch-action: none;
    user-select: none;
    white-space: nowrap;
  }
  .handle:active {
    cursor: grabbing;
  }
  .handle:focus-visible {
    outline: 1px solid var(--c);
  }
  svg {
    fill: currentColor;
  }
</style>
