<!--
  One automation lane of a clip: a parameter's value over the clip's time.
  Click to add a point, drag to move it, Alt-click to remove it. Between
  points the value runs straight.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { TEL } from '../../gen/protocol';
  import type { Synth } from '../../synth';
  import { fitCanvas, onFrame } from '../frame';

  let { slot, lane, color = 'var(--ctl)' }: { slot: number; lane: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  let canvas = $state<HTMLCanvasElement>();
  let version = $state(0);
  onMount(() => {
    const off = synth.clips.subscribe((s) => s === slot && version++);
    let pos = -1;
    const offF = onFrame(() => {
      const t = synth.host?.tel;
      const p = t && t[TEL.clipPlaying] === slot ? t[TEL.clipPos] : -1;
      if (p !== pos) {
        pos = p;
        draw();
      }
    });
    return () => {
      off();
      offF();
    };
  });
  $effect(() => {
    void version;
    void slot;
    void lane;
    draw();
  });
  const data = () => synth.clips.clips[slot].lanes[lane];

  function draw(): void {
    const c = canvas;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    g.clearRect(0, 0, w, h);
    const len = synth.clips.clips[slot].length;
    const l = data();
    const col = getComputedStyle(c).getPropertyValue('--c').trim() || '#7fd1e8';
    const X = (b: number) => (b / len) * w;
    const Y = (v: number) => h - v * (h - 4 * dpr) - 2 * dpr;
    if (!l.param) return;
    const pts = [...l.points].sort((a, b) => a[0] - b[0]);
    g.strokeStyle = col;
    g.lineWidth = 1.5 * dpr;
    g.beginPath();
    if (pts.length) {
      g.moveTo(0, Y(pts[0][1]));
      for (const [b, v] of pts) g.lineTo(X(b), Y(v));
      g.lineTo(w, Y(pts[pts.length - 1][1]));
    }
    g.stroke();
    g.fillStyle = col;
    for (const [b, v] of pts) {
      g.beginPath();
      g.arc(X(b), Y(v), 3 * dpr, 0, Math.PI * 2);
      g.fill();
    }
    const t = synth.host?.tel;
    if (t && t[TEL.clipPlaying] === slot) {
      g.fillStyle = '#fff';
      g.fillRect(X(t[TEL.clipPos]), 0, dpr, h);
    }
  }

  const at = (e: PointerEvent) => {
    const r = canvas!.getBoundingClientRect();
    const len = synth.clips.clips[slot].length;
    return [Math.min(len, Math.max(0, ((e.clientX - r.left) / r.width) * len)), Math.min(1, Math.max(0, 1 - (e.clientY - r.top) / r.height))] as [number, number];
  };
  let dragging = -1;
  function near(p: [number, number]): number {
    const r = canvas!.getBoundingClientRect();
    const len = synth.clips.clips[slot].length;
    return data().points.findIndex(([b, v]) => Math.abs(((b - p[0]) / len) * r.width) < 6 && Math.abs((v - p[1]) * r.height) < 8);
  }
  function onpointerdown(e: PointerEvent): void {
    if (!data().param || e.button !== 0) return;
    const p = at(e);
    const i = near(p);
    if (i >= 0 && e.altKey) {
      synth.clips.edit(slot, (c) => c.lanes[lane].points.splice(i, 1));
      return;
    }
    canvas!.setPointerCapture(e.pointerId);
    if (i >= 0) dragging = i;
    else {
      synth.clips.edit(slot, (c) => c.lanes[lane].points.push(p));
      dragging = data().points.length - 1;
    }
  }
  function onpointermove(e: PointerEvent): void {
    if (dragging < 0 || !canvas!.hasPointerCapture(e.pointerId)) return;
    const p = at(e);
    const i = dragging;
    synth.clips.edit(slot, (c) => (c.lanes[lane].points[i] = p));
  }
</script>

<canvas bind:this={canvas} class="lane" style:--c={color} aria-label={`Automation lane ${lane + 1}`} {onpointerdown} {onpointermove} onpointerup={() => (dragging = -1)}></canvas>

<style>
  .lane {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
    touch-action: none;
  }
</style>
