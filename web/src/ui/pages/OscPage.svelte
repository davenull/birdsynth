<!-- OSC page: sub and noise, the three wavetable oscillators, and both filters. -->
<script lang="ts">
  import OscPanel from '../panels/OscPanel.svelte';
  import SubPanel from '../panels/SubPanel.svelte';
  import NoisePanel from '../panels/NoisePanel.svelte';
  import FilterPanel from '../panels/FilterPanel.svelte';
</script>

<div class="osc-page">
  <div class="col"><SubPanel /><NoisePanel /></div>
  <OscPanel osc={0} />
  <OscPanel osc={1} />
  <OscPanel osc={2} />
  <div class="col filters"><FilterPanel n={1} /><FilterPanel n={2} /></div>
</div>

<style>
  .osc-page {
    display: grid;
    grid-template-columns: 150px 1fr 1fr 1fr 206px;
    gap: 8px;
    min-height: 0;
    height: 100%;
  }
  .col {
    display: grid;
    grid-template-rows: auto 1fr;
    gap: 8px;
    min-height: 0;
  }
  .filters {
    grid-template-rows: 1fr 1fr;
  }
</style>
