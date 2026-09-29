<!--
  A rotary control bound to one parameter.
  Drag up/down (Shift: fine), scroll, or use the arrow keys; double-click
  resets to the default; Ctrl/Cmd-click types a value. The element is an
  ARIA slider named after the parameter's full name, e.g. "Osc A Level".

  Modulation shows as rings in each source's colour (the range the source
  can move the value through) and a dot where the newest voice has it now.
  Alt-drag changes the first routing's amount. Knobs are drop targets for
  the source handles (see ui/mod/drag.ts).
-->
<script lang="ts">
  import { getContext } from 'svelte';
  import { FLAG, PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { SOURCES, TEL, TEL_COUNT } from '../../gen/protocol';
  import { format, parse, snap, steps } from '../../state/param-math';
  import type { ParamBank } from '../../state/bank';
  import type { Synth } from '../../synth';
  import { sourceColor } from '../mod/sources';
  import { onFrame } from '../frame';
  import { menu } from './menu.svelte';

  let {
    param,
    label,
    size = 44,
    color = 'var(--accent)',
    compact = false,
  }: { param: ParamKey; label?: string; size?: number; color?: string; compact?: boolean } = $props();

  const bank = getContext<ParamBank>('bank');
  const synth = getContext<Synth | undefined>('synth');
  const info = $derived(PARAMS[PARAM_ID[param]]);
  const modulatable = $derived(!!(info.flags & FLAG.mod));
  let value = $state(0);
  $effect(() => {
    value = bank.get(info.id);
    return bank.subscribe(info.id, (v) => (value = v));
  });

  // --- modulation aimed at this parameter
  interface Mod {
    slot: number;
    amount: number;
    bipolar: boolean;
    color: string;
    name: string;
  }
  let mods = $state<Mod[]>([]);
  let live = $state<number | null>(null);
  $effect(() => {
    const m = synth?.matrix;
    if (!m) return;
    const id = info.id;
    const read = () =>
      (mods = m.forDest(id).map((slot) => {
        const s = m.slots[slot]!;
        return { slot, amount: s.bypass ? 0 : s.amount * s.output, bipolar: s.bipolar, color: sourceColor(s.source), name: SOURCES[s.source] as string };
      }));
    read();
    return m.subscribeDest(id, read);
  });
  $effect(() => {
    if (!mods.length || !synth) {
      live = null;
      return;
    }
    const id = info.id;
    return onFrame(() => {
      const t = synth.host?.tel;
      let v: number | null = null;
      if (t && t[TEL.voicesActive] > 0) {
        for (let d = 0; d < TEL_COUNT.modDest; d++) {
          if (t[TEL.modDest + d] === id) {
            v = t[TEL.modValue + d];
            break;
          }
        }
      }
      if (v !== live) live = v;
    });
  });

  // --- MIDI learn
  let learning = $state(false);
  let cc = $state<number | null>(null);
  $effect(() => {
    const l = synth?.learn;
    if (!l) return;
    const key = param;
    const read = () => {
      learning = l.armed === key;
      cc = l.mapping(key)?.cc ?? null;
    };
    read();
    return l.subscribe(read);
  });

  function oncontextmenu(e: MouseEvent): void {
    const l = synth?.learn;
    menu.show(e, [
      learning ? { label: 'Stop MIDI learn', action: () => l?.cancel() } : { label: 'MIDI learn', action: () => l?.arm(param), disabled: !l },
      ...(cc !== null ? [{ label: `Forget CC ${cc}`, action: () => l?.clear(param) }] : []),
      { label: 'Reset to default', action: () => set(info.def) },
      ...(mods.length ? [{ label: mods.length > 1 ? `Remove ${mods.length} modulations` : 'Remove modulation', action: () => mods.forEach((m) => synth?.matrix.remove(m.slot)) }] : []),
    ]);
  }

  const A0 = -135;
  const A1 = 135;
  const bipolar = $derived(info.min < 0 && info.max > 0);
  const zero = $derived(bipolar ? -info.min / (info.max - info.min) : 0);
  const angle = $derived(A0 + (A1 - A0) * value);
  const text = $derived(format(info, value));
  const stepCount = $derived(steps(info));

  const clamp01 = (x: number) => Math.min(1, Math.max(0, x));
  /** The ring for one routing: the range the source moves the value through. */
  function ring(m: Mod): [number, number] {
    const lo = m.bipolar ? value - Math.abs(m.amount) : Math.min(value, value + m.amount);
    const hi = m.bipolar ? value + Math.abs(m.amount) : Math.max(value, value + m.amount);
    return [A0 + (A1 - A0) * clamp01(lo), A0 + (A1 - A0) * clamp01(hi)];
  }
  const modText = $derived(mods.map((m) => `${m.name} ${m.amount >= 0 ? '+' : ''}${Math.round(m.amount * 100)}%`).join(', '));

  function polar(deg: number, r: number): [number, number] {
    const a = ((deg - 90) * Math.PI) / 180;
    return [r * Math.cos(a), r * Math.sin(a)];
  }
  function arc(a: number, b: number, r = 38): string {
    const [lo, hi] = a <= b ? [a, b] : [b, a];
    if (hi - lo < 0.01) return '';
    const [x0, y0] = polar(lo, r);
    const [x1, y1] = polar(hi, r);
    return `M${x0.toFixed(2)} ${y0.toFixed(2)}A${r} ${r} 0 ${hi - lo > 180 ? 1 : 0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`;
  }

  function set(v: number): void {
    bank.set(info.id, snap(info, v));
  }

  let dragFrom = 0;
  let dragValue = 0;
  /** Slot whose amount an Alt-drag is changing, or -1. */
  let dragSlot = -1;
  function onpointerdown(e: PointerEvent): void {
    if (e.button !== 0) return;
    const el = e.currentTarget as HTMLElement;
    el.focus();
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      startEdit();
      return;
    }
    el.setPointerCapture(e.pointerId);
    dragFrom = e.clientY;
    dragSlot = e.altKey && mods.length ? mods[0].slot : -1;
    dragValue = dragSlot >= 0 ? (synth!.matrix.slots[dragSlot]?.amount ?? 0) : value;
    e.preventDefault();
  }
  function onpointermove(e: PointerEvent): void {
    const el = e.currentTarget as HTMLElement;
    if (!el.hasPointerCapture(e.pointerId)) return;
    const px = e.shiftKey ? 1200 : 200; // pixels for the whole range
    const v = dragValue + (dragFrom - e.clientY) / px;
    if (dragSlot >= 0) synth!.matrix.update(dragSlot, { amount: Math.fround(Math.max(-1, Math.min(1, v))) });
    else set(v);
  }
  function onwheel(e: WheelEvent): void {
    e.preventDefault();
    const dir = e.deltaY < 0 ? 1 : -1;
    set(value + dir * (stepCount ? 1 / stepCount : e.shiftKey ? 0.002 : 0.02));
  }
  function onkeydown(e: KeyboardEvent): void {
    const fine = stepCount ? 1 / stepCount : e.shiftKey ? 0.001 : 0.01;
    switch (e.key) {
      case 'ArrowUp':
      case 'ArrowRight':
        set(value + fine);
        break;
      case 'ArrowDown':
      case 'ArrowLeft':
        set(value - fine);
        break;
      case 'PageUp':
        set(value + 0.1);
        break;
      case 'PageDown':
        set(value - 0.1);
        break;
      case 'Home':
        set(0);
        break;
      case 'End':
        set(1);
        break;
      case 'Enter':
        startEdit();
        break;
      case 'Delete':
      case 'Backspace':
        set(info.def);
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation(); // keep arrow keys from reaching the QWERTY player
  }

  let editing = $state(false);
  let draft = $state('');
  let input = $state<HTMLInputElement>();
  function startEdit(): void {
    draft = text;
    editing = true;
    queueMicrotask(() => input?.select());
  }
  function commit(): void {
    const v = parse(info, draft);
    if (v != null) set(v);
    editing = false;
  }
</script>

<div
  class="knob"
  role="slider"
  tabindex="0"
  aria-label={info.name}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={Math.round(value * 100)}
  aria-valuetext={mods.length ? `${text}, modulated by ${modText}` : text}
  title={`${info.explain}${mods.length ? `\nModulated by ${modText} (Alt-drag to change)` : ''}${cc !== null ? `\nMIDI CC ${cc}` : ''}${learning ? '\nMove a MIDI control to tie it to this knob' : ''}`}
  class:learning
  data-param={param}
  data-explain={param}
  data-mod={modulatable ? '1' : '0'}
  style:--knob-color={color}
  style:--knob-w={`${Math.max(compact ? 38 : 44, size + 18)}px`}
  {onpointerdown}
  {onpointermove}
  {onwheel}
  {onkeydown}
  {oncontextmenu}
  ondblclick={() => set(info.def)}
>
  <svg width={size} height={size} viewBox="-50 -50 100 100" aria-hidden="true">
    <circle class="face" r="30" />
    <path class="track" d={arc(A0, A1)} />
    <path class="fill" d={bipolar ? arc(A0 + (A1 - A0) * zero, angle) : arc(A0, angle)} />
    <line class="pointer" x1="0" y1="-12" x2="0" y2="-28" transform={`rotate(${angle})`} />
    {#each mods.slice(0, 2) as m, i (m.slot)}
      {@const [a, b] = ring(m)}
      <path class="ring" d={arc(a, b, 46 + i * 5)} style:stroke={m.color} />
    {/each}
    {#if live !== null && mods.length}
      {@const [x, y] = polar(A0 + (A1 - A0) * live, 46)}
      <circle class="dot" cx={x} cy={y} r="4.5" style:fill={mods[0].color} />
    {/if}
  </svg>
  {#if cc !== null || learning}<span class="cc" aria-hidden="true">{learning ? 'learn' : `CC${cc}`}</span>{/if}
  <div class="label">{label ?? info.short}</div>
  {#if editing}
    <input
      bind:this={input}
      bind:value={draft}
      class="edit"
      aria-label={`${info.name} value`}
      onkeydown={(e) => {
        e.stopPropagation();
        if (e.key === 'Enter') commit();
        if (e.key === 'Escape') editing = false;
      }}
      onblur={commit}
    />
  {:else}
    <div class="value">{text}</div>
  {/if}
</div>

<style>
  .knob {
    display: grid;
    justify-items: center;
    gap: 1px;
    width: var(--knob-w, 64px);
    user-select: none;
    touch-action: none;
    cursor: ns-resize;
    outline: none;
    border-radius: var(--radius);
    padding: 4px 0 3px;
  }
  .knob:focus-visible {
    box-shadow: 0 0 0 2px var(--knob-color);
  }
  .knob {
    position: relative;
  }
  .knob.learning {
    animation: learn 0.9s ease-in-out infinite alternate;
  }
  @keyframes learn {
    from {
      box-shadow: 0 0 0 1px color-mix(in srgb, var(--macro) 30%, transparent);
    }
    to {
      box-shadow: 0 0 0 2px var(--macro);
    }
  }
  .cc {
    position: absolute;
    top: 0;
    right: 0;
    font: 8px var(--font-num);
    color: var(--macro);
    pointer-events: none;
  }
  svg {
    overflow: visible;
  }
  .face {
    fill: var(--knob-face);
    stroke: var(--line);
    stroke-width: 1.5;
  }
  .track {
    fill: none;
    stroke: var(--knob-track);
    stroke-width: 7;
    stroke-linecap: round;
  }
  .fill {
    fill: none;
    stroke: var(--knob-color);
    stroke-width: 7;
    stroke-linecap: round;
  }
  .pointer {
    stroke: var(--text);
    stroke-width: 4;
    stroke-linecap: round;
  }
  .ring {
    fill: none;
    stroke-width: 3.5;
    stroke-linecap: round;
    opacity: 0.9;
  }
  .dot {
    stroke: var(--panel);
    stroke-width: 1.5;
  }
  .label {
    font-size: 10px;
    color: var(--text-dim);
    letter-spacing: 0.02em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: var(--knob-w, 64px);
  }
  .value,
  .edit {
    font: 10px/1.2 var(--font-num);
    color: var(--text);
    white-space: nowrap;
    width: calc(var(--knob-w, 64px) - 2px);
    overflow: hidden;
    text-overflow: ellipsis;
    text-align: center;
  }
  .edit {
    background: var(--panel-2);
    border: 1px solid var(--knob-color);
    border-radius: 3px;
    padding: 0;
    outline: none;
  }
</style>
