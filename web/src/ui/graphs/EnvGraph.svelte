<!-- The envelope's shape (attack, hold, decay, sustain, release) and where the newest voice is on it. -->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { TEL } from '../../gen/protocol';
  import { toPlain } from '../../state/param-math';
  import type { Synth } from '../../synth';
  import { fitCanvas, onFrame } from '../frame';

  let { n = 1, color = 'var(--env)' }: { n?: number; color?: string } = $props();
  const synth = getContext<Synth>('synth');
  let canvas = $state<HTMLCanvasElement>();

  onMount(() => {
    let key = '';
    let stroke = '';
    return onFrame(() => {
      const c = canvas;
      if (!c) return;
      const g = c.getContext('2d');
      if (!g) return;
      const { w, h, dpr } = fitCanvas(c);
      const v = (k: string) => {
        const p = PARAMS[PARAM_ID[`env.${n}.${k}` as ParamKey]];
        return toPlain(p, synth.bank.get(p.id));
      };
      const [a, hold, d, s, r] = [v('attack'), v('hold'), v('decay'), v('sustain'), v('release')];
      const tel = synth.host?.tel;
      const level = tel && tel[TEL.voicesActive] > 0 ? tel[TEL.focusEnv + n - 1] : -1;
      const k = `${n}|${w}|${h}|${a}|${hold}|${d}|${s}|${r}|${level.toFixed(3)}`;
      if (k === key) return;
      key = k;
      stroke ||= getComputedStyle(c).getPropertyValue('--env-color').trim() || '#a6e05a';
      g.clearRect(0, 0, w, h);
      // time is shown on a compressed scale so short and long segments both read
      const sq = (ms: number) => Math.sqrt(Math.max(0, ms)) + 2;
      const seg = [sq(a), sq(hold), sq(d), 14, sq(r)];
      const total = seg.reduce((x, y) => x + y, 0);
      const pad = 4 * dpr;
      const W = w - 2 * pad;
      const H = h - 2 * pad;
      const X = (t: number) => pad + (t / total) * W;
      const Y = (l: number) => pad + (1 - l) * H;
      g.strokeStyle = stroke;
      g.lineWidth = 1.6 * dpr;
      g.beginPath();
      g.moveTo(X(0), Y(0));
      let t = 0;
      const steps = 24;
      for (let i = 1; i <= steps; i++) g.lineTo(X(t + (seg[0] * i) / steps), Y(i / steps));
      t += seg[0];
      g.lineTo(X(t + seg[1]), Y(1));
      t += seg[1];
      for (let i = 1; i <= steps; i++) g.lineTo(X(t + (seg[2] * i) / steps), Y(s + (1 - s) * Math.pow(1 - i / steps, 3)));
      t += seg[2];
      g.lineTo(X(t + seg[3]), Y(s));
      t += seg[3];
      for (let i = 1; i <= steps; i++) g.lineTo(X(t + (seg[4] * i) / steps), Y(s * Math.pow(1 - i / steps, 3)));
      g.stroke();
      g.globalAlpha = 0.12;
      g.fillStyle = stroke;
      g.lineTo(X(total), Y(0));
      g.lineTo(X(0), Y(0));
      g.fill();
      g.globalAlpha = 1;
      if (level >= 0) {
        g.fillStyle = '#fff';
        g.beginPath();
        g.arc(pad + 3 * dpr, Y(level), 3 * dpr, 0, Math.PI * 2);
        g.fill();
      }
    });
  });
</script>

<canvas bind:this={canvas} class="eg" style:--env-color={color} aria-label={`Env ${n} shape`}></canvas>

<style>
  .eg {
    width: 100%;
    height: 100%;
    display: block;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: var(--radius);
  }
</style>
