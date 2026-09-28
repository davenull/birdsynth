<!-- GLOBAL page: quality, tempo and rate scaling, bend range and voice limits. Tuning and MIDI arrive with P4. -->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { TEL } from '../../gen/protocol';
  import type { Synth } from '../../synth';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import { onFrame } from '../frame';

  const synth = getContext<Synth>('synth');
  let os = $state(1);
  onMount(() =>
    onFrame(() => {
      const t = synth.host?.tel;
      if (t && t[TEL.oversample] !== os) os = t[TEL.oversample];
    }),
  );
  const color = 'var(--text-dim)';
</script>

<div class="global-page">
  <section class="panel" data-explain="global.quality" aria-label="Quality">
    <h2>QUALITY</h2>
    <Select param="global.quality" wide />
    <p>Oversampling runs only while a warp that needs it (sync, FM, PD, AM, RM) is in use. Now: <b>{os}×</b></p>
  </section>
  <section class="panel" data-explain="global.tempo" aria-label="Tempo">
    <h2>TEMPO</h2>
    <div class="row">
      <Knob param="global.bpm" size={34} {color} />
      <Knob param="global.env_rate" size={30} {color} />
      <Knob param="global.lfo_rate" size={30} {color} />
    </div>
    <p>BPM-synced LFOs and envelopes follow this tempo; the rate knobs speed every envelope or LFO up or down together.</p>
  </section>
  <section class="panel" data-explain="voice.bend" aria-label="Pitch bend">
    <h2>PITCH BEND</h2>
    <div class="row">
      <Knob param="voice.bend_up" size={30} {color} />
      <Knob param="voice.bend_down" size={30} {color} />
    </div>
  </section>
  <section class="panel" data-explain="voice" aria-label="Voices">
    <h2>VOICES</h2>
    <div class="row">
      <Knob param="voice.polyphony" size={30} {color} />
      <Select param="voice.steal" wide />
    </div>
  </section>
  <section class="panel" data-explain="master" aria-label="Master">
    <h2>MASTER</h2>
    <div class="row"><Knob param="master.volume" size={34} {color} /></div>
  </section>
</div>

<style>
  .global-page {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    grid-auto-rows: min-content;
    gap: 8px;
    align-content: start;
  }
  section {
    display: grid;
    gap: 8px;
    align-content: start;
  }
  h2 {
    margin: 0;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.1em;
    color: var(--text-dim);
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: end;
  }
  p {
    margin: 0;
    font-size: 11px;
    color: var(--text-dim);
  }
</style>
