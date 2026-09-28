<!--
  An oscillator's recording, drawn small: the waveform, with the Sample
  type's start and end (outside them shaded), its loop (a bracket) and its
  slices, or for Granular the position and the spray around it, and a
  playhead for the newest voice. Drag a marker to move it.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../../gen/params';
  import { TEL } from '../../../gen/protocol';
  import type { Synth } from '../../../synth';
  import { fitCanvas, onFrame } from '../../frame';

  let { osc, color = 'var(--osc-a)', mode = 'sample' }: { osc: number; color?: string; mode?: 'sample' | 'granular' } = $props();

  const synth = getContext<Synth>('synth');
  const L = $derived('abc'[osc]);
  const key = (s: string) => `osc.${L}.${s}` as ParamKey;
  let canvas = $state<HTMLCanvasElement>();
  let version = $state(0);
  let name = $state('');
  let seconds = $state(0);

  // min/max per column of the mono waveform, rebuilt when the recording changes
  let peaks: Float32Array | null = null;
  let peaksFor: unknown = null;

  onMount(() => {
    const read = () => {
      const r = synth.recordings.osc[osc];
      name = r?.name ?? '';
      seconds = r ? r.channels[0].length / r.rate : 0;
      version++;
    };
    read();
    const offRec = synth.recordings.subscribe((o) => o === osc && read());
    const ids = ['smp_start', 'smp_end', 'loop_start', 'loop_end', 'loop_mode', 'slice', 'gr_position', 'gr_spray'].map((s) => PARAM_ID[key(s)]);
    const offs = ids.map((id) => synth.bank.subscribe(id, () => version++));
    let play = -2;
    const offFrame = onFrame(() => {
      const t = synth.host?.tel;
      const p = t ? t[TEL.oscPlay + osc] : -1;
      if (Math.abs(p - play) > 1e-4) {
        play = p;
        draw(p);
      }
    });
    return () => {
      offRec();
      offs.forEach((f) => f());
      offFrame();
    };
  });

  $effect(() => {
    void version;
    draw();
  });

  const plain = (s: string) => {
    const p = PARAMS[PARAM_ID[key(s)]];
    return p.min + (p.max - p.min) * synth.bank.get(p.id);
  };

  function draw(play = synth.host?.tel[TEL.oscPlay + osc] ?? -1): void {
    const c = canvas;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    g.clearRect(0, 0, w, h);
    const r = synth.recordings.osc[osc];
    if (!r) return;
    if (peaksFor !== r || !peaks || peaks.length !== w * 2) {
      peaksFor = r;
      peaks = new Float32Array(w * 2);
      const x = r.channels[0];
      const y = r.channels[1];
      const per = x.length / w;
      for (let px = 0; px < w; px++) {
        let lo = 0;
        let hi = 0;
        const a = Math.floor(px * per);
        const b = Math.max(a + 1, Math.floor((px + 1) * per));
        for (let i = a; i < b && i < x.length; i += Math.max(1, Math.floor((b - a) / 64))) {
          const v = y ? 0.5 * (x[i] + y[i]) : x[i];
          if (v < lo) lo = v;
          if (v > hi) hi = v;
        }
        peaks[px * 2] = lo;
        peaks[px * 2 + 1] = hi;
      }
    }
    const wave = getComputedStyle(c).getPropertyValue('--wave').trim() || '#4ea3ff';
    const mid = h / 2;
    g.fillStyle = wave;
    g.globalAlpha = 0.85;
    for (let px = 0; px < w; px++) {
      const lo = peaks[px * 2];
      const hi = peaks[px * 2 + 1];
      g.fillRect(px, mid - hi * mid * 0.95, 1, Math.max(dpr, (hi - lo) * mid * 0.95));
    }
    g.globalAlpha = 1;
    if (mode === 'sample') {
      const s = plain('smp_start');
      const e = plain('smp_end');
      g.fillStyle = 'rgba(0,0,0,0.55)';
      g.fillRect(0, 0, s * w, h);
      g.fillRect(e * w, 0, w - e * w, h);
      if (plain('slice') >= 0.5) {
        g.strokeStyle = 'rgba(255,209,102,0.7)';
        g.lineWidth = dpr;
        for (const f of r.slices) {
          const x = (f / r.channels[0].length) * w;
          g.beginPath();
          g.moveTo(x, 0);
          g.lineTo(x, h);
          g.stroke();
        }
      } else if (plain('loop_mode') >= 0.5) {
        const a = plain('loop_start') * w;
        const b = plain('loop_end') * w;
        g.strokeStyle = 'rgba(255,255,255,0.8)';
        g.lineWidth = 1.5 * dpr;
        g.beginPath();
        g.moveTo(a + 4 * dpr, 2 * dpr);
        g.lineTo(a, 2 * dpr);
        g.lineTo(a, h - 2 * dpr);
        g.lineTo(a + 4 * dpr, h - 2 * dpr);
        g.moveTo(b - 4 * dpr, 2 * dpr);
        g.lineTo(b, 2 * dpr);
        g.lineTo(b, h - 2 * dpr);
        g.lineTo(b - 4 * dpr, h - 2 * dpr);
        g.stroke();
      }
    } else {
      const p = plain('gr_position');
      const sp = plain('gr_spray');
      g.fillStyle = 'rgba(255,209,102,0.15)';
      g.fillRect((p - sp) * w, 0, 2 * sp * w, h);
      g.fillStyle = 'rgba(255,209,102,0.9)';
      g.fillRect(p * w - dpr, 0, 2 * dpr, h);
    }
    if (play >= 0) {
      g.fillStyle = '#fff';
      g.fillRect(play * w - dpr * 0.5, 0, dpr, h);
    }
  }

  // --- drag the nearest marker
  let dragging: string | null = null;
  function markers(): [string, number][] {
    if (mode === 'granular') return [['gr_position', plain('gr_position')]];
    const out: [string, number][] = [
      ['smp_start', plain('smp_start')],
      ['smp_end', plain('smp_end')],
    ];
    if (plain('loop_mode') >= 0.5 && plain('slice') < 0.5) out.push(['loop_start', plain('loop_start')], ['loop_end', plain('loop_end')]);
    return out;
  }
  function at(e: PointerEvent): number {
    const r = canvas!.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  }
  function set(which: string, v: number): void {
    const p = PARAMS[PARAM_ID[key(which)]];
    synth.bank.set(p.id, (v - p.min) / (p.max - p.min));
  }
  function onpointerdown(e: PointerEvent): void {
    if (e.button !== 0 || !synth.recordings.osc[osc]) return;
    const x = at(e);
    const near = markers().sort((a, b) => Math.abs(a[1] - x) - Math.abs(b[1] - x))[0];
    if (!near) return;
    dragging = near[0];
    canvas!.setPointerCapture(e.pointerId);
    set(dragging, x);
  }
  function onpointermove(e: PointerEvent): void {
    if (dragging && canvas!.hasPointerCapture(e.pointerId)) set(dragging, at(e));
  }
</script>

<div class="view">
  <canvas
    bind:this={canvas}
    style:--wave={color}
    aria-label={name ? `${name}, ${seconds.toFixed(2)} s: drag to move the ${mode === 'granular' ? 'grain position' : 'start, end and loop points'}` : 'No recording yet'}
    {onpointerdown}
    {onpointermove}
    onpointerup={() => (dragging = null)}
  ></canvas>
  {#if !name}
    <div class="empty">Drop an audio file here, or choose Load audio…</div>
  {:else}
    <div class="name">{name} · {seconds.toFixed(2)} s</div>
  {/if}
</div>

<style>
  .view {
    position: relative;
    height: 100%;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: ew-resize;
    touch-action: none;
  }
  .empty,
  .name {
    position: absolute;
    pointer-events: none;
    font-size: 10px;
    color: var(--text-faint);
  }
  .empty {
    inset: 0;
    display: grid;
    place-items: center;
    text-align: center;
    padding: 0 12px;
  }
  .name {
    left: 5px;
    top: 3px;
    color: var(--text-dim);
  }
</style>
