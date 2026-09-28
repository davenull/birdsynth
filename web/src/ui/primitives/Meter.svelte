<!--
  Stereo peak meter (dBFS) with a slow fall and a clip light that holds for
  two seconds.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Synth } from '../../synth';
  import { onFrame } from '../frame';

  let { synth }: { synth: Synth } = $props();

  const FLOOR = -60;
  let level = $state([FLOOR, FLOOR]);
  let clipAt = $state(-1e9);
  let now = $state(0);

  const toDb = (x: number) => (x > 1e-6 ? 20 * Math.log10(x) : FLOOR);
  const pct = (db: number) => `${Math.max(0, Math.min(1, (db - FLOOR) / (6 - FLOOR))) * 100}%`;

  onMount(() => {
    let last = performance.now();
    return onFrame((t) => {
      const dt = Math.min(0.1, (t - last) / 1000);
      last = t;
      now = t;
      const tel = synth.telemetry();
      const peaks = [tel.peakL, tel.peakR];
      level = level.map((l, i) => Math.max(toDb(peaks[i]), l - 24 * dt)); // falls 24 dB/s
      if (tel.peakL >= 1 || tel.peakR >= 1) clipAt = t;
    });
  });
</script>

<div class="meter" role="meter" aria-label="Output level" aria-valuemin={FLOOR} aria-valuemax={6} aria-valuenow={Math.round(Math.max(...level))}>
  {#each level as l, i (i)}
    <div class="bar"><div class="fill" style:width={pct(l)}></div></div>
  {/each}
  <div class="clip" class:on={now - clipAt < 2000} title="Clipping in the last 2 s"></div>
</div>

<style>
  .meter {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-rows: 1fr 1fr;
    gap: 2px 4px;
    align-items: center;
    width: 120px;
  }
  .bar {
    grid-column: 1;
    height: 4px;
    background: var(--knob-track);
    border-radius: 2px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: linear-gradient(90deg, #2ecc71 0 70%, #f1c40f 85%, var(--clip) 95%);
    background-size: 120px 100%;
  }
  .clip {
    grid-column: 2;
    grid-row: 1 / 3;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--knob-track);
  }
  .clip.on {
    background: var(--clip);
  }
</style>
