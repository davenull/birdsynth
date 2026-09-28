<!--
  An LFO's shape, drawn and edited in place, with the newest voice's
  position on it. Normal: drag points, double-click empty space to add one,
  double-click a point to remove it, Alt-drag a point to bend the segment
  after it. Path: the same on the XY loop. Chaos and S&H: a trace of the
  recent output.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { TEL } from '../../gen/protocol';
  import { evalCurve, evalPath, type LfoPoint } from '../../state/lfo';
  import { toPlain } from '../../state/param-math';
  import type { Synth } from '../../synth';
  import { onFrame } from '../frame';

  let { lfo, width = 180, height = 124 }: { lfo: number; width?: number; height?: number } = $props();
  const synth = getContext<Synth>('synth');
  const typeInfo = $derived(PARAMS[PARAM_ID[`lfo.${lfo + 1}.type` as ParamKey]]);

  let kind = $state<'curve' | 'path' | 'trace'>('curve');
  let pts = $state<LfoPoint[]>([]);
  let dot = $state<[number, number] | null>(null);
  let trace = $state('');

  $effect(() => {
    const read = (v: number) => {
      const t = toPlain(typeInfo, v);
      kind = t === 0 ? 'curve' : t === 1 ? 'path' : 'trace';
    };
    read(synth.bank.get(typeInfo.id));
    return synth.bank.subscribe(typeInfo.id, read);
  });
  $effect(() => {
    const i = lfo;
    const k = kind;
    const read = () => (pts = k === 'trace' ? [] : synth.lfo.get(i, k).map((p) => ({ ...p })));
    read();
    return synth.lfo.subscribe((l) => l === i && read());
  });

  const PAD = 6;
  // curve: x 0..1 across, y -1..1 up; path: x and y -1..1 in a centred square
  const side = $derived(Math.min(width, height) - 2 * PAD);
  function toScreen(p: { x: number; y: number }): [number, number] {
    if (kind === 'path') return [width / 2 + (p.x * side) / 2, height / 2 - (p.y * side) / 2];
    return [PAD + p.x * (width - 2 * PAD), height / 2 - (p.y * (height - 2 * PAD)) / 2];
  }
  function fromScreen(x: number, y: number): { x: number; y: number } {
    if (kind === 'path') return { x: ((x - width / 2) * 2) / side, y: ((height / 2 - y) * 2) / side };
    return { x: (x - PAD) / (width - 2 * PAD), y: ((height / 2 - y) * 2) / (height - 2 * PAD) };
  }

  const line = $derived.by(() => {
    if (kind === 'trace' || !pts.length) return '';
    const n = 160;
    let d = '';
    for (let i = 0; i <= n; i++) {
      const t = i / n;
      const p = kind === 'path' ? evalPath(pts, Math.min(t, 0.999999)) : { x: t, y: evalCurve(pts, Math.min(t, 0.999999)) };
      const [x, y] = toScreen(kind === 'path' ? { x: (p as [number, number])[0], y: (p as [number, number])[1] } : (p as { x: number; y: number }));
      d += `${i ? 'L' : 'M'}${x.toFixed(1)} ${y.toFixed(1)}`;
    }
    return d;
  });

  onMount(() => {
    const hist = new Float32Array(160);
    let at = 0;
    let last = 0;
    return onFrame((now) => {
      const t = synth.host?.tel;
      if (!t) return;
      const x = t[TEL.lfoValue + lfo];
      const y = t[TEL.lfoY + lfo];
      const ph = t[TEL.lfoPhase + lfo];
      if (kind === 'curve') dot = toScreen({ x: ph, y: x });
      else if (kind === 'path') dot = toScreen({ x, y });
      else {
        if (now - last > 25) {
          last = now;
          hist[at] = x;
          at = (at + 1) % hist.length;
          let d = '';
          for (let i = 0; i < hist.length; i++) {
            const v = hist[(at + i) % hist.length];
            d += `${i ? 'L' : 'M'}${(PAD + (i / (hist.length - 1)) * (width - 2 * PAD)).toFixed(1)} ${(height / 2 - (v * (height - 2 * PAD)) / 2).toFixed(1)}`;
          }
          trace = d;
        }
        dot = null;
      }
    });
  });

  // --- editing
  let svg = $state<SVGSVGElement>();
  let dragging = -1;
  let bending = false;
  let bendFrom = 0;
  let bendStart = 0;
  function local(e: PointerEvent | MouseEvent): { x: number; y: number } {
    const m = svg!.getScreenCTM()!.inverse();
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m);
    return fromScreen(p.x, p.y);
  }
  function commit(next: LfoPoint[]): void {
    synth.lfo.set(lfo, kind === 'path' ? 'path' : 'curve', next);
  }
  function down(e: PointerEvent, i: number): void {
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    dragging = i;
    bending = e.altKey;
    bendFrom = e.clientY;
    bendStart = pts[i].c;
  }
  function move(e: PointerEvent): void {
    if (dragging < 0) return;
    const next = pts.map((p) => ({ ...p }));
    if (bending) next[dragging].c = Math.max(-1, Math.min(1, bendStart + (bendFrom - e.clientY) / 80));
    else {
      const q = local(e);
      next[dragging].x = q.x;
      next[dragging].y = q.y;
    }
    pts = next; // draw now; the store sorts and clamps on commit
    commit(next);
  }
  function up(): void {
    dragging = -1;
    // after a sort the indices may have moved; re-read the store
    pts = synth.lfo.get(lfo, kind === 'path' ? 'path' : 'curve').map((p) => ({ ...p }));
  }
  function add(e: MouseEvent): void {
    if (kind === 'trace') return;
    const q = local(e);
    if (kind === 'path') {
      // insert after the nearest point
      let best = 0;
      let bd = Infinity;
      pts.forEach((p, i) => {
        const d = (p.x - q.x) ** 2 + (p.y - q.y) ** 2;
        if (d < bd) (bd = d), (best = i);
      });
      commit([...pts.slice(0, best + 1), { x: q.x, y: q.y, c: 0 }, ...pts.slice(best + 1)]);
    } else commit([...pts, { x: q.x, y: q.y, c: 0 }]);
  }
  function remove(e: MouseEvent, i: number): void {
    e.stopPropagation();
    if (pts.length > 1) commit(pts.filter((_, k) => k !== i));
  }
</script>

<svg
  bind:this={svg}
  class="lfo-editor"
  {width}
  {height}
  role="img"
  aria-label={`LFO ${lfo + 1} shape`}
  ondblclick={add}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
>
  <rect class="bg" x="0.5" y="0.5" width={width - 1} height={height - 1} rx="5" />
  <line class="axis" x1={PAD} x2={width - PAD} y1={height / 2} y2={height / 2} />
  {#if kind === 'trace'}
    <path class="shape" d={trace} />
  {:else}
    <path class="shape" d={line} />
    {#each pts as p, i (i)}
      {@const [x, y] = toScreen(p)}
      <circle
        class="pt"
        cx={x}
        cy={y}
        r="4.5"
        role="button"
        tabindex="-1"
        aria-label={`Point ${i + 1}`}
        onpointerdown={(e) => down(e, i)}
        ondblclick={(e) => remove(e, i)}
      />
    {/each}
  {/if}
  {#if dot}<circle class="now" cx={dot[0]} cy={dot[1]} r="4" />{/if}
</svg>

<style>
  .lfo-editor {
    display: block;
    touch-action: none;
    user-select: none;
  }
  .bg {
    fill: var(--glass);
    stroke: var(--line);
  }
  .axis {
    stroke: rgba(255, 255, 255, 0.08);
  }
  .shape {
    fill: none;
    stroke: var(--lfo);
    stroke-width: 1.8;
  }
  .pt {
    fill: var(--panel);
    stroke: var(--lfo);
    stroke-width: 1.5;
    cursor: move;
  }
  .pt:hover {
    fill: var(--lfo);
  }
  .now {
    fill: #fff;
    stroke: var(--lfo);
    stroke-width: 2;
    pointer-events: none;
  }
</style>
