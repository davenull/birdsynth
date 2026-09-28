<!-- Two parameters on one square: across and up. -->
<script lang="ts">
  import { getContext } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../../gen/params';
  import type { ParamBank } from '../../../state/bank';
  import { format } from '../../../state/param-math';

  let { x, y, color = 'var(--accent)' }: { x: ParamKey; y: ParamKey; color?: string } = $props();
  const bank = getContext<ParamBank>('bank');
  const px = $derived(PARAMS[PARAM_ID[x]]);
  const py = $derived(PARAMS[PARAM_ID[y]]);
  let vx = $state(0);
  let vy = $state(0);
  $effect(() => {
    vx = bank.get(px.id);
    return bank.subscribe(px.id, (v) => (vx = v));
  });
  $effect(() => {
    vy = bank.get(py.id);
    return bank.subscribe(py.id, (v) => (vy = v));
  });
  let el = $state<HTMLElement>();
  function set(e: PointerEvent): void {
    const r = el!.getBoundingClientRect();
    bank.set(px.id, Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)));
    bank.set(py.id, Math.min(1, Math.max(0, 1 - (e.clientY - r.top) / r.height)));
  }
  function key(e: KeyboardEvent): void {
    const d = e.shiftKey ? 0.01 : 0.05;
    if (e.key === 'ArrowLeft') bank.set(px.id, vx - d);
    else if (e.key === 'ArrowRight') bank.set(px.id, vx + d);
    else if (e.key === 'ArrowUp') bank.set(py.id, vy + d);
    else if (e.key === 'ArrowDown') bank.set(py.id, vy - d);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<div
  class="pad"
  bind:this={el}
  role="slider"
  tabindex="0"
  aria-label={`${px.name} and ${py.name}`}
  aria-valuenow={Math.round(vx * 100)}
  aria-valuetext={`${px.short} ${format(px, vx)}, ${py.short} ${format(py, vy)}`}
  title={`Across: ${px.name}. Up: ${py.name}.`}
  data-explain="xy"
  style:--c={color}
  onpointerdown={(e) => {
    el!.setPointerCapture(e.pointerId);
    set(e);
  }}
  onpointermove={(e) => el!.hasPointerCapture(e.pointerId) && set(e)}
  onkeydown={key}
>
  <div class="dot" style:left={`${vx * 100}%`} style:bottom={`${vy * 100}%`}></div>
  <span class="lx">{px.short}</span>
  <span class="ly">{py.short}</span>
</div>

<style>
  .pad {
    position: relative;
    height: 100%;
    aspect-ratio: 1;
    background:
      linear-gradient(rgba(255, 255, 255, 0.05) 1px, transparent 1px) 0 0 / 25% 25%,
      linear-gradient(90deg, rgba(255, 255, 255, 0.05) 1px, transparent 1px) 0 0 / 25% 25%,
      var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    touch-action: none;
    cursor: crosshair;
    outline: none;
  }
  .pad:focus-visible {
    border-color: var(--c);
  }
  .dot {
    position: absolute;
    width: 10px;
    height: 10px;
    margin: 0 0 -5px -5px;
    border-radius: 50%;
    background: var(--c);
    box-shadow: 0 0 8px var(--c);
    pointer-events: none;
  }
  .lx,
  .ly {
    position: absolute;
    font-size: 8px;
    color: var(--text-faint);
    pointer-events: none;
  }
  .lx {
    right: 3px;
    bottom: 1px;
  }
  .ly {
    left: 3px;
    top: 1px;
  }
</style>
