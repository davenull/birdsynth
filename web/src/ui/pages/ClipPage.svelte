<!--
  CLIP page: the transport, the twelve clips (pick one to edit and to play),
  its piano roll and one of its automation lanes. Record writes what you
  play into the clip while the transport runs; Import reads a MIDI file.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { FLAG, PARAMS, PARAM_ID, type ParamInfo, type ParamKey } from '../../gen/params';
  import { TEL } from '../../gen/protocol';
  import type { Synth } from '../../synth';
  import { CLIP_LANES, CLIP_NOTES, CLIPS } from '../../state/seq';
  import { toNorm, toPlain } from '../../state/param-math';
  import { parseSmf, writeSmf } from '../../midi/smf';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import PianoRoll from '../seq/PianoRoll.svelte';
  import AutoLane from '../seq/AutoLane.svelte';
  import { download, pick } from '../browser/files';
  import { onFrame } from '../frame';

  const synth = getContext<Synth>('synth');
  const color = 'var(--macro)';
  let slot = $state(0);
  let playingSlot = $state(-1);
  let playing = $state(false);
  let recording = $state(false);
  let beat = $state(0);
  let grid = $state(0.25);
  let lane = $state(0);
  let version = $state(0);
  let message = $state('');
  let linked = $state(synth.link.linked);

  const plain = (k: string) => toPlain(PARAMS[PARAM_ID[k as ParamKey]], synth.bank.get(PARAM_ID[k as ParamKey]));
  const setPlain = (k: string, v: number) => {
    const p = PARAMS[PARAM_ID[k as ParamKey]];
    synth.bank.set(p.id, toNorm(p, v));
  };

  onMount(() => {
    const read = () => (slot = plain('clip.slot') - 1);
    read();
    const offs = [
      synth.bank.subscribe(PARAM_ID['clip.slot'], read),
      synth.clips.subscribe(() => version++),
      synth.link.subscribe(() => (linked = synth.link.linked)),
      onFrame(() => {
        const t = synth.host?.tel;
        if (!t) return;
        playing = t[TEL.playing] >= 0.5;
        playingSlot = t[TEL.clipPlaying];
        beat = t[TEL.beat];
        recording = !!synth.recording;
      }),
    ];
    return () => offs.forEach((f) => f());
  });

  // (a fresh object each edit: the store edits its clips in place)
  const clip = $derived.by(() => {
    void version;
    return { ...synth.clips.clips[slot] };
  });
  const empty = (s: number) => {
    void version;
    return synth.clips.isEmpty(s);
  };
  // (an imported clip can have a length not in the list: offer it too)
  const lengths = $derived([...new Set([1, 2, 4, 8, 16, 32, 64, 128, 256, clip.length])].sort((a, b) => a - b));
  const lengthName = (l: number) => (l < 4 ? `${l} beat${l === 1 ? '' : 's'}` : l % 4 ? `${l} beats` : `${l / 4} bar${l === 4 ? '' : 's'}`);
  // the parameters a lane can automate (the modulatable ones), by section
  const AUTO = Object.entries(
    PARAMS.filter((p) => p.flags & FLAG.mod).reduce<Record<string, ParamInfo[]>>((g, p) => {
      (g[p.group] ??= []).push(p);
      return g;
    }, {}),
  );

  function choose(s: number): void {
    setPlain('clip.slot', s + 1);
  }

  async function importMidi(): Promise<void> {
    const [f] = await pick('.mid,.midi,audio/midi');
    if (!f) return;
    try {
      const smf = parseSmf(await f.arrayBuffer());
      synth.clips.importSmf(slot, smf);
      message = `Imported ${Math.min(smf.notes.length, CLIP_NOTES)} notes from ${f.name}${smf.notes.length > CLIP_NOTES ? ` (of ${smf.notes.length})` : ''}`;
    } catch (e) {
      message = `${f.name}: ${(e as Error).message}`;
    }
  }

  function exportMidi(): void {
    const c = synth.clips.clips[slot];
    download(`clip ${slot + 1}.mid`, writeSmf(c.notes.map((n) => ({ ...n, channel: 0 })), 480, plain('global.bpm')), 'audio/midi');
  }

  const bars = (b: number) => `${Math.floor(b / 4) + 1}.${(Math.floor(b) % 4) + 1}`;
</script>

<div class="clip-page">
  <section class="panel top" aria-label="Transport and clips" data-explain="clip">
    <div class="transport" data-explain="transport">
      <button class="play" class:on={playing} aria-pressed={playing} aria-label={playing ? 'Stop' : 'Play'} onclick={() => synth.transport(!playing)}>{playing ? '■' : '▶'}</button>
      <button
        class="rec"
        class:on={recording}
        aria-pressed={recording}
        aria-label="Record into this clip"
        title="Record what you play into this clip (overdub)"
        onclick={() => (synth.recording ? synth.stopRecording() : synth.record(slot))}>●</button
      >
      <span class="pos">{bars(beat)}</span>
      {#if linked}<span class="linked" title="Linked with other tabs: Play and Stop start and stop them all">LINK</span>{/if}
      <Knob param="global.bpm" size={26} {color} />
      <Knob param="global.swing" size={26} {color} />
    </div>
    <Toggle param="clip.enable" label="CLIP" {color} power />
    <div class="slots" role="group" aria-label="Clip slots">
      {#each Array(CLIPS) as _, s (s)}
        <button class:on={s === slot} class:live={s === playingSlot} class:empty={empty(s)} aria-pressed={s === slot} aria-label={`Clip ${s + 1}${s === playingSlot ? ', playing' : ''}`} onclick={() => choose(s)}>{s + 1}</button>
      {/each}
    </div>
    <Select param="clip.quantize" />
    <Toggle param="clip.trigger_keys" {color} />
    <Toggle param="clip.loop" {color} />
    <Knob param="clip.transpose" size={26} {color} />
  </section>

  <section class="panel roll" aria-label={`Clip ${slot + 1}`} data-explain="clip.roll">
    <div class="bar">
      <b>CLIP {slot + 1}</b>
      <label
        >Length <select value={clip.length} onchange={(e) => synth.clips.edit(slot, (c) => (c.length = Number(e.currentTarget.value)))}
          >{#each lengths as l (l)}<option value={l}>{lengthName(l)}</option>{/each}</select
        ></label
      >
      <label>Grid <select bind:value={grid}>{#each [1, 0.5, 0.25, 0.125] as g (g)}<option value={g}>{`1/${4 / g}`}</option>{/each}</select></label>
      <span class="faint">{clip.notes.length} notes · click to add, drag to move or stretch, Alt-click to delete, scroll for velocity</span>
      <button onclick={importMidi}>Import MIDI…</button>
      <button onclick={exportMidi} disabled={!clip.notes.length}>Export MIDI</button>
      <button onclick={() => synth.clips.edit(slot, (c) => (c.notes = []))} disabled={!clip.notes.length}>Clear</button>
    </div>
    <div class="canvas"><PianoRoll {slot} {grid} {color} /></div>
    <div class="bar">
      <span class="faint">Automation</span>
      <select bind:value={lane} aria-label="Automation lane">{#each Array(CLIP_LANES) as _, l (l)}<option value={l}>Lane {l + 1}</option>{/each}</select>
      <select
        aria-label="Automated parameter"
        value={clip.lanes[lane].param ?? ''}
        onchange={(e) => {
          const v = e.currentTarget.value;
          synth.clips.edit(slot, (c) => {
            c.lanes[lane].param = v || null;
            if (v && !c.lanes[lane].points.length) c.lanes[lane].points = [[0, synth.bank.get(PARAM_ID[v as ParamKey])]];
          });
        }}
      >
        <option value="">(none)</option>
        {#each AUTO as [group, ps] (group)}
          <optgroup label={group}>{#each ps as p (p.key)}<option value={p.key}>{p.name}</option>{/each}</optgroup>
        {/each}
      </select>
      {#if message}<span class="msg">{message}</span>{/if}
    </div>
    <div class="canvas lane"><AutoLane {slot} {lane} /></div>
  </section>
</div>

<style>
  .clip-page {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 8px;
    height: 100%;
  }
  section {
    min-width: 0;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .transport {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-right: 10px;
    border-right: 1px solid var(--line);
  }
  button {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 3px 8px;
    cursor: pointer;
  }
  button:disabled {
    color: var(--text-faint);
    cursor: default;
  }
  .play,
  .rec {
    width: 30px;
    height: 26px;
    padding: 0;
    font-size: 13px;
  }
  .play.on {
    color: #111;
    background: #2ecc71;
    border-color: #2ecc71;
  }
  .rec {
    color: var(--clip);
  }
  .rec.on {
    color: #fff;
    background: var(--clip);
    border-color: var(--clip);
  }
  .linked {
    font: 600 9px var(--font-ui);
    letter-spacing: 0.08em;
    color: #111;
    background: var(--ctl);
    border-radius: 3px;
    padding: 1px 4px;
  }
  .pos {
    font: 12px var(--font-num);
    min-width: 44px;
    color: var(--text-dim);
  }
  .slots {
    display: grid;
    grid-template-columns: repeat(12, 26px);
    gap: 2px;
  }
  .slots button {
    padding: 3px 0;
    font: 10px var(--font-num);
  }
  .slots button.empty {
    color: var(--text-faint);
  }
  .slots button.on {
    border-color: var(--macro);
  }
  .slots button.live {
    background: color-mix(in srgb, #2ecc71 35%, transparent);
  }
  .roll {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto 70px;
    gap: 5px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    min-width: 0;
  }
  .bar b {
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--macro);
  }
  .bar label {
    display: flex;
    gap: 4px;
    align-items: center;
    color: var(--text-dim);
  }
  select {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
  }
  .faint {
    color: var(--text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }
  .msg {
    color: var(--text-dim);
  }
  .canvas {
    position: relative;
    min-height: 0;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
  }
</style>
