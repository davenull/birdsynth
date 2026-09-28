<!-- The noise oscillator: one of our generated noise recordings, looped or once. -->
<script lang="ts">
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';

  const color = 'var(--noise)';
</script>

<section class="panel src" data-explain="noise" aria-label="Noise oscillator" style:--accent={color}>
  <header><Toggle param="noise.enable" label="NOISE" {color} power /></header>
  <Select param="noise.type" label="" wide />
  <div class="row">
    <Knob param="noise.level" size={24} {color} compact />
    <Knob param="noise.pan" size={24} {color} compact />
    <Knob param="noise.pitch" size={24} {color} compact />
  </div>
  <div class="row">
    <Knob param="noise.phase" size={24} {color} compact />
    <Knob param="noise.rand" size={24} {color} compact />
    <div class="toggles">
      <Toggle param="noise.keytrack" {color} />
      <Toggle param="noise.oneshot" {color} />
    </div>
  </div>
</section>

<style>
  .src {
    display: grid;
    gap: 6px;
    align-content: start;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .toggles {
    display: grid;
    gap: 4px;
  }
</style>
