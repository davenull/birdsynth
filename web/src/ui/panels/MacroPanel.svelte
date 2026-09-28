<!-- Eight macro knobs: route one to many destinations and turn several things at once. -->
<script lang="ts">
  import type { ParamKey } from '../../gen/params';
  import { SOURCE, type SourceName } from '../../state/matrix';
  import Knob from '../primitives/Knob.svelte';
  import SourceHandle from '../mod/SourceHandle.svelte';

  const color = 'var(--macro)';
</script>

<section class="panel macros" data-explain="macro" aria-label="Macros">
  <h2>MACROS</h2>
  <div class="grid">
    {#each [1, 2, 3, 4, 5, 6, 7, 8] as i (i)}
      <div class="cell">
        <SourceHandle source={SOURCE[`Macro ${i}` as SourceName]} label={String(i)} />
        <Knob param={`macro.${i}.value` as ParamKey} label={`Macro ${i}`} size={24} {color} compact />
      </div>
    {/each}
  </div>
</section>

<style>
  .macros {
    display: grid;
    grid-template-rows: auto 1fr;
    gap: 4px;
  }
  h2 {
    margin: 0;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.1em;
    color: var(--text-dim);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    justify-items: center;
    align-content: center;
    row-gap: 2px;
  }
  .cell {
    display: grid;
    justify-items: center;
  }
</style>
