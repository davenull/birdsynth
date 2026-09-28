<!--
  The filter's response curve, computed by the tools worker from the same
  Rust code the engine runs (exact for the linear types). Drag sideways for
  cutoff, up and down for resonance. While a voice plays, a fainter curve
  shows where the modulated cutoff has it now.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { TEL } from '../../gen/protocol';
  import { toNorm, toPlain } from '../../state/param-math';
  import { tools } from '../../tools/client';
  import type { Synth } from '../../synth';
  import { fitCanvas, onFrame } from '../frame';

  let { n = 1, color = 'var(--filter)' }: { n?: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  const P = (k: string) => PARAMS[PARAM_ID[`filter.${n}.${k}` as ParamKey]];
  const [pType, pCut, pRes, pOn, pVar] = [P('type'), P('cutoff'), P('res'), P('enable'), P('var')];
  let canvas = $state<HTMLCanvasElement>();

  const F0 = 20;
  const F1 = 20_000;
  const DB0 = -42;
  const DB1 = 24;
  const POINTS = 161;

  onMount(() => {
    let key = '';
    let stroke = '';
    // curves arrive from the worker; `want` is what the controls ask for now
    const curves: Record<'base' | 'live', { key: string; db: Float32Array | null; busy: boolean; at: number }> = {
      base: { key: '', db: null, busy: false, at: 0 },
      live: { key: '', db: null, busy: false, at: 0 },
    };
    const request = (which: 'base' | 'live', kind: number, cutoff: number, res: number, v: number, sr: number, now: number) => {
      const c = curves[which];
      const want = `${kind}|${cutoff.toFixed(1)}|${res.toFixed(3)}|${v.toFixed(3)}|${sr}`;
      if (want === c.key || c.busy || (which === 'live' && now - c.at < 40)) return;
      c.busy = true;
      c.at = now;
      tools()
        .call({ op: 'filterResponse', kind, cutoff, res, var: v, sr, f0: F0, f1: F1, points: POINTS })
        .then((db) => {
          c.db = db;
          c.key = want;
          key = ''; // redraw
        })
        .finally(() => (c.busy = false));
    };
    return onFrame((now) => {
      const cv = canvas;
      if (!cv) return;
      const g = cv.getContext('2d');
      if (!g) return;
      const { w, h, dpr } = fitCanvas(cv);
      const b = synth.bank;
      const kind = toPlain(pType, b.get(pType.id));
      const cutoff = toPlain(pCut, b.get(pCut.id));
      const res = toPlain(pRes, b.get(pRes.id));
      const v = toPlain(pVar, b.get(pVar.id));
      const on = b.get(pOn.id) >= 0.5;
      const tel = synth.host?.tel;
      // telemetry reports filter 1's live cutoff
      const live = n === 1 && tel && tel[TEL.voicesActive] > 0 && on ? tel[TEL.focusCutoff] : 0;
      const sr = synth.host?.ctx.sampleRate ?? 48_000;
      request('base', kind, cutoff, res, v, sr, now);
      const showLive = live > 0 && Math.abs(live - cutoff) / cutoff > 0.01;
      if (showLive) request('live', kind, live, res, v, sr, now);
      const k = `${w}|${h}|${on}|${showLive}|${curves.base.key}|${curves.live.key}`;
      if (k === key) return;
      key = k;
      stroke ||= getComputedStyle(cv).getPropertyValue('--graph-color').trim() || '#e6b450';
      g.clearRect(0, 0, w, h);
      g.strokeStyle = 'rgba(255,255,255,0.06)';
      g.lineWidth = dpr;
      for (const f of [100, 1000, 10_000]) {
        const x = (Math.log(f / F0) / Math.log(F1 / F0)) * w;
        g.beginPath();
        g.moveTo(x, 0);
        g.lineTo(x, h);
        g.stroke();
      }
      const y0 = (DB1 / (DB1 - DB0)) * h;
      g.beginPath();
      g.moveTo(0, y0);
      g.lineTo(w, y0);
      g.stroke();
      const draw = (db: Float32Array | null, alpha: number, width: number) => {
        if (!db) return;
        g.globalAlpha = alpha;
        g.strokeStyle = stroke;
        g.lineWidth = width * dpr;
        g.beginPath();
        for (let i = 0; i < db.length; i++) {
          const x = (i / (db.length - 1)) * w;
          const y = ((DB1 - Math.max(DB0, Math.min(DB1, db[i]))) / (DB1 - DB0)) * h;
          if (i) g.lineTo(x, y);
          else g.moveTo(x, y);
        }
        g.stroke();
        g.globalAlpha = 1;
      };
      draw(curves.base.db, on ? 1 : 0.35, 1.6);
      if (showLive) draw(curves.live.db, 0.45, 1.2);
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
