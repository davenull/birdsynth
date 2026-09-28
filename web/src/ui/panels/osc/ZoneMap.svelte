<!--
  A multisample's zones over the keyboard (across) and velocity (up), with
  the notes playing now lit.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { Synth } from '../../../synth';
  import type { ZoneInfo } from '../../../state/multis';
  import { onFrame } from '../../frame';

  let { osc, color = 'var(--osc-a)' }: { osc: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  let zones = $state<ZoneInfo[]>([]);
  let name = $state('');
  let notes = $state<number[]>([]);

  onMount(() => {
    const read = () => {
      const m = synth.multis.osc[osc];
      zones = m?.zones ?? [];
      name = m?.name ?? '';
    };
    read();
    const off = synth.multis.subscribe((o) => o === osc && read());
    const offFrame = onFrame(() => {
      const n = synth.telemetry().notes;
      if (n.length !== notes.length || n.some((v, i) => v !== notes[i])) notes = n;
    });
    return () => {
      off();
      offFrame();
    };
  });

  const lo = $derived(zones.length ? Math.min(...zones.map((z) => z.lokey)) : 0);
  const hi = $derived(zones.length ? Math.max(...zones.map((z) => z.hikey)) : 127);
  const x = (k: number) => ((k - lo) / Math.max(1, hi - lo + 1)) * 100;
</script>

<div class="map" style:--c={color}>
  {#if !zones.length}
    <div class="empty">Choose a factory instrument or load an SFZ file</div>
  {:else}
    {#each zones as z, i (i)}
      {@const on = notes.some((n) => n >= z.lokey && n <= z.hikey)}
      <div
        class="zone"
        class:on
        title={`${z.sample}: keys ${z.lokey}–${z.hikey}, velocity ${z.lovel}–${z.hivel}, root ${z.root.toFixed(1)}`}
        style:left={`${x(z.lokey)}%`}
        style:width={`${x(z.hikey + 1) - x(z.lokey)}%`}
        style:bottom={`${(z.lovel / 128) * 100}%`}
        style:height={`${((z.hivel - z.lovel + 1) / 128) * 100}%`}
      ></div>
    {/each}
    <div class="name">{name} · {zones.length} zones</div>
  {/if}
</div>

<style>
  .map {
    position: relative;
    height: 100%;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    overflow: hidden;
  }
  .zone {
    position: absolute;
    box-sizing: border-box;
    border: 1px solid color-mix(in srgb, var(--c) 70%, transparent);
    background: color-mix(in srgb, var(--c) 18%, transparent);
  }
  .zone.on {
    background: color-mix(in srgb, var(--c) 60%, transparent);
  }
  .empty,
  .name {
    position: absolute;
    font-size: 10px;
    pointer-events: none;
  }
  .empty {
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-faint);
    text-align: center;
  }
  .name {
    left: 5px;
    top: 3px;
    color: var(--text-dim);
  }
</style>
