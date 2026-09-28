<!-- One filter: type, response graph and the main controls. The MIX page has the rest. -->
<script lang="ts">
  import type { ParamKey } from '../../gen/params';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import FilterGraph from '../graphs/FilterGraph.svelte';
  import { getContext } from 'svelte';
  import { PARAMS, PARAM_ID } from '../../gen/params';
  import { toPlain } from '../../state/param-math';
  import { filterGroup, resLabel, varLabel } from '../../state/filter-types';
  import type { Synth } from '../../synth';

  let { n = 1 }: { n?: number } = $props();
  const synth = getContext<Synth>('synth');
  const k = (s: string) => `filter.${n}.${s}` as ParamKey;
  const color = 'var(--filter)';
  let kind = $state(0);
  $effect(() => {
    const info = PARAMS[PARAM_ID[k('type')]];
    kind = toPlain(info, synth.bank.get(info.id));
    return synth.bank.subscribe(info.id, (v) => (kind = toPlain(info, v)));
  });
</script>

<section class="panel filter" data-explain={`filter.${n}`} aria-label={`Filter ${n}`} style:--accent={color}>
  <header>
    <Toggle param={k('enable')} label={`FILTER ${n}`} {color} power />
    <Select param={k('type')} label="" wide group={filterGroup} />
  </header>
  <div class="graph"><FilterGraph {n} {color} /></div>
  <div class="row">
    <Knob param={k('cutoff')} size={28} {color} compact />
    <Knob param={k('res')} label={resLabel(kind) || undefined} size={28} {color} compact />
    <Knob param={k('drive')} size={28} {color} compact />
    <Knob param={k('var')} label={varLabel(kind) || 'Var'} size={28} {color} compact />
  </div>
</section>

<style>
  .filter {
    display: grid;
    grid-template-rows: auto 1fr auto;
    gap: 6px;
    min-height: 0;
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
  .row {
    display: flex;
    justify-content: space-between;
  }
</style>
