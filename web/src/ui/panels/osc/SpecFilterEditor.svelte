<!-- The Spectral type's filter: draw a gain at each of 64 frequencies (20 Hz at the left, 20 kHz at the right). -->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { Synth } from '../../../synth';
  import { FILTER_POINTS } from '../../../state/spectral';
  import { fitCanvas } from '../../frame';

  let { osc, color = 'var(--osc-a)' }: { osc: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  let canvas = $state<HTMLCanvasElement>();
  let pts = new Float32Array(FILTER_POINTS);

  function draw(): void {
    const c = canvas;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    g.clearRect(0, 0, w, h);
    const bw = w / FILTER_POINTS;
    g.fillStyle = getComputedStyle(c).getPropertyValue('--c').trim() || '#4ea3ff';
    for (let i = 0; i < FILTER_POINTS; i++) g.fillRect(i * bw + dpr * 0.5, h * (1 - pts[i]), Math.max(dpr, bw - dpr), h * pts[i]);
  }
  onMount(() => {
    pts = synth.specFilters.points[osc].slice();
    draw();
    return synth.specFilters.subscribe((o) => {
      if (o === osc) {
        pts = synth.specFilters.points[osc].slice();
        draw();
      }
    });
  });
  let last: [number, number] | null = null;
  function paint(e: PointerEvent): void {
    const r = canvas!.getBoundingClientRect();
    const i = Math.min(FILTER_POINTS - 1, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * FILTER_POINTS)));
    const v = Math.min(1, Math.max(0, 1 - (e.clientY - r.top) / r.height));
    const [a, b] = last ? [last, [i, v] as [number, number]] : [[i, v] as [number, number], [i, v] as [number, number]];
    const [lo, hi] = a[0] <= b[0] ? [a, b] : [b, a];
    for (let k = lo[0]; k <= hi[0]; k++) pts[k] = hi[0] > lo[0] ? lo[1] + ((hi[1] - lo[1]) * (k - lo[0])) / (hi[0] - lo[0]) : v;
    last = [i, v];
    synth.specFilters.set(osc, pts);
  }
</script>

<div class="editor">
  <canvas
    bind:this={canvas}
    style:--c={color}
    aria-label="Spectral filter: draw the gain across frequency"
    onpointerdown={(e) => {
      canvas!.setPointerCapture(e.pointerId);
      last = null;
      paint(e);
    }}
    onpointermove={(e) => canvas!.hasPointerCapture(e.pointerId) && paint(e)}
    onpointerup={() => (last = null)}
  ></canvas>
  <button onclick={() => synth.specFilters.reset(osc)} title="Every frequency at full gain">Flat</button>
</div>

<style>
  .editor {
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
    cursor: crosshair;
    touch-action: none;
  }
  button {
    position: absolute;
    right: 3px;
    top: 3px;
    font: 9px var(--font-ui);
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 3px;
    padding: 1px 5px;
    cursor: pointer;
  }
</style>
