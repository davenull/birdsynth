<!-- The editor for one FX module: every control from its spec, presets, and the module's own extras. -->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { TEL } from '../../gen/protocol';
  import { MODULE_PRESETS, CONVOLVE, FX_TYPES, moduleParams, refName, refParam, type FxRef } from '../../state/fx';
  import { USER_IR } from '../../state/ir';
  import { toNorm, toPlain } from '../../state/param-math';
  import { PARAMS, PARAM_ID } from '../../gen/params';
  import type { Synth } from '../../synth';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import { onFrame } from '../frame';
  import { filterGroup } from '../../state/filter-types';

  let { r }: { r: FxRef } = $props();
  const synth = getContext<Synth>('synth');
  const COMPRESSOR = 6;
  const FILTER = 9;
  const params = $derived(moduleParams(r).filter((p) => p.local !== 'enable'));
  const presets = $derived(MODULE_PRESETS[FX_TYPES[r.type].key] ?? {});
  const color = 'var(--fx)';

  // compressor gain reduction, per band (one in Single mode, three in Multiband)
  const BANDS = ['Low', 'Mid', 'High'];
  let gr = $state([0, 0, 0]);
  let multiband = $state(false);
  onMount(() =>
    onFrame(() => {
      if (r.type !== COMPRESSOR) return;
      const t = synth.host?.tel;
      if (t) gr = [0, 1, 2].map((b) => t[TEL.fxGr + r.inst * 3 + b]);
    }),
  );
  $effect(() => {
    if (r.type !== COMPRESSOR) return;
    const info = PARAMS[PARAM_ID[refParam(r, 'mode')]];
    const read = () => (multiband = toPlain(info, synth.bank.get(info.id)) >= 0.5);
    read();
    return synth.bank.subscribe(info.id, read);
  });

  // convolver response
  let irName = $state('');
  $effect(() => {
    if (r.type !== CONVOLVE) return;
    const inst = r.inst;
    irName = synth.ir.name(inst);
    const off1 = synth.ir.subscribe((i) => i === inst && (irName = synth.ir.name(inst)));
    const off2 = synth.bank.subscribe(PARAM_ID[refParam(r, 'ir')], () => (irName = synth.ir.name(inst)));
    return () => (off1(), off2());
  });
  let dropError = $state('');
  async function dropIr(file: File): Promise<void> {
    dropError = '';
    const ctx = synth.host?.ctx;
    if (!ctx) return;
    try {
      const audio = await ctx.decodeAudioData(await file.arrayBuffer());
      const ch = Array.from({ length: Math.min(2, audio.numberOfChannels) }, (_, c) => audio.getChannelData(c).slice());
      synth.ir.setUser(r.inst, file.name.replace(/\.[^.]+$/, ''), ch, audio.sampleRate);
      const info = PARAMS[PARAM_ID[refParam(r, 'ir')]];
      synth.bank.set(info.id, toNorm(info, USER_IR));
    } catch (e) {
      dropError = `Couldn't read ${file.name}: ${(e as Error).message ?? e}`;
    }
  }
</script>

<section class="panel module" data-explain={`fx.${FX_TYPES[r.type].key}`} aria-label={refName(r)} style:--accent={color}>
  <header>
    <Toggle param={refParam(r, 'enable')} label={refName(r).toUpperCase()} {color} power />
    {#if Object.keys(presets).length}
      <select class="preset" aria-label={`${refName(r)} preset`} value="" onchange={(e) => {
        const name = (e.currentTarget as HTMLSelectElement).value;
        (e.currentTarget as HTMLSelectElement).value = '';
        if (presets[name]) synth.fx.applyPreset(r, presets[name]);
      }}>
        <option value="">Preset…</option>
        {#each Object.keys(presets) as name (name)}<option value={name}>{name}</option>{/each}
      </select>
    {/if}
    <button class="reset" onclick={() => synth.fx.resetModule(r)}>Reset</button>
  </header>
  <div class="controls">
    {#each params as p (p.id)}
      {#if p.curve.kind === 'enum'}
        <Select param={p.key as never} wide group={r.type === FILTER && p.local === 'type' ? filterGroup : undefined} />
      {:else if p.curve.kind === 'bool'}
        <Toggle param={p.key as never} {color} />
      {:else}
        <Knob param={p.key as never} size={30} {color} />
      {/if}
    {/each}
  </div>
  {#if r.type === COMPRESSOR}
    <div class="gr" role="group" aria-label="Gain reduction">
      <h3>Gain reduction</h3>
      {#each multiband ? [0, 1, 2] : [0] as b (b)}
        {@const g = gr[b]}
        <div class="band">
          <span class="name">{multiband ? BANDS[b] : 'All'}</span>
          <div class="meter"><span style:width={`${Math.min(100, (Math.abs(g) / 24) * 100)}%`} class:up={g > 0}></span></div>
          <span class="value">{g > 0 ? '+' : ''}{g.toFixed(1)} dB</span>
        </div>
      {/each}
    </div>
  {/if}
  {#if r.type === CONVOLVE}
    <div
      class="drop"
      role="region"
      aria-label="Impulse response"
      ondragover={(e) => e.preventDefault()}
      ondrop={(e) => {
        e.preventDefault();
        const f = e.dataTransfer?.files[0];
        if (f) void dropIr(f);
      }}
    >
      <b>{irName}</b> · drop an audio file here to use your own impulse response
      {#if dropError}<p class="error">{dropError}</p>{/if}
    </div>
  {/if}
</section>

<style>
  .module {
    display: grid;
    grid-template-rows: auto auto 1fr;
    gap: 8px;
    height: 100%;
    min-height: 0;
    align-content: start;
  }
  header {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .preset,
  .reset {
    font: 11px var(--font-ui);
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 2px 6px;
  }
  .reset {
    margin-left: auto;
    cursor: pointer;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 4px;
    align-content: start;
    align-items: end;
  }
  /* in the middle of the room the knobs leave */
  .gr {
    align-self: center;
    justify-self: center;
    width: min(100%, 460px);
    display: grid;
    gap: 12px;
  }
  .gr h3 {
    margin: 0;
    font: 600 10px var(--font-ui);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .band {
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr) 64px;
    align-items: center;
    gap: 10px;
  }
  .name {
    font-size: 11px;
    color: var(--text-dim);
  }
  .value {
    font: 11px var(--font-num);
    color: var(--text);
    text-align: right;
  }
  .meter {
    position: relative;
    height: 14px;
    background: var(--glass);
    border-radius: 4px;
    overflow: hidden;
  }
  .meter span {
    position: absolute;
    right: 0;
    top: 0;
    bottom: 0;
    background: var(--fx);
  }
  .meter span.up {
    left: 0;
    right: auto;
    background: var(--env);
  }
  .drop {
    align-self: end;
    font-size: 11px;
    color: var(--text-dim);
    border: 1px dashed var(--line);
    border-radius: 6px;
    padding: 10px;
  }
  .drop b {
    color: var(--text);
  }
  .error {
    color: var(--clip);
    margin: 4px 0 0;
  }
</style>
