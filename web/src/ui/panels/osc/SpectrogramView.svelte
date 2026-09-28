<!--
  The Spectral type's analysis as a picture (time across, log frequency
  up, brighter is louder), with the newest voice's position, and the drawn
  filter over it while it's on.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAM_ID, type ParamKey } from '../../../gen/params';
  import { TEL } from '../../../gen/protocol';
  import type { Synth } from '../../../synth';
  import { fitCanvas, onFrame } from '../../frame';

  let { osc, color = 'var(--osc-a)' }: { osc: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  const L = $derived('abc'[osc]);
  let canvas = $state<HTMLCanvasElement>();
  let has = $state(false);
  let img: ImageData | null = null;
  let imgFor: unknown = null;
  const BINS = 1025;
  const FF = 2 * BINS + 1;

  onMount(() => {
    const off = synth.recordings.subscribe((o) => o === osc && draw());
    const offF = synth.specFilters.subscribe((o) => o === osc && draw());
    const offP = synth.bank.subscribe(PARAM_ID[`osc.${L}.sp_filter` as ParamKey], () => draw());
    let play = -2;
    const offFrame = onFrame(() => {
      const p = synth.host?.tel[TEL.oscPlay + osc] ?? -1;
      if (Math.abs(p - play) > 1e-4) {
        play = p;
        draw();
      }
    });
    draw();
    return () => {
      off();
      offF();
      offP();
      offFrame();
    };
  });

  function draw(): void {
    const c = canvas;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    g.clearRect(0, 0, w, h);
    const a = synth.recordings.spectrum(osc);
    has = !!a;
    if (!a) return;
    if (imgFor !== a || !img || img.width !== w || img.height !== h) {
      imgFor = a;
      img = g.createImageData(w, h);
      // brightness on a 70 dB scale from the loudest bin
      let top = 1e-9;
      for (let m = 0; m < a.frames; m += 4) for (let k = 1; k < BINS; k += 4) top = Math.max(top, a.data[m * FF + k]);
      const [cr, cg, cb] = [95, 211, 255];
      for (let x = 0; x < w; x++) {
        const m = Math.min(a.frames - 1, Math.floor((x / w) * a.frames));
        for (let y = 0; y < h; y++) {
          const hz = 20 * 1000 ** (1 - y / h);
          const k = Math.min(BINS - 1, Math.max(1, Math.round((hz * 2048) / a.rate)));
          const v = a.data[m * FF + k] / top;
          const t = Math.max(0, 1 + (20 * Math.log10(v + 1e-9)) / 70);
          const i = (y * w + x) * 4;
          img.data[i] = cr * t;
          img.data[i + 1] = cg * t;
          img.data[i + 2] = cb * t;
          img.data[i + 3] = 255;
        }
      }
    }
    g.putImageData(img, 0, 0);
    // the filter curve
    const on = synth.bank.get(PARAM_ID[`osc.${L}.sp_filter` as ParamKey]) >= 0.5;
    if (on) {
      const pts = synth.specFilters.points[osc];
      g.strokeStyle = getComputedStyle(c).getPropertyValue('--wave').trim() || '#fff';
      g.lineWidth = 1.5 * dpr;
      g.beginPath();
      for (let i = 0; i < pts.length; i++) {
        const y = h - (i / (pts.length - 1)) * h;
        const x = pts[i] * w * 0.25;
        if (i) g.lineTo(x, y);
        else g.moveTo(x, y);
      }
      g.stroke();
    }
    const p = synth.host?.tel[TEL.oscPlay + osc] ?? -1;
    if (p >= 0) {
      g.fillStyle = '#fff';
      g.fillRect(p * w, 0, dpr, h);
    }
  }
</script>

<div class="view">
  <canvas bind:this={canvas} style:--wave={color} aria-label="Spectrogram of what the Spectral oscillator plays"></canvas>
  {#if !has}<div class="empty">Drop audio or a picture here, or choose Load audio… / Load picture…</div>{/if}
</div>

<style>
  .view {
    position: relative;
    height: 100%;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    overflow: hidden;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 0 12px;
    text-align: center;
    font-size: 10px;
    color: var(--text-faint);
    pointer-events: none;
  }
</style>
