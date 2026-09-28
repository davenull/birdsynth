<!--
  Oscilloscope on one tap, triggered on a rising zero crossing so a steady
  tone stands still. It draws the frames the listener is hearing now, not
  the newest ones rendered.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { TAP, type TapName } from '../../gen/protocol';
  import type { Synth } from '../../synth';
  import { fitCanvas, onFrame } from '../frame';

  let {
    synth,
    tap,
    label,
    color = 'var(--accent)',
    span = 1024,
  }: { synth: Synth; tap: TapName; label: string; color?: string; span?: number } = $props();

  let canvas = $state<HTMLCanvasElement>();
  let live = $state(false);

  onMount(() => {
    const buf = new Float32Array(span * 2);
    let gain = 1;
    let stroke = '';
    return onFrame(() => {
      const c = canvas;
      const host = synth.host;
      if (!c || !host) return;
      const g = c.getContext('2d');
      if (!g) return;
      const { w, h, dpr } = fitCanvas(c);
      g.clearRect(0, 0, w, h);
      g.strokeStyle = 'rgba(255,255,255,0.07)';
      g.lineWidth = 1;
      g.beginPath();
      g.moveTo(0, h / 2);
      g.lineTo(w, h / 2);
      g.stroke();

      const idx = TAP[tap];
      const heard = Math.min(host.heardFrame(), host.taps.latest(idx));
      live = host.taps.read(idx, heard, buf);
      if (!live) return;

      // trigger: the first rising zero crossing in the first half, armed only
      // after the signal has dipped below -5% of peak (hysteresis against noise)
      let peak = 0;
      for (let i = 0; i < buf.length; i++) peak = Math.max(peak, Math.abs(buf[i]));
      const hyst = peak * 0.05;
      let armed = false;
      let t = 0;
      for (let i = 1; i < span; i++) {
        if (buf[i - 1] < -hyst) armed = true;
        if (armed && buf[i - 1] < 0 && buf[i] >= 0) {
          t = i;
          break;
        }
      }
      // auto-range with a slow release so the trace doesn't pump
      const target = peak > 1e-4 ? 0.9 / peak : 1;
      gain = target < gain ? target : gain + (target - gain) * 0.02;

      stroke ||= getComputedStyle(c).getPropertyValue('--trace').trim() || '#4ea3ff';
      g.strokeStyle = stroke;
      g.lineWidth = 1.5 * dpr;
      g.beginPath();
      for (let i = 0; i < span; i++) {
        const x = (i / (span - 1)) * w;
        const y = h / 2 - buf[t + i] * gain * (h / 2);
        if (i) g.lineTo(x, y);
        else g.moveTo(x, y);
      }
      g.stroke();
    });
  });
</script>

<figure class="scope" style:--trace={color}>
  <canvas bind:this={canvas} aria-label={`${label} scope`}></canvas>
  <figcaption>{label}{live ? '' : ' · no signal'}</figcaption>
</figure>

<style>
  .scope {
    margin: 0;
    display: grid;
    grid-template-rows: 1fr auto;
    gap: 4px;
    min-height: 0;
  }
  canvas {
    width: 100%;
    height: 100%;
    min-height: 80px;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: var(--radius);
  }
  figcaption {
    font-size: 10.5px;
    color: var(--text-dim);
  }
</style>
