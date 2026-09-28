<!--
  A rotary control bound to one parameter.
  Drag up/down (Shift: fine), scroll, or use the arrow keys; double-click
  resets to the default; Ctrl/Cmd-click types a value. The element is an
  ARIA slider named after the parameter's full name, e.g. "Osc A Level".
-->
<script lang="ts">
  import { getContext } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { format, parse, snap, steps } from '../../state/param-math';
  import type { ParamBank } from '../../state/bank';

  let {
    param,
    label,
    size = 44,
    color = 'var(--accent)',
  }: { param: ParamKey; label?: string; size?: number; color?: string } = $props();

  const bank = getContext<ParamBank>('bank');
  const info = $derived(PARAMS[PARAM_ID[param]]);
  let value = $state(0);
  $effect(() => {
    value = bank.get(info.id);
    return bank.subscribe(info.id, (v) => (value = v));
  });

  const A0 = -135;
  const A1 = 135;
  const bipolar = $derived(info.min < 0 && info.max > 0);
  const zero = $derived(bipolar ? -info.min / (info.max - info.min) : 0);
  const angle = $derived(A0 + (A1 - A0) * value);
  const text = $derived(format(info, value));
  const stepCount = $derived(steps(info));

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
    dragValue = value;
    e.preventDefault();
  }
  function onpointermove(e: PointerEvent): void {
    const el = e.currentTarget as HTMLElement;
    if (!el.hasPointerCapture(e.pointerId)) return;
    const px = e.shiftKey ? 1200 : 200; // pixels for the whole range
    set(dragValue + (dragFrom - e.clientY) / px);
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
  aria-valuetext={text}
  title={info.explain}
  data-param={param}
  data-explain={param}
  style:--knob-color={color}
  style:--knob-w={`${Math.max(44, size + 18)}px`}
  {onpointerdown}
  {onpointermove}
  {onwheel}
  {onkeydown}
  ondblclick={() => set(info.def)}
>
  <svg width={size} height={size} viewBox="-50 -50 100 100" aria-hidden="true">
    <circle class="face" r="30" />
    <path class="track" d={arc(A0, A1)} />
    <path class="fill" d={bipolar ? arc(A0 + (A1 - A0) * zero, angle) : arc(A0, angle)} />
    <line class="pointer" x1="0" y1="-12" x2="0" y2="-28" transform={`rotate(${angle})`} />
  </svg>
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
    box-shadow: 0 0 0 1px var(--knob-color);
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
