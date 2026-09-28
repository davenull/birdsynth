<!--
  The sample editor, in place of the page: an oscillator's recording, big.
  Scroll to zoom (around the pointer), Shift-scroll to move along; drag the
  start, end and loop markers; double-click to add a slice, Alt-click one to
  remove it, or find them again at the transients. Root from pitch sets the
  root note to the recording's own pitch.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';
  import { TEL } from '../gen/protocol';
  import type { Synth } from '../synth';
  import { mono, type Recording } from '../state/recordings';
  import { toNorm, toPlain } from '../state/param-math';
  import { tools } from '../tools/client';
  import Knob from '../ui/primitives/Knob.svelte';
  import Select from '../ui/primitives/Select.svelte';
  import Toggle from '../ui/primitives/Toggle.svelte';
  import { fitCanvas, onFrame } from '../ui/frame';
  import { sampleEditor } from './sampler.svelte';

  const synth = getContext<Synth>('synth');
  const osc = $derived(sampleEditor.osc ?? 0);
  const L = $derived('abc'[osc]);
  const color = $derived(`var(--osc-${L})`);
  const k = (s: string) => `osc.${L}.${s}` as ParamKey;
  let canvas = $state<HTMLCanvasElement>();
  let rec = $state.raw<Recording | null>(null);
  let view = $state({ from: 0, to: 1 }); // the part shown, as shares of the recording
  let message = $state('');
  let version = $state(0);

  onMount(() => {
    const read = () => {
      rec = synth.recordings.osc[osc];
      version++;
    };
    read();
    const off = synth.recordings.subscribe((o) => o === osc && read());
    const ids = ['smp_start', 'smp_end', 'loop_start', 'loop_end', 'loop_mode', 'slice'].map((s) => PARAM_ID[k(s)]);
    const offs = ids.map((id) => synth.bank.subscribe(id, () => version++));
    let play = -2;
    const offFrame = onFrame(() => {
      const p = synth.host?.tel[TEL.oscPlay + osc] ?? -1;
      if (Math.abs(p - play) > 1e-5) {
        play = p;
        draw();
      }
    });
    return () => {
      off();
      offs.forEach((f) => f());
      offFrame();
    };
  });

  $effect(() => {
    void version;
    void view;
    draw();
  });

  const share = (key: string) => {
    const p = PARAMS[PARAM_ID[k(key)]];
    return toPlain(p, synth.bank.get(p.id));
  };
  const setShare = (key: string, v: number) => {
    const p = PARAMS[PARAM_ID[k(key)]];
    synth.bank.set(p.id, toNorm(p, Math.min(1, Math.max(0, v))));
  };

  function draw(): void {
    const c = canvas;
    const r = rec;
    if (!c) return;
    const g = c.getContext('2d');
    if (!g) return;
    const { w, h, dpr } = fitCanvas(c);
    g.clearRect(0, 0, w, h);
    if (!r) return;
    const n = r.channels[0].length;
    const a0 = Math.floor(view.from * n);
    const a1 = Math.ceil(view.to * n);
    const per = (a1 - a0) / w;
    const wave = getComputedStyle(c).getPropertyValue('--wave').trim() || '#4ea3ff';
    const lanes = r.channels.length;
    const lh = h / lanes;
    for (let ch = 0; ch < lanes; ch++) {
      const x = r.channels[ch];
      const mid = lh * ch + lh / 2;
      g.strokeStyle = 'rgba(255,255,255,0.08)';
      g.lineWidth = dpr;
      g.beginPath();
      g.moveTo(0, mid);
      g.lineTo(w, mid);
      g.stroke();
      g.fillStyle = wave;
      if (per > 2) {
        for (let px = 0; px < w; px++) {
          const s = a0 + Math.floor(px * per);
          const e = Math.min(n, a0 + Math.floor((px + 1) * per));
          let lo = 0;
          let hi = 0;
          for (let i = s; i < e; i++) {
            if (x[i] < lo) lo = x[i];
            if (x[i] > hi) hi = x[i];
          }
          g.fillRect(px, mid - hi * (lh / 2) * 0.95, 1, Math.max(dpr, (hi - lo) * (lh / 2) * 0.95));
        }
      } else {
        g.strokeStyle = wave;
        g.lineWidth = 1.5 * dpr;
        g.beginPath();
        for (let px = 0; px < w; px++) {
          const i = Math.min(n - 1, a0 + Math.floor(px * per));
          const y = mid - x[i] * (lh / 2) * 0.95;
          if (px) g.lineTo(px, y);
          else g.moveTo(px, y);
        }
        g.stroke();
      }
    }
    const X = (s: number) => ((s - view.from) / (view.to - view.from)) * w;
    g.fillStyle = 'rgba(0,0,0,0.5)';
    g.fillRect(0, 0, Math.max(0, X(share('smp_start'))), h);
    g.fillRect(X(share('smp_end')), 0, w, h);
    const line = (s: number, col: string, label: string) => {
      const x = X(s);
      if (x < -2 || x > w + 2) return;
      g.fillStyle = col;
      g.fillRect(x - dpr * 0.75, 0, 1.5 * dpr, h);
      g.font = `${9 * dpr}px Inter, sans-serif`;
      g.fillText(label, x + 3 * dpr, 11 * dpr);
    };
    line(share('smp_start'), '#e7e9ee', 'start');
    line(share('smp_end'), '#e7e9ee', 'end');
    if (share('loop_mode') > 0) {
      line(share('loop_start'), '#2ecc71', 'loop');
      line(share('loop_end'), '#2ecc71', '');
    }
    r.slices.forEach((f, i) => line(f / n, 'rgba(255,209,102,0.85)', String(i + 1)));
    const p = synth.host?.tel[TEL.oscPlay + osc] ?? -1;
    if (p >= 0) line(p, '#ff6ec7', '');
  }

  // --- pointer: drag markers, add and remove slices
  let drag: { kind: 'param'; key: string } | { kind: 'slice'; i: number } | null = null;
  const at = (e: MouseEvent) => {
    const r = canvas!.getBoundingClientRect();
    return view.from + ((e.clientX - r.left) / r.width) * (view.to - view.from);
  };
  function nearest(e: MouseEvent): typeof drag {
    const r = rec;
    if (!r) return null;
    const s = at(e);
    const tol = ((view.to - view.from) * 8) / canvas!.getBoundingClientRect().width;
    const cands: [number, NonNullable<typeof drag>][] = [
      [share('smp_start'), { kind: 'param', key: 'smp_start' }],
      [share('smp_end'), { kind: 'param', key: 'smp_end' }],
    ];
    if (share('loop_mode') > 0) cands.push([share('loop_start'), { kind: 'param', key: 'loop_start' }], [share('loop_end'), { kind: 'param', key: 'loop_end' }]);
    r.slices.forEach((f, i) => cands.push([f / r.channels[0].length, { kind: 'slice', i }]));
    const best = cands.sort((a, b) => Math.abs(a[0] - s) - Math.abs(b[0] - s))[0];
    return best && Math.abs(best[0] - s) <= tol ? best[1] : null;
  }
  function onpointerdown(e: PointerEvent): void {
    if (e.button !== 0 || !rec) return;
    const hit = nearest(e);
    if (hit?.kind === 'slice' && e.altKey) {
      const s = rec.slices.filter((_, i) => i !== hit.i);
      void synth.recordings.setSlices(osc, s);
      return;
    }
    drag = hit;
    if (drag) canvas!.setPointerCapture(e.pointerId);
  }
  function onpointermove(e: PointerEvent): void {
    if (!drag || !rec || !canvas!.hasPointerCapture(e.pointerId)) return;
    const s = Math.min(1, Math.max(0, at(e)));
    if (drag.kind === 'param') setShare(drag.key, s);
    else {
      rec.slices[drag.i] = s * rec.channels[0].length;
      version++;
    }
  }
  function onpointerup(): void {
    if (drag?.kind === 'slice' && rec) void synth.recordings.setSlices(osc, rec.slices);
    drag = null;
  }
  function ondblclick(e: MouseEvent): void {
    if (!rec) return;
    const f = Math.round(at(e) * rec.channels[0].length);
    void synth.recordings.setSlices(osc, [...rec.slices, f]);
  }
  function onwheel(e: WheelEvent): void {
    e.preventDefault();
    const span = view.to - view.from;
    if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      const d = ((e.shiftKey ? e.deltaY : e.deltaX) / 500) * span;
      const from = Math.min(1 - span, Math.max(0, view.from + d));
      view = { from, to: from + span };
      return;
    }
    const pivot = at(e);
    const z = Math.exp(e.deltaY / 300);
    const next = Math.min(1, Math.max(64 / (rec?.channels[0].length ?? 64), span * z));
    let from = pivot - ((pivot - view.from) / span) * next;
    from = Math.min(1 - next, Math.max(0, from));
    view = { from, to: from + next };
  }

  async function detect(): Promise<void> {
    const r = rec;
    if (!r) return;
    const found = await tools().call({ op: 'onsets', audio: mono(r), sr: r.rate });
    await synth.recordings.setSlices(osc, [0, ...[...found].filter((f) => f > r.rate * 0.02)]);
    message = `${found.length + 1} slices`;
  }

  async function rootFromPitch(): Promise<void> {
    const r = rec;
    if (!r) return;
    const s = Math.floor(share('smp_start') * r.channels[0].length);
    const hz = await tools().call({ op: 'pitch', audio: mono(r).subarray(s, s + Math.min(r.channels[0].length - s, r.rate * 2)), sr: r.rate });
    if (!hz) {
      message = 'No clear pitch in this recording';
      return;
    }
    const note = 69 + 12 * Math.log2(hz / 440);
    const root = PARAMS[PARAM_ID[k('root')]];
    synth.bank.set(root.id, toNorm(root, Math.round(note)));
    const cents = Math.round((note - Math.round(note)) * 100);
    const fine = PARAMS[PARAM_ID[k('fine')]];
    synth.bank.set(fine.id, toNorm(fine, -cents));
    message = `${hz.toFixed(2)} Hz: root ${Math.round(note)}, fine ${-cents} cents`;
  }

  async function transform(fn: (x: Float32Array) => Float32Array, what: string): Promise<void> {
    const r = rec;
    if (!r) return;
    await synth.recordings.set(osc, { ...r, channels: r.channels.map(fn) }, false);
    message = what;
  }
  const normalize = () => {
    const peak = rec ? Math.max(...rec.channels.map((c) => c.reduce((m, v) => Math.max(m, Math.abs(v)), 0))) : 0;
    if (peak > 0) void transform((x) => x.map((v) => v / peak), 'Normalized to full scale');
  };
  const reverse = () => {
    const n = rec?.channels[0].length ?? 0;
    const sl = rec?.slices ?? [];
    void transform((x) => x.slice().reverse(), 'Reversed').then(() => synth.recordings.setSlices(osc, sl.map((f) => n - f)));
  };
</script>

<section class="sampler panel" aria-label={`Sample editor: Osc ${L.toUpperCase()}`} data-explain="sampler" style:--accent={color}>
  <div class="bar">
    <b>SAMPLE · OSC {L.toUpperCase()}</b>
    <span class="name">{rec ? `${rec.name} · ${(rec.channels[0].length / rec.rate).toFixed(2)} s · ${rec.rate} Hz · ${rec.channels.length === 1 ? 'mono' : 'stereo'} · ${rec.slices.length} slices` : 'No recording on this oscillator: drop one on its panel'}</span>
    <button onclick={detect} disabled={!rec}>Find slices</button>
    <button onclick={rootFromPitch} disabled={!rec}>Root from pitch</button>
    <button onclick={normalize} disabled={!rec}>Normalize</button>
    <button onclick={reverse} disabled={!rec}>Reverse</button>
    <button onclick={() => (view = { from: 0, to: 1 })}>Show all</button>
    <button class="done" onclick={() => sampleEditor.close()}>Done</button>
  </div>
  <div class="wave">
    <canvas bind:this={canvas} style:--wave={color} aria-label="The recording: drag markers; double-click adds a slice, Alt-click removes one; scroll to zoom" {onpointerdown} {onpointermove} {onpointerup} {ondblclick} {onwheel}></canvas>
  </div>
  <div class="controls">
    <Knob param={k('smp_start')} size={30} {color} />
    <Knob param={k('smp_end')} size={30} {color} />
    <Knob param={k('loop_start')} size={30} {color} />
    <Knob param={k('loop_end')} size={30} {color} />
    <Knob param={k('xfade')} size={30} {color} />
    <Knob param={k('rate')} size={30} {color} />
    <Knob param={k('root')} size={30} {color} />
    <Knob param={k('fine')} size={30} {color} />
    <div class="opts">
      <Select param={k('loop_mode')} />
      <Toggle param={k('snap')} {color} />
      <Toggle param={k('keytrack')} {color} />
      <Toggle param={k('slice')} {color} />
      <Toggle param={k('tail')} {color} />
    </div>
    {#if message}<span class="msg" role="status">{message}</span>{/if}
  </div>
</section>

<style>
  .sampler {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    min-width: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .bar b {
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--accent);
  }
  .name {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  button {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 3px 8px;
    cursor: pointer;
  }
  button:disabled {
    color: var(--text-faint);
    cursor: default;
  }
  .done {
    border-color: var(--accent);
  }
  .wave {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: ew-resize;
    touch-action: none;
  }
  .controls {
    display: flex;
    align-items: end;
    gap: 6px;
    flex-wrap: wrap;
  }
  .opts {
    display: flex;
    gap: 5px;
    align-items: center;
    margin-left: 8px;
  }
  .msg {
    margin-left: auto;
    font-size: 11px;
    color: var(--text-dim);
  }
</style>
