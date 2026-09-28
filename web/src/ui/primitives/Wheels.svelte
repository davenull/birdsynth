<!--
  On-screen pitch and mod wheels, for playing without a controller (and in
  Safari, which has no Web MIDI). Pitch springs back to the centre when
  let go; mod stays. Both follow a connected controller's wheels.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Synth } from '../../synth';

  let { synth }: { synth: Synth } = $props();
  let bend = $state(0);
  let mod = $state(0);

  onMount(() =>
    synth.onWheels(() => {
      bend = synth.wheels.bend;
      mod = synth.wheels.mod;
    }),
  );

  const H = 62;
  function drag(kind: 'bend' | 'mod') {
    return {
      down(e: PointerEvent) {
        const el = e.currentTarget as HTMLElement;
        el.setPointerCapture(e.pointerId);
        el.focus();
        this.move(e);
        e.preventDefault();
      },
      move(e: PointerEvent) {
        const el = e.currentTarget as HTMLElement;
        if (!el.hasPointerCapture(e.pointerId)) return;
        const r = el.getBoundingClientRect();
        const y = Math.min(1, Math.max(0, 1 - (e.clientY - r.top) / r.height));
        if (kind === 'bend') synth.pitchBend(0, y * 2 - 1);
        else synth.controller(0, 1, y);
      },
      up() {
        if (kind === 'bend') synth.pitchBend(0, 0);
      },
    };
  }
  const bendDrag = drag('bend');
  const modDrag = drag('mod');

  function key(kind: 'bend' | 'mod', e: KeyboardEvent): void {
    const d = e.key === 'ArrowUp' ? 0.1 : e.key === 'ArrowDown' ? -0.1 : 0;
    if (!d) return;
    e.preventDefault();
    e.stopPropagation();
    if (kind === 'bend') synth.pitchBend(0, Math.max(-1, Math.min(1, bend + d)));
    else synth.controller(0, 1, Math.max(0, Math.min(1, mod + d)));
  }
</script>

<div class="wheels" data-explain="wheels">
  {#each [{ kind: 'bend' as const, name: 'Pitch wheel', short: 'PITCH', v: (bend + 1) / 2, h: bendDrag, color: 'var(--ctl)' }, { kind: 'mod' as const, name: 'Mod wheel', short: 'MOD', v: mod, h: modDrag, color: 'var(--ctl)' }] as w (w.kind)}
    <div class="wheel">
      <div
        class="track"
        role="slider"
        tabindex="0"
        aria-label={w.name}
        aria-valuemin={w.kind === 'bend' ? -100 : 0}
        aria-valuemax={100}
        aria-valuenow={Math.round(w.kind === 'bend' ? bend * 100 : mod * 100)}
        style:height={`${H}px`}
        onpointerdown={(e) => w.h.down(e)}
        onpointermove={(e) => w.h.move(e)}
        onpointerup={() => w.h.up()}
        onpointercancel={() => w.h.up()}
        onkeydown={(e) => key(w.kind, e)}
        onkeyup={(e) => w.kind === 'bend' && (e.key === 'ArrowUp' || e.key === 'ArrowDown') && synth.pitchBend(0, 0)}
      >
        {#if w.kind === 'bend'}<div class="centre"></div>{/if}
        <div class="fill" style:background={w.color} style:height={w.kind === 'bend' ? '0' : `${w.v * 100}%`}></div>
        <div class="thumb" style:bottom={`calc(${w.v * 100}% - ${w.v * 10}px)`}></div>
      </div>
      <span>{w.short}</span>
    </div>
  {/each}
</div>

<style>
  .wheels {
    display: flex;
    gap: 6px;
    align-items: flex-end;
    padding: 0 2px;
  }
  .wheel {
    display: grid;
    justify-items: center;
    gap: 2px;
  }
  .wheel span {
    font: 600 8px var(--font-ui);
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .track {
    position: relative;
    width: 20px;
    background: linear-gradient(90deg, #0b0c0f, #1f232b 50%, #0b0c0f);
    border: 1px solid var(--line);
    border-radius: 5px;
    touch-action: none;
    cursor: ns-resize;
    overflow: hidden;
    outline: none;
  }
  .track:focus-visible {
    border-color: var(--ctl);
  }
  .fill {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    opacity: 0.25;
  }
  .centre {
    position: absolute;
    left: 3px;
    right: 3px;
    top: 50%;
    height: 1px;
    background: var(--text-faint);
  }
  .thumb {
    position: absolute;
    left: 1px;
    right: 1px;
    height: 10px;
    border-radius: 3px;
    background: linear-gradient(#cfd4dc, #8f96a3);
  }
</style>
