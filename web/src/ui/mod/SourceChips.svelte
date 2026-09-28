<!-- Handles for the note and controller sources, which have no panel of their own. -->
<script lang="ts">
  import { SOURCE, type SourceName } from '../../state/matrix';
  import SourceHandle from './SourceHandle.svelte';

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
</script>

<div class="chips" data-explain="sources" aria-label="More modulation sources">
  {#each CHIPS as [name, label] (name)}<SourceHandle source={SOURCE[name]} {label} />{/each}
</div>

<style>
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
  }
</style>
