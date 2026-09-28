<!--
  One main oscillator. Its Type picks what it plays: a wavetable (pitch,
  unison and warps), a recording (Sample), a multisample, grains of a
  recording (Granular), or a recording rebuilt from its spectrum (Spectral).
  Drop a file on the panel to load it the way the type wants.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { ParamKey } from '../../gen/params';
  import type { Synth } from '../../synth';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import WtDisplay from '../graphs/WtDisplay.svelte';
  import RemapEditor from '../graphs/RemapEditor.svelte';
  import SampleView from './osc/SampleView.svelte';
  import ZoneMap from './osc/ZoneMap.svelte';
  import SpectrogramView from './osc/SpectrogramView.svelte';
  import SpecFilterEditor from './osc/SpecFilterEditor.svelte';
  import XYPad from './osc/XYPad.svelte';
  import { PARAMS, PARAM_ID } from '../../gen/params';
  import { toPlain } from '../../state/param-math';
  import { TYPE } from '../../state/recordings';
  import { pictureToAnalysis } from '../../state/spectral';
  import { editorView } from '../../editor/editor.svelte';
  import { sampleEditor } from '../../sampler/sampler.svelte';

  let { osc }: { osc: number } = $props();
  const synth = getContext<Synth>('synth');
  const L = $derived('abc'[osc]);
  const NAME = $derived(`Osc ${L.toUpperCase()}`);
  const color = $derived(`var(--osc-${L})`);
  const k = (s: string) => `osc.${L}.${s}` as ParamKey;
  /** Every warp mode: phase, cross-modulation, distortion and filter warps. */
  const WARP_MAX = 61;

  const plain = (key: ParamKey) => toPlain(PARAMS[PARAM_ID[key]], synth.bank.get(PARAM_ID[key]));
  let type = $state(0);
  let remapOn = $state(false);
  let filterOn = $state(false);
  $effect(() => {
    const ids = [PARAM_ID[k('warp1_mode')], PARAM_ID[k('warp2_mode')]];
    const read = () => {
      remapOn = ids.some((id) => [12, 13, 14, 15].includes(toPlain(PARAMS[id], synth.bank.get(id))));
      type = plain(k('type'));
      filterOn = plain(k('sp_filter')) >= 0.5;
    };
    read();
    const offs = [...ids, PARAM_ID[k('type')], PARAM_ID[k('sp_filter')]].map((id) => synth.bank.subscribe(id, read));
    return () => offs.forEach((f) => f());
  });

  let tableName = $state('Saw');
  let factory = $state<{ name: string; frames: number }[]>([]);
  let multiNames = $state<string[]>([]);
  let recName = $state('');
  let multiName = $state('');
  let busy = $state(false);
  let error = $state('');
  let fileInput = $state<HTMLInputElement>();
  let audioInput = $state<HTMLInputElement>();
  let pictureInput = $state<HTMLInputElement>();
  let sfzInput = $state<HTMLInputElement>();
  onMount(() => {
    tableName = synth.tables.osc[osc]?.name ?? 'Saw';
    synth.tables.factoryList().then((l) => (factory = l));
    synth.multis.factoryList().then((l) => (multiNames = l));
    const readRec = () => (recName = synth.recordings.picture[osc]?.name ?? synth.recordings.osc[osc]?.name ?? '');
    const readMulti = () => (multiName = synth.multis.osc[osc]?.name ?? '');
    readRec();
    readMulti();
    const offs = [
      synth.tables.onChange((o, t) => {
        if (o === osc) tableName = t.name;
      }),
      synth.recordings.subscribe((o) => o === osc && readRec()),
      synth.multis.subscribe((o) => o === osc && readMulti()),
    ];
    return () => offs.forEach((f) => f());
  });

  async function run(fn: () => Promise<unknown>, what: string): Promise<void> {
    busy = true;
    error = '';
    try {
      await fn();
    } catch (e) {
      error = `${what}: ${(e as Error).message ?? e}`;
    } finally {
      busy = false;
    }
  }

  function pick(v: string) {
    error = '';
    if (v === '__import') return fileInput?.click();
    if (v === '__export') return exportWav();
    if (v === '__edit') return editorView.open(synth.tables, osc);
    if (v === '__load') return audioInput?.click();
    if (v === '__pic') return pictureInput?.click();
    if (v === '__sfz') return sfzInput?.click();
    if (v === '__sedit') return sampleEditor.open(osc);
    if (v.startsWith('multi:')) return run(() => synth.multis.loadFactory(osc, v.slice(6)), 'Couldn’t load it');
    if (v.startsWith('table:')) return run(() => synth.tables.loadFactory(osc, v.slice(6)), 'Couldn’t load it');
  }

  const importWav = (file: File) => run(async () => synth.tables.importWav(osc, file.name, await file.arrayBuffer()), `Couldn’t read ${file.name}`);
  const importAudio = (file: File) => run(() => synth.recordings.importFile(osc, file), `Couldn’t read ${file.name}`);
  const importSfz = (files: File[]) =>
    run(async () => {
      const sfz = files.find((f) => /\.sfz$/i.test(f.name));
      if (!sfz) throw new Error('pick the .sfz file together with its samples');
      const m = await synth.multis.loadSfz(osc, sfz, files.filter((f) => f !== sfz).map((f) => ({ name: f.name, path: (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name, arrayBuffer: () => f.arrayBuffer() })));
      if (m.warnings.length) error = m.warnings.slice(0, 2).join('; ');
    }, 'Couldn’t load the SFZ');
  const importPicture = (file: File) =>
    run(async () => {
      const bmp = await createImageBitmap(file);
      const c = new OffscreenCanvas(Math.min(1024, bmp.width), Math.min(512, bmp.height));
      const g = c.getContext('2d')!;
      g.drawImage(bmp, 0, 0, c.width, c.height);
      const img = g.getImageData(0, 0, c.width, c.height);
      const rate = synth.host?.ctx.sampleRate ?? 48_000;
      const seconds = 4;
      await synth.recordings.setPicture(osc, { name: file.name.replace(/\.[^.]+$/, ''), bytes: await file.arrayBuffer(), seconds }, pictureToAnalysis(img, seconds, rate));
    }, `Couldn’t read ${file.name}`);

  function drop(files: File[]): void {
    if (!files.length) return;
    if (type === TYPE.wavetable) return void importWav(files[0]);
    if (type === TYPE.multi) return void importSfz(files);
    if (type === TYPE.spectral && files[0].type.startsWith('image/')) return void importPicture(files[0]);
    void importAudio(files[0]);
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

  const current = $derived(type === TYPE.wavetable ? `table:${tableName}` : type === TYPE.multi ? `multi:${multiName}` : '__rec');
</script>

<section
  class="panel osc"
  data-explain={`osc.${L}`}
  aria-label={NAME}
  style:--accent={color}
  ondragover={(e) => e.preventDefault()}
  ondrop={(e) => {
    e.preventDefault();
    drop([...(e.dataTransfer?.files ?? [])]);
  }}
>
  <header>
    <Toggle param={k('enable')} label={NAME.toUpperCase()} {color} power />
    <div class="type"><Select param={k('type')} label="" /></div>
    <select
      class="table"
      aria-label={`${NAME} source`}
      value={current}
      onchange={(e) => {
        const el = e.currentTarget as HTMLSelectElement;
        const v = el.value;
        el.value = current; // actions aren't sources: keep showing what's loaded
        void pick(v);
      }}
      disabled={busy}
    >
      {#if type === TYPE.wavetable}
        {#if !factory.some((t) => t.name === tableName)}<option value={`table:${tableName}`}>{tableName}</option>{/if}
        <optgroup label="Factory">
          {#each factory as t (t.name)}<option value={`table:${t.name}`}>{t.name}{t.frames > 1 ? ` (${t.frames})` : ''}</option>{/each}
        </optgroup>
        <optgroup label="File">
          <option value="__edit">Edit…</option>
          <option value="__import">Import WAV…</option>
          <option value="__export">Export WAV…</option>
        </optgroup>
      {:else if type === TYPE.multi}
        {#if !multiNames.includes(multiName)}<option value={`multi:${multiName}`}>{multiName || 'No multisample'}</option>{/if}
        <optgroup label="Factory">
          {#each multiNames as n (n)}<option value={`multi:${n}`}>{n}</option>{/each}
        </optgroup>
        <optgroup label="File"><option value="__sfz">Load SFZ…</option></optgroup>
      {:else}
        <option value="__rec">{recName || 'No recording'}</option>
        <option value="__load">Load audio…</option>
        {#if type === TYPE.spectral}<option value="__pic">Load picture…</option>{/if}
        <option value="__sedit">Edit…</option>
      {/if}
    </select>
    <button
      class="edit"
      aria-label={`Edit ${NAME}'s ${type === TYPE.wavetable ? 'wavetable' : 'recording'}`}
      title={type === TYPE.wavetable ? 'Edit this wavetable' : 'Edit this recording'}
      onclick={() => (type === TYPE.wavetable ? editorView.open(synth.tables, osc) : sampleEditor.open(osc))}>✎</button
    >
    <div class="route"><Select param={k('route')} label="" /></div>
    <input bind:this={fileInput} type="file" accept=".wav,audio/wav" hidden onchange={(e) => {
      const f = (e.currentTarget as HTMLInputElement).files?.[0];
      if (f) void importWav(f);
      (e.currentTarget as HTMLInputElement).value = '';
    }} />
    <input bind:this={audioInput} type="file" accept="audio/*,.wav,.aif,.aiff,.flac,.mp3,.ogg,.m4a" hidden onchange={(e) => {
      const f = (e.currentTarget as HTMLInputElement).files?.[0];
      if (f) void importAudio(f);
      (e.currentTarget as HTMLInputElement).value = '';
    }} />
    <input bind:this={pictureInput} type="file" accept="image/*" hidden onchange={(e) => {
      const f = (e.currentTarget as HTMLInputElement).files?.[0];
      if (f) void importPicture(f);
      (e.currentTarget as HTMLInputElement).value = '';
    }} />
    <input bind:this={sfzInput} type="file" accept=".sfz,audio/*,.wav,.flac,.aif,.aiff" multiple hidden onchange={(e) => {
      const fs = [...((e.currentTarget as HTMLInputElement).files ?? [])];
      if (fs.length) void importSfz(fs);
      (e.currentTarget as HTMLInputElement).value = '';
    }} />
  </header>
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if type === TYPE.wavetable}
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
    {#if remapOn}
      <div class="remap"><span>Remap curve</span><RemapEditor {osc} {color} width={170} height={64} /></div>
    {/if}
  {:else if type === TYPE.sample}
    <div class="display"><SampleView {osc} {color} /></div>
    <div class="grid">
      <Knob param={k('smp_start')} size={30} {color} />
      <Knob param={k('smp_end')} size={30} {color} />
      <Knob param={k('loop_start')} size={30} {color} />
      <Knob param={k('loop_end')} size={30} {color} />
      <Knob param={k('xfade')} size={30} {color} />
      <Knob param={k('rate')} size={30} {color} />
      <Knob param={k('level')} size={30} {color} />
      <Knob param={k('pan')} size={30} {color} />
      <Knob param={k('octave')} size={30} {color} />
      <Knob param={k('semi')} size={30} {color} />
      <Knob param={k('fine')} size={30} {color} />
      <Knob param={k('root')} size={30} {color} />
    </div>
    <div class="row">
      <Select param={k('loop_mode')} />
      <Toggle param={k('snap')} {color} />
      <Toggle param={k('keytrack')} {color} />
      <Toggle param={k('slice')} {color} />
      <Toggle param={k('tail')} {color} />
    </div>
  {:else if type === TYPE.multi}
    <div class="display"><ZoneMap {osc} {color} /></div>
    <div class="grid">
      <Knob param={k('level')} size={30} {color} />
      <Knob param={k('pan')} size={30} {color} />
      <Knob param={k('octave')} size={30} {color} />
      <Knob param={k('semi')} size={30} {color} />
      <Knob param={k('fine')} size={30} {color} />
      <Knob param={k('rate')} size={30} {color} />
    </div>
  {:else if type === TYPE.granular}
    <div class="display split">
      <SampleView {osc} {color} mode="granular" />
      <XYPad x={k('gr_position')} y={k('gr_length')} {color} />
    </div>
    <div class="grid">
      <Knob param={k('gr_density')} size={30} {color} />
      <Knob param={k('gr_length')} size={30} {color} />
      <Knob param={k('gr_position')} size={30} {color} />
      <Knob param={k('gr_scan')} size={30} {color} />
      <Knob param={k('gr_spray')} size={30} {color} />
      <Knob param={k('gr_pitch_rand')} size={30} {color} />
      <Knob param={k('gr_pan')} size={30} {color} />
      <Knob param={k('gr_timbre')} size={30} {color} />
      <Knob param={k('level')} size={30} {color} />
      <Knob param={k('octave')} size={30} {color} />
      <Knob param={k('semi')} size={30} {color} />
      <Knob param={k('fine')} size={30} {color} />
    </div>
    <div class="row">
      <Select param={k('gr_window')} />
      <Select param={k('gr_direction')} />
      <Toggle param={k('gr_sync')} {color} />
      <Toggle param={k('gr_loop')} {color} />
    </div>
  {:else}
    <div class="display" class:halves={filterOn}>
      <SpectrogramView {osc} {color} />
      {#if filterOn}<SpecFilterEditor {osc} {color} />{/if}
    </div>
    <div class="grid">
      <Knob param={k('sp_position')} size={30} {color} />
      <Knob param={k('sp_scan')} size={30} {color} />
      <Knob param={k('sp_low')} size={30} {color} />
      <Knob param={k('sp_high')} size={30} {color} />
      <Knob param={k('sp_warp_amt')} size={30} {color} />
      <Knob param={k('root')} size={30} {color} />
      <Knob param={k('level')} size={30} {color} />
      <Knob param={k('pan')} size={30} {color} />
      <Knob param={k('octave')} size={30} {color} />
      <Knob param={k('semi')} size={30} {color} />
      <Knob param={k('fine')} size={30} {color} />
      <Knob param={k('coarse')} size={30} {color} />
    </div>
    <div class="row">
      <Select param={k('sp_warp')} />
      <Toggle param={k('sp_loop')} {color} />
      <Toggle param={k('sp_filter')} {color} />
      <Toggle param={k('keytrack')} {color} />
    </div>
  {/if}
  <details class="more">
    <summary>{type === TYPE.wavetable ? 'Unison and phase options' : type === TYPE.sample ? 'Unison options' : 'Pitch options'}</summary>
    {#if type === TYPE.wavetable || type === TYPE.sample}
      <div class="grid small">
        <Knob param={k('unison')} size={24} {color} />
        <Knob param={k('detune')} size={24} {color} />
        <Knob param={k('width')} size={24} {color} />
        <Knob param={k('uni_range')} size={24} {color} />
        {#if type === TYPE.wavetable}
          <Knob param={k('wt_spread')} size={24} {color} />
          <Knob param={k('warp_spread')} size={24} {color} />
        {:else}
          <Knob param={k('blend')} size={24} {color} />
        {/if}
      </div>
      <div class="row">
        <Select param={k('uni_mode')} />
        <Select param={k('uni_stack')} />
        {#if type === TYPE.wavetable}<Select param={k('pitch_mode')} />{/if}
      </div>
    {/if}
    {#if type === TYPE.wavetable}
      <div class="row">
        <Toggle param={k('wt_smooth')} {color} />
        <Toggle param={k('phase_mem')} {color} />
      </div>
    {:else}
      <div class="row">
        <Knob param={k('coarse')} size={24} {color} />
        {#if type !== TYPE.spectral}<Knob param={k('root')} size={24} {color} />{/if}
        {#if type === TYPE.granular}<Toggle param={k('keytrack')} {color} />{/if}
      </div>
    {/if}
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
  .edit {
    flex: none;
    width: 22px;
    height: 20px;
    padding: 0;
    font-size: 12px;
    line-height: 1;
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    cursor: pointer;
  }
  .edit:hover {
    color: var(--text);
    border-color: var(--accent);
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
  .display.split {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 4px;
  }
  .display.halves {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
  }
  .type :global(select) {
    max-width: 88px;
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
  .remap {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 10px;
    color: var(--text-dim);
  }
  .error {
    margin: 0;
    font-size: 10px;
    color: var(--clip);
  }
</style>
