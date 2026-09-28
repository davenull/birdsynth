<!--
  Every frame as a thumbnail. Click to edit a frame; Shift-click selects a
  range, Cmd/Ctrl-click adds or removes one; drag a thumbnail to move it.
  The arrow keys step through the frames.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import { fitCanvas } from '../ui/frame';
  import type { WtEditor } from './model';

  let { ed, color = 'var(--osc-a)' }: { ed: WtEditor; color?: string } = $props();

  const TW = 46; // thumbnail width, CSS px
  const FL = 2048;
  let canvas = $state<HTMLCanvasElement>();
  let scroller = $state<HTMLElement>();
  let version = $state(0);
  let drop = $state<number | null>(null);
  onMount(() => ed.subscribe(() => (version = ed.version)));
  const count = $derived.by(() => {
    void version;
    return ed.count;
  });
  const current = $derived.by(() => {
    void version;
    return ed.current;
  });

  function paint(): void {
    const c = canvas;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    g.clearRect(0, 0, w, h);
    const tw = TW * dpr;
    const wave = getComputedStyle(c).getPropertyValue('--wave').trim() || '#4ea3ff';
    for (let f = 0; f < ed.count; f++) {
      const x0 = f * tw;
      const sel = ed.selected.has(f);
      g.fillStyle = f === ed.current ? 'rgba(78,163,255,0.22)' : sel ? 'rgba(78,163,255,0.1)' : 'rgba(255,255,255,0.03)';
      g.fillRect(x0 + dpr, 0, tw - 2 * dpr, h - 12 * dpr);
      if (sel) {
        g.strokeStyle = wave;
        g.lineWidth = dpr;
        g.strokeRect(x0 + 1.5 * dpr, 0.5 * dpr, tw - 3 * dpr, h - 13 * dpr);
      }
      const fr = ed.frame(f);
      const mid = (h - 12 * dpr) / 2;
      const amp = mid - 3 * dpr;
      g.strokeStyle = wave;
      g.globalAlpha = sel || f === ed.current ? 1 : 0.6;
      g.lineWidth = dpr;
      g.beginPath();
      const n = Math.floor(tw - 6 * dpr);
      for (let i = 0; i <= n; i++) {
        const v = fr[Math.min(FL - 1, Math.floor((i / n) * FL))];
        const x = x0 + 3 * dpr + i;
        const y = mid - v * amp;
        if (i) g.lineTo(x, y);
        else g.moveTo(x, y);
      }
      g.stroke();
      g.globalAlpha = 1;
      g.fillStyle = 'rgba(255,255,255,0.4)';
      g.font = `${8.5 * dpr}px JetBrains Mono, monospace`;
      g.fillText(String(f + 1), x0 + 3 * dpr, h - 2 * dpr);
    }
    if (drop !== null) {
      g.fillStyle = wave;
      g.fillRect(drop * tw - dpr, 0, 2 * dpr, h);
    }
  }

  $effect(() => {
    void version;
    void drop;
    paint();
  });

  // keep the current frame in view
  $effect(() => {
    void version;
    const s = scroller;
    if (!s) return;
    const x = ed.current * TW;
    if (x < s.scrollLeft) s.scrollLeft = x;
    else if (x + TW > s.scrollLeft + s.clientWidth) s.scrollLeft = x + TW - s.clientWidth;
  });

  let from: number | null = null;
  let downX = 0;
  let moved = false;

  const index = (e: PointerEvent, round = false) => {
    const r = canvas!.getBoundingClientRect();
    const x = ((e.clientX - r.left) / r.width) * ed.count;
    return Math.max(0, Math.min(ed.count - (round ? 0 : 1), round ? Math.round(x) : Math.floor(x)));
  };

  function onpointerdown(e: PointerEvent): void {
    if (e.button !== 0) return;
    const i = index(e);
    if (e.shiftKey) ed.select(i, 'range');
    else if (e.metaKey || e.ctrlKey) ed.select(i, 'toggle');
    else {
      from = i;
      downX = e.clientX;
      moved = false;
      canvas!.setPointerCapture(e.pointerId);
      if (!ed.selected.has(i) || ed.selected.size === 1) ed.select(i);
      else ed.focus(i);
    }
  }

  function onpointermove(e: PointerEvent): void {
    if (from === null || !canvas?.hasPointerCapture(e.pointerId)) return;
    if (Math.abs(e.clientX - downX) > 6) moved = true;
    if (moved) drop = index(e, true);
  }

  function onpointerup(e: PointerEvent): void {
    if (from !== null && moved && drop !== null) {
      const to = drop > from ? drop - 1 : drop;
      ed.moveFrame(from, to);
    } else if (from !== null && !moved) {
      ed.select(index(e));
    }
    from = null;
    drop = null;
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    e.stopPropagation();
    const i = ed.current + (e.key === 'ArrowRight' ? 1 : -1);
    ed.select(i, e.shiftKey ? 'range' : 'only');
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="strip" bind:this={scroller} role="listbox" aria-label="Frames" aria-multiselectable="true" tabindex="0" {onkeydown}>
  <canvas
    bind:this={canvas}
    style:width={`${count * TW}px`}
    style:--wave={color}
    aria-label={`${count} frames; frame ${current + 1} is being edited`}
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={() => {
      from = null;
      drop = null;
    }}
  ></canvas>
</div>

<style>
  .strip {
    min-width: 0;
    flex: none;
    overflow-x: auto;
    overflow-y: hidden;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    outline: none;
  }
  .strip:focus-visible {
    border-color: var(--accent);
  }
  canvas {
    display: block;
    height: 52px;
    touch-action: none;
  }
</style>
