<!-- The sub oscillator: a plain shape an octave or more below the note. -->
<script lang="ts">
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';

  const color = 'var(--sub)';
</script>

<section class="panel src" data-explain="sub" aria-label="Sub oscillator" style:--accent={color}>
  <header><Toggle param="sub.enable" label="SUB" {color} power /></header>
  <Select param="sub.shape" label="" wide />
  <div class="row">
    <Knob param="sub.octave" size={24} {color} compact />
    <Knob param="sub.level" size={24} {color} compact />
    <Knob param="sub.pan" size={24} {color} compact />
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
  }
</style>
