<!--
  A row of bars to draw values on (step lanes, Voice Control): drag across
  to set them. Bipolar rows grow from the middle. The highlighted column is
  the step playing now.
-->
<script lang="ts">
  import { fitCanvas } from '../frame';

  let {
    values,
    min = 0,
    max = 1,
    bipolar = false,
    active = -1,
    count = values.length,
    height = 44,
    color = 'var(--accent)',
    label,
    format = (v: number) => v.toFixed(2),
    onchange,
  }: {
    values: ArrayLike<number>;
    min?: number;
    max?: number;
    bipolar?: boolean;
    active?: number;
    count?: number;
    height?: number;
    color?: string;
    label: string;
    format?: (v: number) => string;
    onchange: (i: number, v: number) => void;
  } = $props();

  let canvas = $state<HTMLCanvasElement>();

  $effect(() => {
    void values;
    void active;
    void count;
    const c = canvas;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    g.clearRect(0, 0, w, h);
    const n = values.length;
    const bw = w / n;
    const col = getComputedStyle(c).getPropertyValue('--c').trim() || '#4ea3ff';
    const y = (v: number) => h - ((v - min) / (max - min)) * h;
    const zero = bipolar ? y(0) : h;
    for (let i = 0; i < n; i++) {
      const x = i * bw;
      if (i === active) {
        g.fillStyle = 'rgba(255,255,255,0.08)';
        g.fillRect(x, 0, bw, h);
      }
      g.globalAlpha = i < count ? 1 : 0.25;
      g.fillStyle = col;
      const v = values[i];
      const top = Math.min(zero, y(v));
      g.fillRect(x + 1.5 * dpr, top, Math.max(dpr, bw - 3 * dpr), Math.max(dpr, Math.abs(y(v) - zero)));
      g.globalAlpha = 1;
    }
    if (bipolar) {
      g.fillStyle = 'rgba(255,255,255,0.2)';
      g.fillRect(0, zero, w, dpr);
    }
  });

  let last: [number, number] | null = null;
  function set(e: PointerEvent): void {
    const r = canvas!.getBoundingClientRect();
    const n = values.length;
    const i = Math.min(n - 1, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * n)));
    const v = min + (1 - Math.min(1, Math.max(0, (e.clientY - r.top) / r.height))) * (max - min);
    if (last && last[0] !== i) {
      const [a, av] = last;
      const step = i > a ? 1 : -1;
      for (let k = a + step; k !== i; k += step) onchange(k, av + ((v - av) * (k - a)) / (i - a));
    }
    onchange(i, v);
    last = [i, v];
  }
</script>

<div class="bars">
  <span class="label">{label}</span>
  <canvas
    bind:this={canvas}
    style:height={`${height}px`}
    style:--c={color}
    aria-label={`${label}: ${Array.from(values, format).join(', ')}`}
    onpointerdown={(e) => {
      canvas!.setPointerCapture(e.pointerId);
      last = null;
      set(e);
    }}
    onpointermove={(e) => canvas!.hasPointerCapture(e.pointerId) && set(e)}
    onpointerup={() => (last = null)}
  ></canvas>
</div>

<style>
  .bars {
    display: grid;
    grid-template-columns: 62px minmax(0, 1fr);
    align-items: center;
    gap: 6px;
  }
  .label {
    font: 600 9.5px var(--font-ui);
    letter-spacing: 0.05em;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  canvas {
    display: block;
    width: 100%;
    min-width: 0;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 3px;
    cursor: ns-resize;
    touch-action: none;
  }
</style>
