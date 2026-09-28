<!-- Filter 1: type, response graph and controls. -->
<script lang="ts">
  import type { ParamKey } from '../../gen/params';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import FilterGraph from '../graphs/FilterGraph.svelte';

  let { n = 1 }: { n?: number } = $props();
  const k = (s: string) => `filter.${n}.${s}` as ParamKey;
  const color = 'var(--filter)';
</script>

<section class="panel filter" data-explain={`filter.${n}`} aria-label={`Filter ${n}`} style:--accent={color}>
  <header>
    <Toggle param={k('enable')} label={`FILTER ${n}`} {color} power />
    <Select param={k('type')} label="" wide />
  </header>
  <div class="graph"><FilterGraph {n} {color} /></div>
  <div class="grid">
    <Knob param={k('cutoff')} size={34} {color} />
    <Knob param={k('res')} size={34} {color} />
    <Knob param={k('drive')} size={34} {color} />
    <Knob param={k('mix')} size={30} {color} />
    <Knob param={k('keytrack')} size={30} {color} />
  </div>
</section>

<style>
  .filter {
    display: grid;
    grid-template-rows: auto 150px auto;
    gap: 8px;
    align-content: start;
  }
  header {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: end;
    gap: 6px;
  }
  .graph {
    min-height: 0;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    justify-items: center;
    row-gap: 4px;
  }
</style>
