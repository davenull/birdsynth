<!-- MIX page: the routing diagram and a channel strip per source and filter. -->
<script lang="ts">
  import type { ParamKey } from '../../gen/params';
  import type { Synth } from '../../synth';
  import { getContext } from 'svelte';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import Scope from '../primitives/Scope.svelte';
  import RoutingDiagram from '../graphs/RoutingDiagram.svelte';
  import { filterGroup } from '../../state/filter-types';

  const synth = getContext<Synth>('synth');
  const SOURCES = [
    { key: 'osc.a', name: 'OSC A', color: 'var(--osc-a)' },
    { key: 'osc.b', name: 'OSC B', color: 'var(--osc-b)' },
    { key: 'osc.c', name: 'OSC C', color: 'var(--osc-c)' },
    { key: 'sub', name: 'SUB', color: 'var(--sub)' },
    { key: 'noise', name: 'NOISE', color: 'var(--noise)' },
  ];
  const p = (a: string, b: string) => `${a}.${b}` as ParamKey;
</script>

<div class="mix-page">
  <section class="panel flow" data-explain="mix" aria-label="Signal routing"><RoutingDiagram /></section>
  <div class="strips">
    {#each SOURCES as s (s.key)}
      <section class="panel strip" data-explain={`mix.${s.key}`} aria-label={`${s.name} mix`} style:--accent={s.color}>
        <Toggle param={p(s.key, 'enable')} label={s.name} color={s.color} power />
        <div class="row">
          <Knob param={p(s.key, 'level')} size={24} color={s.color} compact />
          <Knob param={p(s.key, 'pan')} size={24} color={s.color} compact />
        </div>
        <Select param={p(s.key, 'route')} label="Route" wide />
        <div class="row">
          <Knob param={p(s.key, 'balance')} size={22} color={s.color} compact />
          <Knob param={p(s.key, 'send1')} size={22} color={s.color} compact />
          <Knob param={p(s.key, 'send2')} size={22} color={s.color} compact />
        </div>
      </section>
    {/each}
    {#each [1, 2] as n (n)}
      <section class="panel strip" data-explain={`filter.${n}`} aria-label={`Filter ${n} mix`} style:--accent="var(--filter)">
        <Toggle param={p(`filter.${n}`, 'enable')} label={`FILTER ${n}`} color="var(--filter)" power />
        <div class="row">
          <Knob param={p(`filter.${n}`, 'level')} size={24} color="var(--filter)" compact />
          <Knob param={p(`filter.${n}`, 'pan')} size={24} color="var(--filter)" compact />
          <Knob param={p(`filter.${n}`, 'mix')} size={24} color="var(--filter)" compact />
        </div>
        <Select param={p(`filter.${n}`, 'type')} label="Type" wide group={filterGroup} />
        <div class="row">
          <Knob param={p(`filter.${n}`, 'keytrack')} size={22} color="var(--filter)" compact />
          <Knob param={p(`filter.${n}`, 'stereo')} size={22} color="var(--filter)" compact />
          <Knob param={p(`filter.${n}`, 'var')} size={22} color="var(--filter)" compact />
        </div>
      </section>
    {/each}
    <section class="panel strip out" data-explain="mix.filter_routing" aria-label="Filter routing">
      <Select param="mix.filter_routing" wide />
      <Scope {synth} tap="focus.out" label="Newest voice" color="var(--osc-a)" />
    </section>
  </div>
</div>

<style>
  .mix-page {
    display: grid;
    grid-template-rows: auto 1fr;
    gap: 8px;
    height: 100%;
    min-height: 0;
  }
  .flow {
    padding: 6px 10px;
  }
  .strips {
    display: grid;
    grid-template-columns: repeat(7, 1fr) 1.3fr;
    gap: 8px;
    min-height: 0;
  }
  .strip {
    display: grid;
    gap: 5px;
    align-content: start;
  }
  .row {
    display: flex;
    justify-content: space-around;
  }
  .out {
    grid-template-rows: auto 1fr;
    align-content: stretch;
  }
</style>
