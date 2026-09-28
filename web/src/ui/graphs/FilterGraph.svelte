<!--
  The filter's response curve (exact, see state/filter-response.ts). Drag
  sideways for cutoff, up and down for resonance.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { TEL } from '../../gen/protocol';
  import { response } from '../../state/filter-response';
  import { toNorm, toPlain } from '../../state/param-math';
  import type { Synth } from '../../synth';
  import { fitCanvas, onFrame } from '../frame';

  let { n = 1, color = 'var(--filter)' }: { n?: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  const P = (k: string) => PARAMS[PARAM_ID[`filter.${n}.${k}` as ParamKey]];
  const [pType, pCut, pRes, pOn] = [P('type'), P('cutoff'), P('res'), P('enable')];
  let canvas = $state<HTMLCanvasElement>();

  const F0 = 20;
  const F1 = 20_000;
  const DB0 = -42;
  const DB1 = 24;
  const xOf = (f: number, w: number) => (Math.log(f / F0) / Math.log(F1 / F0)) * w;
  const fOf = (x: number, w: number) => F0 * Math.pow(F1 / F0, x / w);

  onMount(() => {
    let key = '';
    let stroke = '';
    return onFrame(() => {
      const c = canvas;
      if (!c) return;
      const g = c.getContext('2d');
      if (!g) return;
      const { w, h, dpr } = fitCanvas(c);
      const b = synth.bank;
      const kind = toPlain(pType, b.get(pType.id));
      const cutoff = toPlain(pCut, b.get(pCut.id));
      const res = toPlain(pRes, b.get(pRes.id));
      const on = b.get(pOn.id) >= 0.5;
      const tel = synth.host?.tel;
      const live = tel && tel[TEL.voicesActive] > 0 && on ? tel[TEL.focusCutoff] : 0;
      const sr = synth.host?.ctx.sampleRate ?? 48_000;
      const k = `${w}|${h}|${kind}|${cutoff.toFixed(1)}|${res.toFixed(3)}|${on}|${live.toFixed(0)}`;
      if (k === key) return;
      key = k;
      stroke ||= getComputedStyle(c).getPropertyValue('--graph-color').trim() || '#e6b450';
      g.clearRect(0, 0, w, h);
      g.strokeStyle = 'rgba(255,255,255,0.06)';
      g.lineWidth = dpr;
      for (const f of [100, 1000, 10_000]) {
        const x = xOf(f, w);
        g.beginPath();
        g.moveTo(x, 0);
        g.lineTo(x, h);
        g.stroke();
      }
      const y0 = ((DB1 - 0) / (DB1 - DB0)) * h;
      g.beginPath();
      g.moveTo(0, y0);
      g.lineTo(w, y0);
      g.stroke();
      const curve = (fc: number, alpha: number, width: number) => {
        g.globalAlpha = alpha;
        g.strokeStyle = stroke;
        g.lineWidth = width * dpr;
        g.beginPath();
        for (let i = 0; i <= 160; i++) {
          const x = (i / 160) * w;
          const db = 20 * Math.log10(Math.max(1e-6, response(kind, fc, res, sr, fOf(x, w))));
          const y = ((DB1 - Math.max(DB0, Math.min(DB1, db))) / (DB1 - DB0)) * h;
          if (i) g.lineTo(x, y);
          else g.moveTo(x, y);
        }
        g.stroke();
        g.globalAlpha = 1;
      };
      curve(cutoff, on ? 1 : 0.35, 1.6);
      if (live && Math.abs(live - cutoff) / cutoff > 0.01) curve(live, 0.45, 1.2);
    });
  });

  let drag: { x: number; y: number; c: number; r: number } | null = null;
</script>

<canvas
  bind:this={canvas}
  class="fg"
  style:--graph-color={color}
  aria-label={`Filter ${n} response; drag to set cutoff and resonance`}
  onpointerdown={(e) => {
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, c: synth.bank.get(pCut.id), r: synth.bank.get(pRes.id) };
  }}
  onpointermove={(e) => {
    if (!drag) return;
    const el = e.currentTarget as HTMLElement;
    synth.bank.set(pCut.id, drag.c + (e.clientX - drag.x) / el.clientWidth);
    synth.bank.set(pRes.id, drag.r - (e.clientY - drag.y) / (el.clientHeight * 2));
  }}
  onpointerup={() => (drag = null)}
  ondblclick={() => {
    synth.bank.set(pCut.id, pCut.def);
    synth.bank.set(pRes.id, toNorm(pRes, toPlain(pRes, pRes.def)));
  }}
></canvas>

<style>
  .fg {
    width: 100%;
    height: 100%;
    display: block;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    cursor: move;
    touch-action: none;
  }
</style>
