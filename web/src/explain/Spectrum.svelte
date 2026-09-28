<!--
  Live spectrum of a tap on a log frequency axis, 20 Hz to 20 kHz, with
  marks at the harmonics of the note the focused voice plays: whatever
  shows between the marks isn't part of the note (aliasing, noise, or
  the effects' own sound).
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { TAP, TEL, type TapName } from '../gen/protocol';
  import type { Synth } from '../synth';
  import { fitCanvas, onFrame } from '../ui/frame';
  import { spectrumDb } from './fft';

  let { synth, tap = 'master.l', height = 150, harmonics = true }: { synth: Synth; tap?: TapName; height?: number; harmonics?: boolean } = $props();

  let canvas = $state<HTMLCanvasElement>();
  let live = $state(false);
  $effect(() => synth.useTap(tap));

  const N = 4096;
  const F0 = 20;
  const F1 = 20_000;
  const DB0 = -110;

  onMount(() => {
    const buf = new Float32Array(N);
    const db = new Float32Array(N / 2 + 1);
    const smooth = new Float32Array(N / 2 + 1).fill(DB0);
    return onFrame(() => {
      const c = canvas;
      const host = synth.host;
      if (!c || !host) return;
      const g = c.getContext('2d');
      if (!g) return;
      const { w, h, dpr } = fitCanvas(c);
      const sr = host.ctx.sampleRate;
      const x = (f: number) => (Math.log(f / F0) / Math.log(F1 / F0)) * w;
      const y = (d: number) => (Math.max(DB0, Math.min(0, d)) / DB0) * h;
      g.clearRect(0, 0, w, h);
      // grid: decades and every 20 dB
      g.strokeStyle = 'rgba(255,255,255,0.06)';
      g.fillStyle = 'rgba(255,255,255,0.35)';
      g.font = `${9 * dpr}px JetBrains Mono, monospace`;
      g.lineWidth = dpr;
      for (const f of [100, 1000, 10_000]) {
        g.beginPath();
        g.moveTo(x(f), 0);
        g.lineTo(x(f), h);
        g.stroke();
        g.fillText(f >= 1000 ? `${f / 1000}k` : `${f}`, x(f) + 3 * dpr, h - 3 * dpr);
      }
      for (let d = -20; d > DB0; d -= 20) {
        g.beginPath();
        g.moveTo(0, y(d));
        g.lineTo(w, y(d));
        g.stroke();
        g.fillText(`${d}`, 2 * dpr, y(d) - 2 * dpr);
      }
      // the Nyquist limit
      const ny = x(sr / 2);
      if (ny < w) {
        g.fillStyle = 'rgba(255,82,82,0.08)';
        g.fillRect(ny, 0, w - ny, h);
      }
      const t = host.tel;
      const note = t[TEL.focusPitch];
      const sounding = t[TEL.voicesActive] > 0;
      if (harmonics && sounding && note > 0) {
        const f0 = 440 * 2 ** ((note - 69) / 12);
        g.strokeStyle = 'rgba(255,209,102,0.35)';
        for (let k = 1; k * f0 < Math.min(F1, sr / 2); k++) {
          const hx = x(k * f0);
          g.beginPath();
          g.moveTo(hx, 0);
          g.lineTo(hx, 6 * dpr);
          g.stroke();
        }
      }
      const idx = TAP[tap];
      const heard = Math.min(host.heardFrame(), host.taps.latest(idx));
      live = host.taps.read(idx, heard, buf);
      if (!live) return;
      spectrumDb(buf, db);
      // fast attack, slow fall, so peaks are readable
      for (let k = 0; k < db.length; k++) smooth[k] = db[k] > smooth[k] ? db[k] : smooth[k] + (db[k] - smooth[k]) * 0.25;
      g.strokeStyle = getComputedStyle(c).getPropertyValue('--spectrum') || '#5fd3ff';
      g.lineWidth = 1.2 * dpr;
      g.beginPath();
      // one point per pixel column: the loudest bin in it
      const binHz = sr / N;
      let started = false;
      for (let px = 0; px < w; px++) {
        const fa = F0 * (F1 / F0) ** (px / w);
        const fb = F0 * (F1 / F0) ** ((px + 1) / w);
        let ka = Math.floor(fa / binHz);
        const kb = Math.min(db.length - 1, Math.max(ka, Math.floor(fb / binHz)));
        if (ka >= db.length) break;
        let m = DB0;
        for (; ka <= kb; ka++) m = Math.max(m, smooth[ka]);
        if (!started) g.moveTo(px, y(m));
        else g.lineTo(px, y(m));
        started = true;
      }
      g.stroke();
    });
  });
</script>

<div class="spectrum">
  <canvas bind:this={canvas} style:height={`${height}px`} aria-label="Spectrum of the output: frequency across, level up"></canvas>
  {#if !live}<div class="idle">Play a note to see its spectrum</div>{/if}
</div>

<style>
  .spectrum {
    position: relative;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    --spectrum: var(--fx);
  }
  canvas {
    display: block;
    width: 100%;
  }
  .idle {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: 11px;
    color: var(--text-faint);
    pointer-events: none;
  }
</style>
