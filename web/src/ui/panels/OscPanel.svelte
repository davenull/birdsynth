<!-- One main oscillator: wavetable, pitch, unison and warps. -->
<script lang="ts">
  import { getContext } from 'svelte';
  import type { ParamKey } from '../../gen/params';
  import type { Synth } from '../../synth';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import WtDisplay from '../graphs/WtDisplay.svelte';

  let { osc }: { osc: number } = $props();
  const synth = getContext<Synth>('synth');
  const L = $derived('abc'[osc]);
  const NAME = $derived(`Osc ${L.toUpperCase()}`);
  const color = $derived(`var(--osc-${L})`);
  const k = (s: string) => `osc.${L}.${s}` as ParamKey;
  /** Warp modes built so far: phase warps and cross-modulation (the distortion and filter warps arrive in P3). */
  const WARP_MAX = 45;

  let tableName = $state('Saw');
  let factory = $state<{ name: string; frames: number }[]>([]);
  let busy = $state(false);
  let error = $state('');
  let fileInput = $state<HTMLInputElement>();
  $effect(() => {
    tableName = synth.tables.osc[osc]?.name ?? 'Saw';
    synth.tables.factoryList().then((l) => (factory = l));
    return synth.tables.onChange((o, t) => {
      if (o === osc) tableName = t.name;
    });
  });

  async function pick(v: string) {
    error = '';
    if (v === '__import') return fileInput?.click();
    if (v === '__export') return exportWav();
    busy = true;
    try {
      await synth.tables.loadFactory(osc, v);
    } catch (e) {
      error = String((e as Error).message ?? e);
    } finally {
      busy = false;
    }
  }

  async function importFile(file: File) {
    busy = true;
    error = '';
    try {
      await synth.tables.importWav(osc, file.name, await file.arrayBuffer());
    } catch (e) {
      error = `Couldn't read ${file.name}: ${(e as Error).message ?? e}`;
    } finally {
      busy = false;
    }
  }

  function exportWav() {
    const blob = synth.tables.exportWav(osc);
    if (!blob) return;
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${tableName}.wav`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  }
</script>

<section
  class="panel osc"
  data-explain={`osc.${L}`}
  aria-label={NAME}
  style:--accent={color}
  ondragover={(e) => e.preventDefault()}
  ondrop={(e) => {
    e.preventDefault();
    const f = e.dataTransfer?.files[0];
    if (f) importFile(f);
  }}
>
  <header>
    <Toggle param={k('enable')} label={NAME.toUpperCase()} {color} power />
    <select class="table" aria-label={`${NAME} wavetable`} value={tableName} onchange={(e) => pick((e.currentTarget as HTMLSelectElement).value)} disabled={busy}>
      {#if !factory.some((t) => t.name === tableName)}<option value={tableName}>{tableName}</option>{/if}
      <optgroup label="Factory">
        {#each factory as t (t.name)}<option value={t.name}>{t.name}{t.frames > 1 ? ` (${t.frames})` : ''}</option>{/each}
      </optgroup>
      <optgroup label="File">
        <option value="__import">Import WAV…</option>
        <option value="__export">Export WAV…</option>
      </optgroup>
    </select>
    <div class="route"><Select param={k('route')} label="" /></div>
    <input bind:this={fileInput} type="file" accept=".wav,audio/wav" hidden onchange={(e) => {
      const f = (e.currentTarget as HTMLInputElement).files?.[0];
      if (f) importFile(f);
      (e.currentTarget as HTMLInputElement).value = '';
    }} />
  </header>
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="display"><WtDisplay {osc} {color} /></div>
  <div class="grid">
    <Knob param={k('wt_pos')} size={30} {color} />
    <Knob param={k('level')} size={30} {color} />
    <Knob param={k('pan')} size={30} {color} />
    <Knob param={k('unison')} size={30} {color} />
    <Knob param={k('detune')} size={30} {color} />
    <Knob param={k('blend')} size={30} {color} />
    <Knob param={k('octave')} size={30} {color} />
    <Knob param={k('semi')} size={30} {color} />
    <Knob param={k('fine')} size={30} {color} />
    <Knob param={k('coarse')} size={30} {color} />
    <Knob param={k('phase')} size={30} {color} />
    <Knob param={k('rand_phase')} size={30} {color} />
  </div>
  <div class="warps">
    <Select param={k('warp1_mode')} max={WARP_MAX} />
    <Knob param={k('warp1_amount')} label="Amt" size={26} {color} />
    <Select param={k('warp2_mode')} max={WARP_MAX} />
    <Knob param={k('warp2_amount')} label="Amt" size={26} {color} />
  </div>
  <details class="more">
    <summary>Unison and phase options</summary>
    <div class="grid small">
      <Knob param={k('width')} size={24} {color} />
      <Knob param={k('uni_range')} size={24} {color} />
      <Knob param={k('wt_spread')} size={24} {color} />
      <Knob param={k('warp_spread')} size={24} {color} />
    </div>
    <div class="row">
      <Select param={k('uni_mode')} />
      <Select param={k('uni_stack')} />
      <Select param={k('pitch_mode')} />
    </div>
    <div class="row">
      <Toggle param={k('wt_smooth')} {color} />
      <Toggle param={k('phase_mem')} {color} />
    </div>
  </details>
</section>

<style>
  .osc {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .table {
    flex: 1;
    min-width: 0;
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 2px 4px;
  }
  .display {
    height: 104px;
    flex: none;
  }
  .route :global(select) {
    max-width: 74px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    justify-items: center;
    row-gap: 2px;
  }
  .grid.small {
    grid-template-columns: repeat(4, 1fr);
  }
  .warps {
    display: grid;
    grid-template-columns: 1fr auto 1fr auto;
    align-items: end;
    gap: 4px;
  }
  .more summary {
    font-size: 10px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .more[open] {
    display: grid;
    gap: 4px;
  }
  .row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .error {
    margin: 0;
    font-size: 10px;
    color: var(--clip);
  }
</style>
