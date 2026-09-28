<!--
  The wavetable display. 3D: the table's frames stacked back to front, with
  the frame now playing drawn in the oscillator's colour. 2D: one cycle of
  the current frame as it sounds through the warps. Click to switch.
  The position follows the newest voice, so modulation is visible.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAM_ID, type ParamKey } from '../../gen/params';
  import { CONST, TEL } from '../../gen/protocol';
  import { toPlain } from '../../state/param-math';
  import { PARAMS } from '../../gen/params';
  import type { Synth } from '../../synth';
  import type { OscTable } from '../../state/tables';
  import { tools } from '../../tools/client';
  import { fitCanvas, onFrame } from '../frame';

  let { osc, color }: { osc: number; color: string } = $props();
  const synth = getContext<Synth>('synth');
  const letter = $derived('abc'[osc]);
  const id = (k: string) => PARAM_ID[`osc.${letter}.${k}` as ParamKey];

  let canvas = $state<HTMLCanvasElement>();
  let mode = $state<'3d' | '2d'>('3d');
  let table = $state<OscTable | null>(null);

  const POINTS = 96;
  const MAX_SHOWN = 40;

  onMount(() => {
    table = synth.tables.osc[osc];
    const offTables = synth.tables.onChange((o, t) => {
      if (o === osc) {
        table = t;
        layer = null;
      }
    });
    let layer: HTMLCanvasElement | null = null;
    let drawnKey = '';
    let preview: Float32Array | null = null;
    let previewKey = '';
    let previewBusy = false;
    let remapVer = 0;
    const offRemap = synth.remap.subscribe((o) => o === osc && remapVer++);
    let stroke = '';

    const frameAt = (t: OscTable, pos: number, out: Float32Array) => {
      const f = pos * (t.count - 1);
      const i0 = Math.floor(f);
      const i1 = Math.min(i0 + 1, t.count - 1);
      const w = f - i0;
      const a = t.frames.subarray(i0 * CONST.frameLen, (i0 + 1) * CONST.frameLen);
      const b = t.frames.subarray(i1 * CONST.frameLen, (i1 + 1) * CONST.frameLen);
      for (let i = 0; i < CONST.frameLen; i++) out[i] = a[i] + (b[i] - a[i]) * w;
      return out;
    };
    const scratch = new Float32Array(CONST.frameLen);

    const off = onFrame(() => {
      const c = canvas;
      if (!c) return;
      const g = c.getContext('2d');
      if (!g) return;
      const { w, h, dpr } = fitCanvas(c);
      const tel = synth.host?.tel;
      const bank = synth.bank;
      const voices = tel ? tel[TEL.voicesActive] : 0;
      const pos = voices > 0 && tel ? tel[TEL.oscWtPos + osc] : toPlain(PARAMS[id('wt_pos')], bank.get(id('wt_pos')));
      const t = table;
      const warps: [number, number, number, number] = [
        toPlain(PARAMS[id('warp1_mode')], bank.get(id('warp1_mode'))),
        toPlain(PARAMS[id('warp1_amount')], bank.get(id('warp1_amount'))),
        toPlain(PARAMS[id('warp2_mode')], bank.get(id('warp2_mode'))),
        toPlain(PARAMS[id('warp2_amount')], bank.get(id('warp2_amount'))),
      ];
      const key = `${mode}|${w}|${h}|${pos.toFixed(4)}|${t?.source}|${t?.count}|${warps.join(',')}|${preview ? previewKey : ''}`;
      if (key === drawnKey) return;
      drawnKey = key;
      stroke ||= getComputedStyle(c).getPropertyValue('--wt-color').trim() || '#4ea3ff';
      g.clearRect(0, 0, w, h);
      if (!t) {
        g.fillStyle = 'rgba(255,255,255,0.3)';
        g.font = `${11 * dpr}px Inter, sans-serif`;
        g.fillText('built-in saw', 8 * dpr, 16 * dpr);
        return;
      }
      if (mode === '3d') {
        const ox = w * 0.22;
        const oy = -h * 0.42;
        const fw = w * 0.72;
        const amp = h * 0.2;
        const x0 = w * 0.03;
        const y0 = h * 0.72;
        const shown = Math.min(t.count, MAX_SHOWN);
        if (!layer || layer.width !== w || layer.height !== h) {
          layer = document.createElement('canvas');
          layer.width = w;
          layer.height = h;
          const lg = layer.getContext('2d')!;
          lg.lineWidth = dpr;
          for (let k = shown - 1; k >= 0; k--) {
            const f = shown > 1 ? Math.round((k / (shown - 1)) * (t.count - 1)) : 0;
            const d = shown > 1 ? k / (shown - 1) : 0;
            lg.strokeStyle = `rgba(200,210,225,${0.12 + 0.18 * (1 - d)})`;
            lg.beginPath();
            for (let i = 0; i <= POINTS; i++) {
              const v = t.frames[f * CONST.frameLen + Math.min(CONST.frameLen - 1, Math.floor((i / POINTS) * CONST.frameLen))];
              const x = x0 + (i / POINTS) * fw + d * ox;
              const y = y0 - v * amp + d * oy;
              if (i) lg.lineTo(x, y);
              else lg.moveTo(x, y);
            }
            lg.stroke();
          }
        }
        g.drawImage(layer, 0, 0);
        const cur = frameAt(t, pos, scratch);
        const d = pos;
        g.strokeStyle = stroke;
        g.lineWidth = 2 * dpr;
        g.beginPath();
        for (let i = 0; i <= POINTS; i++) {
          const v = cur[Math.min(CONST.frameLen - 1, Math.floor((i / POINTS) * CONST.frameLen))];
          const x = x0 + (i / POINTS) * fw + d * ox;
          const y = y0 - v * amp + d * oy;
          if (i) g.lineTo(x, y);
          else g.moveTo(x, y);
        }
        g.stroke();
      } else {
        // 2D: the current frame through the warps (computed by the tools worker)
        const want = `${t.source}|${t.count}|${pos.toFixed(3)}|${warps.join(',')}|${remapVer}`;
        if (want !== previewKey && !previewBusy) {
          previewBusy = true;
          const frame = frameAt(t, pos, new Float32Array(CONST.frameLen));
          const remap = synth.remap.lut(osc).slice();
          tools()
            .call({ op: 'preview', frame, w1: [warps[0], warps[1]], w2: [warps[2], warps[3]], points: 256, remap }, [frame.buffer, remap.buffer])
            .then((p) => {
              preview = p;
              previewKey = want;
            })
            .finally(() => (previewBusy = false));
        }
        g.strokeStyle = 'rgba(255,255,255,0.08)';
        g.lineWidth = dpr;
        g.beginPath();
        g.moveTo(0, h / 2);
        g.lineTo(w, h / 2);
        g.stroke();
        if (preview) {
          g.strokeStyle = stroke;
          g.lineWidth = 2 * dpr;
          g.beginPath();
          for (let i = 0; i < preview.length; i++) {
            const x = (i / (preview.length - 1)) * w;
            const y = h / 2 - preview[i] * h * 0.42;
            if (i) g.lineTo(x, y);
            else g.moveTo(x, y);
          }
          g.stroke();
        }
      }
      g.fillStyle = 'rgba(230,233,238,0.55)';
      g.font = `${10 * dpr}px "JetBrains Mono", monospace`;
      g.fillText(`${Math.round(pos * (t.count - 1)) + 1}/${t.count}`, w - 44 * dpr, 13 * dpr);
    });
    return () => {
      off();
      offTables();
      offRemap();
    };
  });
</script>

<button class="wt" style:--wt-color={color} onclick={() => (mode = mode === '3d' ? '2d' : '3d')} aria-label={`Osc ${letter.toUpperCase()} wavetable display (${mode}); click to switch`}>
  <canvas bind:this={canvas}></canvas>
  <span class="mode">{mode.toUpperCase()}</span>
</button>

<style>
  .wt {
    position: relative;
    display: block;
    width: 100%;
    height: 100%;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    background: var(--glass);
    cursor: pointer;
  }
  canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
  .mode {
    position: absolute;
    left: 6px;
    top: 4px;
    font: 9px var(--font-num);
    color: var(--text-faint);
  }
</style>
