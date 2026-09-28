<!--
  An oscillator's remap curve (for the Remap 1-4 warps): x is where in the
  cycle (or, for Remap 4, the input level), y where it maps to. Drag points,
  double-click to add or remove, Alt-drag to bend a segment.
-->
<script lang="ts">
  import { getContext } from 'svelte';
  import type { LfoPoint } from '../../state/lfo';
  import { curveLut } from '../../state/remap';
  import type { Synth } from '../../synth';

  let { osc, width = 150, height = 72, color = 'var(--accent)' }: { osc: number; width?: number; height?: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  const PAD = 5;

  let pts = $state<LfoPoint[]>([]);
  $effect(() => {
    const o = osc;
    const read = () => (pts = synth.remap.points[o].map((p) => ({ ...p })));
    read();
    return synth.remap.subscribe((i) => i === o && read());
  });

  const sx = (x: number) => PAD + x * (width - 2 * PAD);
  const sy = (y: number) => height - PAD - y * (height - 2 * PAD);
  const line = $derived.by(() => {
    if (!pts.length) return '';
    const lut = curveLut(pts);
    let d = '';
    for (let i = 0; i < lut.length; i += 4) d += `${i ? 'L' : 'M'}${sx(i / (lut.length - 1)).toFixed(1)} ${sy(lut[i]).toFixed(1)}`;
    return d + `L${sx(1).toFixed(1)} ${sy(lut[lut.length - 1]).toFixed(1)}`;
  });

  let svg = $state<SVGSVGElement>();
  let dragging = -1;
  let bending = false;
  let from = 0;
  let c0 = 0;
  function local(e: PointerEvent | MouseEvent): { x: number; y: number } {
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(svg!.getScreenCTM()!.inverse());
    return { x: (p.x - PAD) / (width - 2 * PAD), y: (height - PAD - p.y) / (height - 2 * PAD) };
  }
  function down(e: PointerEvent, i: number): void {
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    dragging = i;
    bending = e.altKey;
    from = e.clientY;
    c0 = pts[i].c;
  }
  function move(e: PointerEvent): void {
    if (dragging < 0) return;
    const next = pts.map((p) => ({ ...p }));
    if (bending) next[dragging].c = Math.max(-1, Math.min(1, c0 + (from - e.clientY) / 60));
    else Object.assign(next[dragging], local(e));
    pts = next;
    synth.remap.set(osc, next);
  }
  function up(): void {
    dragging = -1;
    pts = synth.remap.points[osc].map((p) => ({ ...p }));
  }
</script>

<svg
  bind:this={svg}
  class="remap"
  {width}
  {height}
  role="img"
  aria-label={`Osc ${'ABC'[osc]} remap curve`}
  style:--c={color}
  ondblclick={(e) => synth.remap.set(osc, [...pts, { ...local(e), c: 0 }])}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
>
  <rect x="0.5" y="0.5" width={width - 1} height={height - 1} rx="4" class="bg" />
  <line class="diag" x1={sx(0)} y1={sy(0)} x2={sx(1)} y2={sy(1)} />
  <path class="curve" d={line} />
  {#each pts as p, i (i)}
    <circle
      class="pt"
      cx={sx(p.x)}
      cy={sy(p.y)}
      r="4"
      role="button"
      tabindex="-1"
      aria-label={`Remap point ${i + 1}`}
      onpointerdown={(e) => down(e, i)}
      ondblclick={(e) => {
        e.stopPropagation();
        if (pts.length > 2) synth.remap.set(osc, pts.filter((_, k) => k !== i));
      }}
    />
  {/each}
</svg>

<style>
  .remap {
    display: block;
    touch-action: none;
    user-select: none;
  }
  .bg {
    fill: var(--glass);
    stroke: var(--line);
  }
  .diag {
    stroke: rgba(255, 255, 255, 0.08);
    stroke-dasharray: 3 3;
  }
  .curve {
    fill: none;
    stroke: var(--c);
    stroke-width: 1.6;
  }
  .pt {
    fill: var(--panel);
    stroke: var(--c);
    stroke-width: 1.4;
    cursor: move;
  }
</style>
