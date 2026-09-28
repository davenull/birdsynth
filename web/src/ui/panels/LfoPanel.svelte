<!-- Ten LFOs, one at a time: shape editor, timing and trigger controls, and handles to drag them onto knobs. -->
<script lang="ts">
  import { getContext } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { SOURCE, type SourceName } from '../../state/matrix';
  import { SHAPE_PRESETS } from '../../state/lfo';
  import { toPlain } from '../../state/param-math';
  import type { Synth } from '../../synth';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import LfoEditor from '../graphs/LfoEditor.svelte';
  import SourceHandle from '../mod/SourceHandle.svelte';

  const synth = getContext<Synth>('synth');
  let n = $state(1);
  const k = (s: string) => `lfo.${n}.${s}` as ParamKey;
  const color = 'var(--lfo)';

  let bpm = $state(false);
  let type = $state(0);
  $effect(() => {
    const b = PARAMS[PARAM_ID[k('bpm')]];
    const t = PARAMS[PARAM_ID[k('type')]];
    bpm = synth.bank.get(b.id) >= 0.5;
    type = toPlain(t, synth.bank.get(t.id));
    const offB = synth.bank.subscribe(b.id, (v) => (bpm = v >= 0.5));
    const offT = synth.bank.subscribe(t.id, (v) => (type = toPlain(t, v)));
    return () => (offB(), offT());
  });

  function preset(name: string): void {
    const make = SHAPE_PRESETS[name];
    if (make) synth.lfo.set(n - 1, 'curve', make());
  }
</script>

<section class="panel lfo" data-explain="lfo" aria-label="LFOs">
  <div class="top">
    <div class="tabs" role="tablist" aria-label="LFO">
      {#each [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] as i (i)}
        <button role="tab" aria-selected={n === i} aria-label={`LFO ${i}`} class:on={n === i} onclick={() => (n = i)}>{i}</button>
      {/each}
    </div>
    <div class="handles">
      <SourceHandle source={SOURCE[`LFO ${n}` as SourceName]} label={`LFO ${n}`} />
      {#if type === 1 || type === 2 || type === 3}<SourceHandle source={SOURCE[`LFO ${n} Y` as SourceName]} label="Y" />{/if}
    </div>
  </div>
  {#key n}
    <div class="body">
      <div class="editor">
        <LfoEditor lfo={n - 1} width={172} height={112} />
        {#if type === 0}
          <select class="preset" aria-label={`LFO ${n} shape preset`} onchange={(e) => {
            preset((e.currentTarget as HTMLSelectElement).value);
            (e.currentTarget as HTMLSelectElement).value = '';
          }}>
            <option value="">Shape…</option>
            {#each Object.keys(SHAPE_PRESETS) as name (name)}<option value={name}>{name}</option>{/each}
          </select>
        {/if}
      </div>
      <div class="controls">
        <div class="row selects">
          <Select param={k('type')} label="" />
          <Select param={k('mode')} label="" />
          <Select param={k('direction')} label="" />
        </div>
        <div class="row">
          {#if bpm}
            <div class="sync">
              <Select param={k('sync_rate')} />
              <Select param={k('sync_mod')} />
            </div>
          {:else}
            <Knob param={k('rate')} size={24} {color} compact />
          {/if}
          <Knob param={k('rise')} size={24} {color} compact />
          <Knob param={k('delay')} size={24} {color} compact />
          <Knob param={k('smooth')} size={24} {color} compact />
          <Knob param={k('phase')} size={24} {color} compact />
        </div>
        <div class="row toggles">
          <Toggle param={k('bpm')} {color} />
          <Toggle param={k('x10')} {color} />
          <Toggle param={k('poly')} {color} />
        </div>
      </div>
    </div>
  {/key}
</section>

<style>
  .lfo {
    display: grid;
    grid-template-rows: auto 1fr;
    gap: 6px;
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
    gap: 2px;
  }
  .tabs button {
    font: 600 9.5px var(--font-num);
    color: var(--text-dim);
    background: transparent;
    border: 1px solid var(--line);
    border-radius: 4px;
    width: 22px;
    padding: 2px 0;
    cursor: pointer;
  }
  .tabs button.on {
    color: var(--text);
    border-color: var(--lfo);
  }
  .handles {
    display: flex;
    gap: 4px;
  }
  .body {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px;
    min-height: 0;
  }
  .editor {
    display: grid;
    gap: 4px;
    align-content: start;
  }
  .preset {
    font: 10px var(--font-ui);
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
  }
  .controls {
    display: grid;
    gap: 4px;
    align-content: start;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .selects {
    gap: 4px;
  }
  .sync {
    display: grid;
    gap: 2px;
  }
  .toggles {
    gap: 4px;
  }
</style>
