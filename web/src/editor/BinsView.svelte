<!--
  The current frame as harmonics: amplitude bars above, phase bars below.
  Drag across the bars to set them; the frame is rebuilt from the
  harmonics as you drag, so you hear it. Amplitudes show on a dB scale
  (-60 to 0) so quiet upper harmonics are visible.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { fitCanvas } from '../ui/frame';
  import type { WtEditor } from './model';

  let { ed, shown = 64, color = 'var(--osc-a)' }: { ed: WtEditor; shown?: number; color?: string } = $props();

  let magCanvas = $state<HTMLCanvasElement>();
  let phCanvas = $state<HTMLCanvasElement>();
  let mag = $state.raw<Float32Array>(new Float32Array(1025));
  let phase = $state.raw<Float32Array>(new Float32Array(1025));
  let version = $state(0);
  let dragging = false;

  onMount(() => ed.subscribe(() => (version = ed.version)));

  // re-analyse when the frame changes (but not while this view is the one changing it)
  let asked = 0;
  $effect(() => {
    void version;
    if (dragging) return;
    const n = ++asked;
    void ed.analyze().then((r) => {
      if (n !== asked || dragging) return;
      mag = r.mag;
      phase = r.phase;
    });
  });

  const DB = 60;
  const toY = (m: number) => Math.max(0, Math.min(1, 1 + (20 * Math.log10(Math.max(m, 1e-9))) / DB));
  const fromY = (t: number) => (t <= 0.001 ? 0 : 10 ** (((t - 1) * DB) / 20));

  function paint(): void {
    for (const [c, kind] of [
      [magCanvas, 'mag'],
      [phCanvas, 'phase'],
    ] as const) {
      if (!c) continue;
      const g = c.getContext('2d');
      if (!g) continue;
      const { w, h, dpr } = fitCanvas(c);
      g.clearRect(0, 0, w, h);
      const bw = w / shown;
      g.fillStyle = getComputedStyle(c).getPropertyValue('--bar').trim() || '#4ea3ff';
      for (let k = 1; k <= shown; k++) {
        const x = (k - 1) * bw;
        if (kind === 'mag') {
          const t = toY(mag[k]);
          g.fillRect(x + dpr, h * (1 - t), Math.max(dpr, bw - 2 * dpr), h * t);
        } else {
          const t = phase[k] / Math.PI; // -1..1
          const y0 = h / 2;
          const y1 = y0 - t * (h / 2 - dpr);
          g.globalAlpha = mag[k] > 1e-4 ? 1 : 0.25;
          g.fillRect(x + dpr, Math.min(y0, y1), Math.max(dpr, bw - 2 * dpr), Math.max(dpr, Math.abs(y1 - y0)));
          g.globalAlpha = 1;
        }
      }
      g.fillStyle = 'rgba(255,255,255,0.35)';
      g.font = `${9 * dpr}px JetBrains Mono, monospace`;
      for (let k = 1; k <= shown; k *= 2) g.fillText(String(k), (k - 1) * bw + 2 * dpr, kind === 'mag' ? 10 * dpr : h - 3 * dpr);
      if (kind === 'phase') {
        g.fillStyle = 'rgba(255,255,255,0.15)';
        g.fillRect(0, h / 2, w, dpr);
      }
    }
  }

  $effect(() => {
    void mag;
    void phase;
    void shown;
    paint();
  });

  let last: { k: number; t: number } | null = null;
  let target: 'mag' | 'phase' = 'mag';
  let pending = false;

  function at(e: PointerEvent, c: HTMLCanvasElement): { k: number; t: number } {
    const r = c.getBoundingClientRect();
    const k = Math.min(shown, Math.max(1, Math.floor(((e.clientX - r.left) / r.width) * shown) + 1));
    const t = Math.min(1, Math.max(0, 1 - (e.clientY - r.top) / r.height));
    return { k, t };
  }

  function set(p: { k: number; t: number }): void {
    const [a, b] = last && last.k !== p.k ? [last, p] : [p, p];
    const [lo, hi] = a.k <= b.k ? [a, b] : [b, a];
    for (let k = lo.k; k <= hi.k; k++) {
      const u = hi.k > lo.k ? (k - lo.k) / (hi.k - lo.k) : 1;
      const t = lo.t + (hi.t - lo.t) * u;
      if (target === 'mag') mag[k] = fromY(t);
      else phase[k] = (t * 2 - 1) * Math.PI;
    }
    last = p;
    paint();
    rebuild();
  }

  /** Rebuild the frame from the bars, one request at a time. */
  function rebuild(): void {
    if (pending) return;
    pending = true;
    void (async () => {
      const f = await ed.synthesizeFrame(mag.slice(), phase.slice());
      pending = false;
      ed.writeFrame(f);
    })();
  }

  function down(e: PointerEvent, kind: 'mag' | 'phase'): void {
    if (e.button !== 0) return;
    const c = e.currentTarget as HTMLCanvasElement;
    c.setPointerCapture(e.pointerId);
    target = kind;
    dragging = true;
    ed.beginStroke();
    last = null;
    set(at(e, c));
    e.preventDefault();
  }

  function move(e: PointerEvent): void {
    const c = e.currentTarget as HTMLCanvasElement;
    if (!c.hasPointerCapture(e.pointerId)) return;
    set(at(e, c));
  }

  async function up(): Promise<void> {
    if (!dragging) return;
    last = null;
    const f = await ed.synthesizeFrame(mag.slice(), phase.slice());
    ed.writeFrame(f);
    ed.endStroke();
    dragging = false;
  }
</script>

<div class="bins" style:--bar={color}>
  <div class="cv"><canvas bind:this={magCanvas} class="mag" aria-label="Harmonic amplitudes: drag to set" onpointerdown={(e) => down(e, 'mag')} onpointermove={move} onpointerup={up} onpointercancel={up}></canvas></div>
  <div class="cv"><canvas bind:this={phCanvas} class="phase" aria-label="Harmonic phases: drag to set" onpointerdown={(e) => down(e, 'phase')} onpointermove={move} onpointerup={up} onpointercancel={up}></canvas></div>
</div>

<style>
  .bins {
    display: grid;
    grid-template-rows: 2fr 1fr;
    gap: 4px;
    height: 100%;
    min-height: 0;
  }
  .cv {
    position: relative;
    min-height: 0;
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
    width: 100%;
    height: 100%;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    cursor: ns-resize;
    touch-action: none;
  }
</style>
