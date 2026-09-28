<!--
  On-screen piano. Press a key to play; drag across keys to glide from
  note to note. Keys light up while their voices sound.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Synth } from '../../synth';
  import { onFrame } from '../frame';

  let { synth, low = 36, high = 84 }: { synth: Synth; low?: number; high?: number } = $props();

  const NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
  const isBlack = (n: number) => [1, 3, 6, 8, 10].includes(n % 12);
  const noteName = (n: number) => `${NAMES[n % 12]}${Math.floor(n / 12) - 1}`;

  const keys = $derived.by(() => {
    const out: { note: number; black: boolean; x: number }[] = [];
    let white = 0;
    for (let n = low; n <= high; n++) {
      if (isBlack(n)) out.push({ note: n, black: true, x: white - 0.32 });
      else out.push({ note: n, black: false, x: white++ });
    }
    return { list: out, whites: white };
  });

  let sounding = $state(new Set<number>());
  const pressed = new Map<number, number>(); // pointerId -> note

  onMount(() =>
    onFrame(() => {
      const next = new Set(synth.telemetry().notes);
      if (next.size !== sounding.size || [...next].some((n) => !sounding.has(n))) sounding = next;
    }),
  );

  function play(pointer: number, note: number | null): void {
    const was = pressed.get(pointer);
    if (was === note) return;
    if (was !== undefined) synth.noteOff(was);
    if (note == null) pressed.delete(pointer);
    else {
      pressed.set(pointer, note);
      synth.noteOn(note, 0.8);
    }
  }

  const noteAt = (e: PointerEvent): number | null => {
    const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    const n = el?.dataset.note;
    return n === undefined ? null : Number(n);
  };
</script>

<div
  class="keyboard"
  role="group"
  aria-label="Keyboard"
  style:--whites={keys.whites}
  onpointerdown={(e) => {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    play(e.pointerId, noteAt(e));
    e.preventDefault();
  }}
  onpointermove={(e) => {
    if (pressed.has(e.pointerId)) play(e.pointerId, noteAt(e));
  }}
  onpointerup={(e) => play(e.pointerId, null)}
  onpointercancel={(e) => play(e.pointerId, null)}
>
  {#each keys.list as k (k.note)}
    <div
      class="key"
      class:black={k.black}
      class:on={sounding.has(k.note)}
      data-note={k.note}
      style:left={`calc(${k.x} * 100% / var(--whites))`}
      aria-label={noteName(k.note)}
    >
      {#if !k.black && k.note % 12 === 0}<span class="name">{noteName(k.note)}</span>{/if}
    </div>
  {/each}
</div>

<style>
  .keyboard {
    position: relative;
    height: 72px;
    user-select: none;
    touch-action: none;
    background: #000;
    border-radius: 0 0 var(--radius) var(--radius);
    overflow: hidden;
  }
  .key {
    position: absolute;
    top: 0;
    bottom: 0;
    width: calc(100% / var(--whites));
    background: linear-gradient(#d9dde3, #f4f5f7 70%);
    border-right: 1px solid #9aa1ad;
    box-sizing: border-box;
  }
  .key.black {
    width: calc(100% / var(--whites) * 0.64);
    bottom: 38%;
    background: linear-gradient(#1b1e24, #333a45);
    border: 1px solid #0b0c0f;
    border-top: 0;
    border-radius: 0 0 3px 3px;
    z-index: 1;
  }
  .key.on {
    background: var(--accent);
  }
  .key.black.on {
    background: color-mix(in srgb, var(--accent) 75%, #000);
  }
  .name {
    position: absolute;
    bottom: 3px;
    left: 0;
    right: 0;
    text-align: center;
    font-size: 9px;
    color: #59606b;
    pointer-events: none;
  }
</style>
