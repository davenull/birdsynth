<!--
  Handles for the note and controller sources, which have no panel of their
  own; and, on pages without the modulation strip, compact ones for the
  envelopes, LFOs and macros too.
-->
<script lang="ts">
  import { SOURCE, type SourceName } from '../../state/matrix';
  import SourceHandle from './SourceHandle.svelte';
  import { sourceColor } from './sources';

  let { panels = false }: { panels?: boolean } = $props();

  const CHIPS: [SourceName, string][] = [
    ['Velocity', 'VEL'],
    ['Note', 'NOTE'],
    ['Release Vel', 'REL VEL'],
    ['Mod Wheel', 'WHEEL'],
    ['Pitch Bend', 'BEND'],
    ['Aftertouch', 'AT'],
    ['Poly AT', 'POLY AT'],
    ['Rand 1', 'RAND 1'],
    ['Rand 2', 'RAND 2'],
  ];
  // the rest (voice index, active voices, fixed, MPE...) are in the MATRIX page's source lists
  const range = (n: number) => Array.from({ length: n }, (_, i) => i + 1);
  const GROUPS: { name: string; family: string; n: number }[] = [
    { name: 'ENV', family: 'Env', n: 4 },
    { name: 'LFO', family: 'LFO', n: 10 },
    { name: 'MACRO', family: 'Macro', n: 8 },
  ];
</script>

<div class="chips" data-explain="sources" aria-label="More modulation sources">
  {#each CHIPS as [name, label] (name)}<SourceHandle source={SOURCE[name]} {label} />{/each}
  {#if panels}
    {#each GROUPS as g (g.name)}
      <span class="group" style:--c={sourceColor(SOURCE[`${g.family} 1` as SourceName])}>
        <span class="gname">{g.name}</span>
        {#each range(g.n) as i (i)}<SourceHandle source={SOURCE[`${g.family} ${i}` as SourceName]} label={String(i)} bare />{/each}
      </span>
    {/each}
  {/if}
</div>

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
  }
  .group {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    margin-left: 6px;
  }
  .gname {
    font: 600 9.5px var(--font-ui);
    letter-spacing: 0.06em;
    color: var(--c);
    margin-right: 2px;
  }
</style>
