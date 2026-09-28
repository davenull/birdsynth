<!-- The four envelopes, one at a time. Env 1 always shapes the amp. -->
<script lang="ts">
  import type { ParamKey } from '../../gen/params';
  import Knob from '../primitives/Knob.svelte';
  import EnvGraph from '../graphs/EnvGraph.svelte';

  let n = $state(1);
  const color = 'var(--env)';
  const k = (s: string) => `env.${n}.${s}` as ParamKey;
</script>

<section class="panel env" data-explain="env" aria-label="Envelopes">
  <div class="tabs" role="tablist" aria-label="Envelope">
    {#each [1, 2, 3, 4] as i (i)}
      <button role="tab" aria-selected={n === i} class:on={n === i} onclick={() => (n = i)}>ENV {i}{i === 1 ? ' · AMP' : ''}</button>
    {/each}
  </div>
  {#key n}
    <div class="body">
      <div class="graph"><EnvGraph {n} {color} /></div>
      <div class="knobs">
        <Knob param={k('attack')} size={30} {color} />
        <Knob param={k('hold')} size={30} {color} />
        <Knob param={k('decay')} size={30} {color} />
        <Knob param={k('sustain')} size={30} {color} />
        <Knob param={k('release')} size={30} {color} />
      </div>
    </div>
  {/key}
</section>

<style>
  .env {
    display: grid;
    grid-template-rows: auto 1fr;
    gap: 6px;
  }
  .tabs {
    display: flex;
    gap: 4px;
  }
  .tabs button {
    font: 600 10px var(--font-ui);
    letter-spacing: 0.05em;
    color: var(--text-dim);
    background: transparent;
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 2px 8px;
    cursor: pointer;
  }
  .tabs button.on {
    color: var(--text);
    border-color: var(--env);
  }
  .body {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 8px;
    min-height: 0;
  }
  .graph {
    min-height: 0;
  }
  .knobs {
    display: flex;
    align-items: center;
  }
</style>
