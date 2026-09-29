<!--
  A clip's notes: time across (the clip's length), keys up. Click an empty
  spot to add a note one grid step long; drag a note to move it, drag its
  right end to change its length; Alt-click (or double-click) removes it;
  scroll over a note to change its velocity, anywhere else to move up and
  down the keyboard. Notes snap to the grid (Shift: free). The playhead
  shows where the clip is playing. From the keyboard: the arrows move a
  cursor (Shift+Up/Down an octave), Enter adds or removes a note there,
  Shift+Left/Right shortens or lengthens it, Delete removes it.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { TEL } from '../../gen/protocol';
  import type { Synth } from '../../synth';
  import type { ClipNote } from '../../state/seq';
  import { fitCanvas, onFrame } from '../frame';

  let { slot, grid = 0.25, color = 'var(--macro)' }: { slot: number; grid?: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  let canvas = $state<HTMLCanvasElement>();
  let version = $state(0);
  // the keys in view: `rows` of them from `low`, fitted to a clip's notes when it's picked
  let low = $state(51);
  let rows = $state(19);
  /** The keyboard's cursor (beat, key), drawn while the roll has focus. */
  let cursor = $state({ beat: 0, key: 60 });
  let focused = $state(false);
  const high = $derived(low + rows - 1);
  const fit = (s: number) => {
    const keys = synth.clips.clips[s].notes.map((n) => n.key);
    const [lo, hi] = keys.length ? [Math.min(...keys), Math.max(...keys)] : [60, 60];
    rows = Math.max(19, Math.min(49, hi - lo + 7));
    low = Math.max(0, Math.min(128 - rows, Math.round((lo + hi) / 2 - (rows - 1) / 2)));
  };
  $effect(() => fit(slot));
  onMount(() => {
    const off = synth.clips.subscribe((s) => {
      if (s !== slot) return;
      // notes arriving in an empty view (an import, say): go to them
      const c = synth.clips.clips[s];
      if (c.notes.length && !c.notes.some((n) => n.key >= low && n.key <= high)) fit(s);
      version++;
    });
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
    void grid;
    void low;
    void high;
    void cursor;
    void focused;
    draw();
  });

  const clip = () => synth.clips.clips[slot];

  function draw(): void {
    const c = canvas;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    const cl = clip();
    g.clearRect(0, 0, w, h);
    const keys = high - low + 1;
    const kh = h / keys;
    const X = (b: number) => (b / cl.length) * w;
    const Y = (k: number) => h - (k - low + 1) * kh;
    // rows: black keys darker, C lighter
    for (let k = low; k <= high; k++) {
      const pc = k % 12;
      g.fillStyle = [1, 3, 6, 8, 10].includes(pc) ? 'rgba(0,0,0,0.25)' : pc === 0 ? 'rgba(255,255,255,0.05)' : 'rgba(255,255,255,0.02)';
      g.fillRect(0, Y(k), w, kh);
    }
    // grid: steps faint, beats stronger, bars strongest
    for (let b = 0; b <= cl.length + 1e-9; b += grid) {
      const bar = Math.abs(b / 4 - Math.round(b / 4)) < 1e-6;
      const beat = Math.abs(b - Math.round(b)) < 1e-6;
      g.fillStyle = bar ? 'rgba(255,255,255,0.22)' : beat ? 'rgba(255,255,255,0.1)' : 'rgba(255,255,255,0.04)';
      g.fillRect(Math.round(X(b)), 0, dpr, h);
    }
    const col = getComputedStyle(c).getPropertyValue('--c').trim() || '#ffd166';
    for (const n of cl.notes) {
      if (n.key < low || n.key > high) continue;
      g.globalAlpha = 0.35 + 0.65 * n.velocity;
      g.fillStyle = col;
      g.fillRect(X(n.start) + dpr, Y(n.key) + dpr, Math.max(2 * dpr, X(n.length) - 2 * dpr), kh - 2 * dpr);
      g.globalAlpha = 1;
    }
    const t = synth.host?.tel;
    if (t && t[TEL.clipPlaying] === slot) {
      g.fillStyle = '#fff';
      g.fillRect(X(t[TEL.clipPos]), 0, dpr, h);
    }
    if (focused && cursor.key >= low && cursor.key <= high) {
      g.strokeStyle = '#fff';
      g.lineWidth = 1.5 * dpr;
      g.strokeRect(X(cursor.beat), Y(cursor.key), Math.max(2 * dpr, X(grid)), kh);
    }
    g.fillStyle = 'rgba(255,255,255,0.35)';
    g.font = `${9 * dpr}px JetBrains Mono, monospace`;
    for (let k = low; k <= high; k++) if (k % 12 === 0) g.fillText(`C${k / 12 - 1}`, 2 * dpr, Y(k) + kh - 2 * dpr);
  }

  function at(e: PointerEvent | WheelEvent | MouseEvent): { beat: number; key: number } {
    const r = canvas!.getBoundingClientRect();
    const cl = clip();
    const row = Math.floor(((e.clientY - r.top) / r.height) * (high - low + 1));
    return { beat: ((e.clientX - r.left) / r.width) * cl.length, key: Math.max(low, Math.min(high, high - row)) };
  }
  function hit(p: { beat: number; key: number }): number {
    return clip().notes.findIndex((n) => n.key === p.key && p.beat >= n.start && p.beat < n.start + n.length);
  }
  const snap = (b: number, free: boolean) => (free ? b : Math.round(b / grid) * grid);

  let drag: { i: number; mode: 'move' | 'size'; dBeat: number; dKey: number; orig: ClipNote } | null = null;
  function onpointerdown(e: PointerEvent): void {
    if (e.button !== 0) return;
    const p = at(e);
    const i = hit(p);
    if (i >= 0 && e.altKey) {
      synth.clips.edit(slot, (c) => c.notes.splice(i, 1));
      return;
    }
    canvas!.setPointerCapture(e.pointerId);
    if (i >= 0) {
      const n = clip().notes[i];
      const endPx = ((n.start + n.length) / clip().length) * canvas!.getBoundingClientRect().width;
      const r = canvas!.getBoundingClientRect();
      const nearEnd = e.clientX - r.left > endPx - 6;
      drag = { i, mode: nearEnd ? 'size' : 'move', dBeat: p.beat - n.start, dKey: 0, orig: { ...n } };
    } else {
      const start = Math.max(0, Math.floor(p.beat / grid) * grid);
      if (start >= clip().length) return;
      synth.clips.edit(slot, (c) => c.notes.push({ start, length: grid, key: p.key, velocity: 0.8, chance: 1, bend: 0 }));
      drag = { i: clip().notes.length - 1, mode: 'size', dBeat: 0, dKey: 0, orig: { ...clip().notes[clip().notes.length - 1] } };
      synth.noteOn(p.key, 0.8);
      setTimeout(() => synth.noteOff(p.key), 150);
    }
  }
  function onpointermove(e: PointerEvent): void {
    if (!drag || !canvas!.hasPointerCapture(e.pointerId)) return;
    const p = at(e);
    const d = drag;
    synth.clips.edit(slot, (c) => {
      const n = c.notes[d.i];
      if (!n) return;
      if (d.mode === 'move') {
        n.start = Math.max(0, Math.min(c.length - grid / 4, snap(p.beat - d.dBeat, e.shiftKey)));
        n.key = Math.max(0, Math.min(127, p.key));
      } else {
        n.length = Math.max(grid / 4, snap(p.beat, e.shiftKey) - n.start);
      }
    });
  }
  function ondblclick(e: MouseEvent): void {
    const i = hit(at(e));
    if (i >= 0) synth.clips.edit(slot, (c) => c.notes.splice(i, 1));
  }
  const under = () => clip().notes.findIndex((n) => n.key === cursor.key && cursor.beat >= n.start - 1e-9 && cursor.beat < n.start + n.length - 1e-9);
  const noteName = (k: number) => `${['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'][k % 12]}${Math.floor(k / 12) - 1}`;
  function onkeydown(e: KeyboardEvent): void {
    const len = clip().length;
    const i = under();
    const move = (beat: number, key: number) => {
      cursor = { beat: Math.max(0, Math.min(len - grid, beat)), key: Math.max(0, Math.min(127, key)) };
      if (cursor.key < low) low = cursor.key;
      if (cursor.key > high) low = Math.min(128 - rows, cursor.key - rows + 1);
    };
    switch (e.key) {
      case 'ArrowLeft':
      case 'ArrowRight': {
        const d = e.key === 'ArrowLeft' ? -grid : grid;
        if (e.shiftKey && i >= 0) synth.clips.edit(slot, (c) => (c.notes[i].length = Math.max(grid, Math.min(len - c.notes[i].start, c.notes[i].length + d))));
        else move(cursor.beat + d, cursor.key);
        break;
      }
      case 'ArrowUp':
      case 'ArrowDown':
        move(cursor.beat, cursor.key + (e.key === 'ArrowUp' ? 1 : -1) * (e.shiftKey ? 12 : 1));
        break;
      case 'Enter':
      case ' ':
        if (i >= 0) synth.clips.edit(slot, (c) => c.notes.splice(i, 1));
        else {
          synth.clips.edit(slot, (c) => c.notes.push({ start: cursor.beat, length: grid, key: cursor.key, velocity: 0.8, chance: 1, bend: 0 }));
          synth.noteOn(cursor.key, 0.8);
          const k = cursor.key;
          setTimeout(() => synth.noteOff(k), 150);
        }
        break;
      case 'Delete':
      case 'Backspace':
        if (i >= 0) synth.clips.edit(slot, (c) => c.notes.splice(i, 1));
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }

  const label = () => {
    void version;
    const c = clip();
    return `Clip ${slot + 1}: ${c.notes.length} notes over ${c.length} beats. Cursor at beat ${cursor.beat + 1}, ${noteName(cursor.key)}${under() >= 0 ? ', on a note' : ''}. Arrows move, Enter adds or removes a note.`;
  };

  let scrolled = 0;
  function onwheel(e: WheelEvent): void {
    e.preventDefault();
    const i = hit(at(e));
    if (i >= 0) {
      synth.clips.edit(slot, (c) => (c.notes[i].velocity = Math.min(1, Math.max(0.05, c.notes[i].velocity - e.deltaY / 1000))));
      return;
    }
    scrolled += e.deltaY;
    const keys = Math.trunc(scrolled / 40);
    if (!keys) return;
    scrolled -= keys * 40;
    low = Math.max(0, Math.min(128 - rows, low - keys));
  }
</script>

<!-- an application region: a custom keyboard-driven editor (keys in the header comment) -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="wrap"
  tabindex="0"
  role="application"
  aria-roledescription="piano roll"
  aria-label={label()}
  {onkeydown}
  onfocus={() => (focused = true)}
  onblur={() => (focused = false)}
>
  <canvas bind:this={canvas} class="roll" style:--c={color} data-low={low} data-rows={rows} {onpointerdown} {onpointermove} onpointerup={() => (drag = null)} {ondblclick} {onwheel}></canvas>
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
  }
  .roll {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
    touch-action: none;
  }
</style>
