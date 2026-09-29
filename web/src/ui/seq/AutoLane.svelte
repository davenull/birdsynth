<!--
  One automation lane of a clip: a parameter's value over the clip's time.
  Click to add a point, drag to move it, Alt-click to remove it. Between
  points the value runs straight. From the keyboard: Left/Right move along
  the clip a sixteenth at a time, Up/Down set the value there (adding a
  point), Delete removes the point there.
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
  /** The keyboard's place along the clip, in beats (drawn while focused). */
  let cur = $state(0);
  let focused = $state(false);
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
    void cur;
    void focused;
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
    if (focused) {
      g.fillStyle = 'rgba(255,255,255,0.5)';
      g.fillRect(X(cur), 0, dpr, h);
      g.strokeStyle = '#fff';
      g.lineWidth = 1.5 * dpr;
      g.beginPath();
      g.arc(X(cur), Y(valueAt(cur)), 5 * dpr, 0, Math.PI * 2);
      g.stroke();
    }
  }

  const nowValue = () => {
    void version;
    return Math.round(valueAt(cur) * 100);
  };

  /** The lane's value at a beat (straight between points, flat outside them). */
  function valueAt(b: number): number {
    const pts = [...data().points].sort((x, y) => x[0] - y[0]);
    if (!pts.length) return 0.5;
    if (b <= pts[0][0]) return pts[0][1];
    for (let i = 1; i < pts.length; i++) if (b <= pts[i][0]) return pts[i - 1][1] + ((pts[i][1] - pts[i - 1][1]) * (b - pts[i - 1][0])) / Math.max(1e-9, pts[i][0] - pts[i - 1][0]);
    return pts[pts.length - 1][1];
  }

  function onkeydown(e: KeyboardEvent): void {
    if (!data().param) return;
    const len = synth.clips.clips[slot].length;
    const i = data().points.findIndex((p) => Math.abs(p[0] - cur) < 1e-6);
    const put = (v: number) =>
      synth.clips.edit(slot, (c) => {
        const pts = c.lanes[lane].points;
        const p: [number, number] = [cur, Math.min(1, Math.max(0, v))];
        if (i >= 0) pts[i] = p;
        else pts.push(p);
      });
    switch (e.key) {
      case 'ArrowLeft':
        cur = Math.max(0, cur - 0.25);
        break;
      case 'ArrowRight':
        cur = Math.min(len, cur + 0.25);
        break;
      case 'ArrowUp':
      case 'ArrowDown':
        put(valueAt(cur) + (e.key === 'ArrowUp' ? 1 : -1) * (e.shiftKey ? 0.01 : 0.05));
        break;
      case 'Delete':
      case 'Backspace':
        if (i >= 0) synth.clips.edit(slot, (c) => c.lanes[lane].points.splice(i, 1));
        break;
      default:
        return;
    }
    e.preventDefault();
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

<canvas
  bind:this={canvas}
  class="lane"
  style:--c={color}
  tabindex="0"
  role="slider"
  aria-label={`Automation lane ${lane + 1}${data().param ? '' : ' (pick a parameter first)'}, beat ${cur + 1}`}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={nowValue()}
  {onkeydown}
  onfocus={() => (focused = true)}
  onblur={() => (focused = false)}
  {onpointerdown}
  {onpointermove}
  onpointerup={() => (dragging = -1)}
></canvas>

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
