<!-- The four envelopes, one at a time. Env 1 always shapes the amp; any of them can be dragged onto a knob. -->
<script lang="ts">
  import type { ParamKey } from '../../gen/params';
  import { SOURCE, type SourceName } from '../../state/matrix';
  import { strip } from '../strip.svelte';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import EnvGraph from '../graphs/EnvGraph.svelte';
  import SourceHandle from '../mod/SourceHandle.svelte';

  const n = $derived(strip.env);
  const color = 'var(--env)';
  const k = (s: string) => `env.${n}.${s}` as ParamKey;
</script>

<section class="panel env" data-explain="env" aria-label="Envelopes">
  <div class="top">
    <div class="tabs" role="tablist" aria-label="Envelope">
      {#each [1, 2, 3, 4] as i (i)}
        <button role="tab" aria-selected={n === i} class:on={n === i} onclick={() => (strip.env = i)}>ENV {i}{i === 1 ? ' · AMP' : ''}</button>
      {/each}
    </div>
    <SourceHandle source={SOURCE[`Env ${n}` as SourceName]} label={`ENV ${n}`} />
  </div>
  {#key n}
    <div class="body">
      <div class="graph"><EnvGraph {n} {color} /></div>
      <div class="knobs">
        <Knob param={k('attack')} size={24} {color} compact />
        <Knob param={k('hold')} size={24} {color} compact />
        <Knob param={k('decay')} size={24} {color} compact />
        <Knob param={k('sustain')} size={24} {color} compact />
        <Knob param={k('release')} size={24} {color} compact />
        <Knob param={k('attack_curve')} label="A Crv" size={20} {color} compact />
        <Knob param={k('decay_curve')} label="D Crv" size={20} {color} compact />
        <Knob param={k('release_curve')} label="R Crv" size={20} {color} compact />
      </div>
    </div>
    <div class="opts">
      <Toggle param={k('bpm')} {color} />
      <Toggle param={k('legato_invert')} {color} />
      <Select param={k('retrig')} />
    </div>
  {/key}
</section>

<style>
  .env {
    display: grid;
    grid-template-rows: auto 1fr auto;
    gap: 5px;
    min-height: 0;
  }
  .top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .tabs {
    display: flex;
    gap: 3px;
  }
  .tabs button {
    font: 600 9.5px var(--font-ui);
    letter-spacing: 0.04em;
    color: var(--text-dim);
    background: transparent;
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 2px 6px;
    cursor: pointer;
  }
  .tabs button.on {
    color: var(--text);
    border-color: var(--env);
  }
  .body {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 6px;
    min-height: 0;
  }
  .graph {
    min-height: 0;
  }
  .knobs {
    display: grid;
    grid-template-columns: repeat(4, auto);
    align-content: center;
  }
  .opts {
    display: flex;
    align-items: end;
    gap: 6px;
  }
  .opts :global(.select) {
    grid-auto-flow: column;
    align-items: center;
    gap: 4px;
  }
</style>
