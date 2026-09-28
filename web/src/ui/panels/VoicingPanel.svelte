<!-- Voicing: polyphony, mono and legato, glide, stealing and bend range. -->
<script lang="ts">
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';

  const color = 'var(--text-dim)';
</script>

<section class="panel voicing" data-explain="voice" aria-label="Voicing">
  <h2>VOICING</h2>
  <div class="row">
    <Knob param="voice.polyphony" label="Voices" size={28} {color} />
    <Knob param="voice.glide" size={28} {color} />
    <Knob param="voice.glide_curve" size={28} {color} />
    <Knob param="voice.bend_up" size={26} {color} />
    <Knob param="voice.bend_down" size={26} {color} />
  </div>
  <div class="row toggles">
    <Toggle param="voice.mono" />
    <Toggle param="voice.legato" />
    <Toggle param="voice.glide_always" />
    <Toggle param="voice.glide_scaled" />
    <Select param="voice.steal" />
  </div>
</section>

<style>
  .voicing {
    display: grid;
    gap: 6px;
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
    align-items: end;
    gap: 2px;
  }
  .toggles {
    gap: 5px;
    flex-wrap: wrap;
  }
</style>
