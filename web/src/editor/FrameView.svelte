<!--
  The current frame, large, to draw on. Pen draws freehand (straight
  segments between pointer samples, so a fast stroke leaves no gaps); Line
  draws from where you press to where you let go. With the grid's snap on,
  points land on its columns and rows, for stepped and quantised shapes.
  The previous frame shows faintly behind.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { fitCanvas } from '../ui/frame';
  import type { WtEditor } from './model';

  let {
    ed,
    tool = 'pen',
    gridX = 8,
    gridY = 4,
    snap = false,
    color = 'var(--osc-a)',
  }: { ed: WtEditor; tool?: 'pen' | 'line'; gridX?: number; gridY?: number; snap?: boolean; color?: string } = $props();

  const FL = 2048;
  let canvas = $state<HTMLCanvasElement>();
  let version = $state(0);
  onMount(() => ed.subscribe(() => (version = ed.version)));
  const label = $derived.by(() => {
    void version;
    return `Frame ${ed.current + 1} of ${ed.count}: draw to change it`;
  });

  function draw(): void {
    const c = canvas;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    const css = getComputedStyle(c);
    g.clearRect(0, 0, w, h);
    const y = (v: number) => h / 2 - v * (h / 2 - 6 * dpr);
    // grid
    g.lineWidth = dpr;
    g.strokeStyle = 'rgba(255,255,255,0.06)';
    for (let i = 1; i < gridX; i++) {
      const x = Math.round((i / gridX) * w) + 0.5;
      g.beginPath();
      g.moveTo(x, 0);
      g.lineTo(x, h);
      g.stroke();
    }
    for (let j = 1; j < gridY * 2; j++) {
      const v = 1 - j / gridY;
      g.beginPath();
      g.moveTo(0, y(v));
      g.lineTo(w, y(v));
      g.stroke();
    }
    g.strokeStyle = 'rgba(255,255,255,0.18)';
    g.beginPath();
    g.moveTo(0, y(0));
    g.lineTo(w, y(0));
    g.stroke();
    const line = (f: Float32Array, stroke: string, width: number) => {
      g.strokeStyle = stroke;
      g.lineWidth = width * dpr;
      g.beginPath();
      const step = FL / w;
      for (let px = 0; px < w; px++) {
        // min and max in the column, so single-sample spikes still show
        const a = Math.floor(px * step);
        const b = Math.max(a + 1, Math.floor((px + 1) * step));
        let lo = Infinity;
        let hi = -Infinity;
        for (let i = a; i < b && i < FL; i++) {
          lo = Math.min(lo, f[i]);
          hi = Math.max(hi, f[i]);
        }
        if (px === 0) g.moveTo(px, y(f[0]));
        g.lineTo(px, y(hi));
        if (lo !== hi) g.lineTo(px, y(lo));
      }
      g.stroke();
    };
    if (ed.current > 0) line(ed.frame(ed.current - 1), 'rgba(255,255,255,0.12)', 1);
    line(ed.frame(), css.getPropertyValue('--wave').trim() || '#4ea3ff', 2);
  }

  $effect(() => {
    void version;
    void gridX;
    void gridY;
    draw();
  });

  // --- pointer drawing
  let last: [number, number] | null = null;
  let start: [number, number] | null = null;
  let before: Float32Array | null = null;

  function point(e: PointerEvent): [number, number] {
    const r = canvas!.getBoundingClientRect();
    let fx = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    let v = Math.min(1, Math.max(-1, 1 - (2 * (e.clientY - r.top)) / r.height));
    if (snap) {
      if (gridX > 0) fx = Math.floor(fx * gridX) / gridX;
      if (gridY > 0) v = Math.round(v * gridY) / gridY;
    }
    return [fx * (FL - 1), v];
  }

  function paint(a: [number, number], b: [number, number]): void {
    if (snap && gridX > 0) {
      // snapped: fill each grid column the pointer crossed with its level
      const col = FL / gridX;
      const [lo, hi] = a[0] <= b[0] ? [a, b] : [b, a];
      for (let c = Math.floor(lo[0] / col); c <= Math.floor(hi[0] / col); c++) {
        const t = hi[0] > lo[0] ? (c * col - lo[0]) / (hi[0] - lo[0]) : 1;
        const v = Math.round((lo[1] + (hi[1] - lo[1]) * Math.min(1, Math.max(0, t))) * gridY) / gridY;
        ed.segment(c * col, v, Math.min(FL - 1, (c + 1) * col - 1), v);
      }
    } else {
      ed.segment(a[0], a[1], b[0], b[1]);
    }
  }

  function onpointerdown(e: PointerEvent): void {
    if (e.button !== 0) return;
    canvas!.setPointerCapture(e.pointerId);
    ed.beginStroke();
    const p = point(e);
    start = p;
    last = p;
    before = ed.frame().slice();
    if (tool === 'pen') paint(p, p);
    e.preventDefault();
  }

  function onpointermove(e: PointerEvent): void {
    if (!canvas?.hasPointerCapture(e.pointerId) || !last) return;
    const p = point(e);
    if (tool === 'pen') {
      paint(last, p);
      last = p;
    } else if (start && before) {
      // a line: start again from the frame as it was, then draw start → here
      ed.writeFrame(before);
      paint(start, p);
    }
  }

  function onpointerup(): void {
    if (!last) return;
    last = null;
    start = null;
    before = null;
    ed.endStroke();
  }
</script>

<div class="cv">
<canvas
  bind:this={canvas}
  class="frame-view"
  style:--wave={color}
  aria-label={label}
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={onpointerup}
></canvas>
</div>

<style>
  /* the canvas sits absolutely in its cell: a canvas sized by its own
     pixels would otherwise grow the grid row it's measured from */
  .cv {
    position: relative;
    min-height: 0;
    min-width: 0;
  }
  .frame-view {
    position: absolute;
    inset: 0;
    display: block;
    width: 100%;
    height: 100%;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    cursor: crosshair;
    touch-action: none;
  }
</style>
