<!--
  FLOW page: the whole signal path, live. Every node is a scope on its own
  tap (the newest voice for the per-voice stages, every voice for the
  buses), and the wires follow the routing: each source's route and sends,
  the filters in series or parallel, where the FX buses go. On the right,
  every modulation routing with the value it gives its destination now.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { SOURCES, TAP, TEL, TEL_COUNT, type TapName } from '../../gen/protocol';
  import { toPlain } from '../../state/param-math';
  import type { ModSlot } from '../../state/matrix';
  import { filterShares, routeTo } from '../../state/routing';
  import type { Synth } from '../../synth';
  import { sourceColor } from '../mod/sources';
  import { nav } from '../nav.svelte';
  import { fitCanvas, onFrame } from '../frame';

  const synth = getContext<Synth>('synth');

  interface Node {
    id: string;
    name: string;
    tap: TapName;
    x: number;
    y: number;
    color: string;
    explain: string;
    /** Where clicking the node goes. */
    page?: 'osc' | 'mix' | 'fx';
  }
  const COL = [0, 172, 344, 516, 688, 860];
  const NW = 150;
  const NH = 104;
  const NODES: Node[] = [
    { id: 'osc.a', name: 'OSC A', tap: 'focus.osc.a', x: COL[0], y: 0, color: 'var(--osc-a)', explain: 'osc.a', page: 'osc' },
    { id: 'osc.b', name: 'OSC B', tap: 'focus.osc.b', x: COL[0], y: 120, color: 'var(--osc-b)', explain: 'osc.b', page: 'osc' },
    { id: 'osc.c', name: 'OSC C', tap: 'focus.osc.c', x: COL[0], y: 240, color: 'var(--osc-c)', explain: 'osc.c', page: 'osc' },
    { id: 'sub', name: 'SUB', tap: 'focus.sub', x: COL[0], y: 360, color: 'var(--sub)', explain: 'sub', page: 'osc' },
    { id: 'noise', name: 'NOISE', tap: 'focus.noise', x: COL[0], y: 480, color: 'var(--noise)', explain: 'noise', page: 'osc' },
    { id: 'f1', name: 'FILTER 1', tap: 'focus.filter', x: COL[1], y: 120, color: 'var(--filter)', explain: 'filter.1', page: 'osc' },
    { id: 'f2', name: 'FILTER 2', tap: 'focus.filter2', x: COL[1], y: 360, color: 'var(--filter)', explain: 'filter.2', page: 'osc' },
    { id: 'amp', name: 'AMP · ENV 1', tap: 'focus.out', x: COL[2], y: 240, color: 'var(--env)', explain: 'flow.amp', page: 'osc' },
    { id: 'main', name: 'MAIN BUS', tap: 'bus.main', x: COL[3], y: 60, color: 'var(--text)', explain: 'flow.buses', page: 'mix' },
    { id: 'direct', name: 'DIRECT', tap: 'bus.direct', x: COL[3], y: 200, color: 'var(--text)', explain: 'flow.buses', page: 'mix' },
    { id: 'bus1', name: 'BUS 1', tap: 'bus.1', x: COL[3], y: 340, color: 'var(--fx)', explain: 'flow.buses', page: 'mix' },
    { id: 'bus2', name: 'BUS 2', tap: 'bus.2', x: COL[3], y: 480, color: 'var(--fx)', explain: 'flow.buses', page: 'mix' },
    { id: 'fxmain', name: 'MAIN FX', tap: 'fx.main', x: COL[4], y: 60, color: 'var(--fx)', explain: 'fx', page: 'fx' },
    { id: 'fx1', name: 'BUS 1 FX', tap: 'fx.bus1', x: COL[4], y: 340, color: 'var(--fx)', explain: 'fx', page: 'fx' },
    { id: 'fx2', name: 'BUS 2 FX', tap: 'fx.bus2', x: COL[4], y: 480, color: 'var(--fx)', explain: 'fx', page: 'fx' },
    { id: 'master', name: 'MASTER', tap: 'master.l', x: COL[5], y: 200, color: 'var(--text)', explain: 'master', page: 'mix' },
  ];
  const byId = Object.fromEntries(NODES.map((n) => [n.id, n])) as Record<string, Node>;
  const SRC = ['osc.a', 'osc.b', 'osc.c', 'sub', 'noise'];

  const plain = (key: string) => {
    const p = PARAMS[PARAM_ID[key as ParamKey]];
    return toPlain(p, synth.bank.get(p.id));
  };

  interface Wire {
    from: string;
    to: string;
    color: string;
    w: number;
    dashed?: boolean;
  }
  let wires = $state<Wire[]>([]);
  /** Nodes nothing flows through (dimmed). */
  let idle = $state<Record<string, boolean>>({});
  let levels = $state<Record<string, number>>({});
  let mods = $state<{ slot: number; source: string; color: string; dest: string; amount: number; value: number }[]>([]);
  let voices = $state(0);
  const canvases: Record<string, HTMLCanvasElement | undefined> = $state({});

  function layout(): void {
    const ws: Wire[] = [];
    const used = new Set<string>(['master']);
    const f1On = plain('filter.1.enable') >= 0.5;
    const f2On = plain('filter.2.enable') >= 0.5;
    const serial = plain('mix.filter_routing') < 0.5;
    let direct = false;
    for (const k of SRC) {
      if (plain(`${k}.enable`) < 0.5) continue;
      used.add(k);
      const lv = Math.min(1, plain(`${k}.level`));
      const color = byId[k].color;
      let to = routeTo(k, plain(`${k}.route`));
      if ((to === 'f1' || to === 'f2') && !f1On && !f2On) to = 'main'; // nothing to filter: straight on
      if (to === 'f1' || to === 'f2') {
        const [x1, x2] = filterShares(to, plain(`${k}.balance`));
        if (x1 > 0) ws.push({ from: k, to: 'f1', color, w: lv * x1 });
        if (x2 > 0) ws.push({ from: k, to: 'f2', color, w: lv * x2 });
        used.add('f1').add('f2');
      } else if (to === 'main' || to === 'direct') {
        ws.push({ from: k, to: 'amp', color, w: lv });
        if (to === 'direct') direct = true;
      }
      const s1 = plain(`${k}.send1`);
      const s2 = plain(`${k}.send2`);
      if (s1 > 0) ws.push({ from: k, to: 'bus1', color, w: lv * s1, dashed: true });
      if (s2 > 0) ws.push({ from: k, to: 'bus2', color, w: lv * s2, dashed: true });
      if (s1 > 0) used.add('bus1');
      if (s2 > 0) used.add('bus2');
    }
    const fc = 'var(--filter)';
    if (used.has('f1')) {
      if (serial) {
        ws.push({ from: 'f1', to: 'f2', color: fc, w: 0.7 });
        ws.push({ from: 'f2', to: 'amp', color: fc, w: 0.7 });
      } else {
        ws.push({ from: 'f1', to: 'amp', color: fc, w: 0.7 });
        ws.push({ from: 'f2', to: 'amp', color: fc, w: 0.7 });
      }
    }
    if (ws.some((w) => w.to === 'amp')) {
      used.add('amp').add('main');
      ws.push({ from: 'amp', to: 'main', color: 'var(--env)', w: 0.8 });
      if (direct) {
        used.add('direct');
        ws.push({ from: 'amp', to: 'direct', color: 'var(--env)', w: 0.5 });
      }
    }
    const tc = 'var(--text-dim)';
    ws.push({ from: 'main', to: 'fxmain', color: tc, w: 0.8 });
    ws.push({ from: 'fxmain', to: 'master', color: tc, w: 0.8 });
    if (direct) ws.push({ from: 'direct', to: 'master', color: tc, w: 0.6 });
    for (const [b, fx, key] of [
      ['bus1', 'fx1', 'rack.bus1_to'],
      ['bus2', 'fx2', 'rack.bus2_to'],
    ] as const) {
      if (!used.has(b)) continue;
      used.add(fx);
      ws.push({ from: b, to: fx, color: 'var(--fx)', w: 0.7 });
      ws.push({ from: fx, to: plain(key) >= 0.5 ? 'master' : 'fxmain', color: 'var(--fx)', w: 0.6 });
    }
    used.add('fxmain');
    wires = ws;
    idle = Object.fromEntries(NODES.map((n) => [n.id, !used.has(n.id)]));
  }

  const WATCH = [
    ...SRC.flatMap((s) => ['enable', 'level', 'route', 'balance', 'send1', 'send2'].map((k) => `${s}.${k}`)),
    'filter.1.enable',
    'filter.2.enable',
    'mix.filter_routing',
    'rack.bus1_to',
    'rack.bus2_to',
  ];

  function readMods(): void {
    const t = synth.host?.tel;
    const out: typeof mods = [];
    synth.matrix.slots.forEach((s: ModSlot | null, i) => {
      if (!s || s.bypass) return;
      let value = synth.bank.get(s.dest);
      if (t) {
        for (let d = 0; d < TEL_COUNT.modDest; d++) {
          if (t[TEL.modDest + d] === s.dest) {
            value = t[TEL.modValue + d];
            break;
          }
        }
      }
      const name = SOURCES[s.source] as string;
      out.push({ slot: i, source: name, color: sourceColor(s.source), dest: PARAMS[s.dest]?.name ?? '?', amount: s.amount, value });
    });
    mods = out;
  }

  onMount(() => {
    layout();
    const offs = WATCH.filter((k) => PARAM_ID[k as ParamKey] !== undefined).map((k) => synth.bank.subscribe(PARAM_ID[k as ParamKey], layout));
    offs.push(synth.matrix.subscribe(readMods));
    const release = NODES.map((n) => synth.useTap(n.tap));
    const buf = new Float32Array(1024);
    const strokes: Record<string, string> = {};
    let last = 0;
    const stop = onFrame((now) => {
      const host = synth.host;
      if (!host) return;
      const lv: Record<string, number> = {};
      for (const n of NODES) {
        const c = canvases[n.id];
        const idx = TAP[n.tap];
        const heard = Math.min(host.heardFrame(), host.taps.latest(idx));
        const ok = host.taps.read(idx, heard, buf);
        let peak = 0;
        let e = 0;
        if (ok)
          for (let i = 0; i < buf.length; i++) {
            peak = Math.max(peak, Math.abs(buf[i]));
            e += buf[i] * buf[i];
          }
        lv[n.id] = ok ? Math.sqrt(e / buf.length) : 0;
        if (!c) continue;
        const g = c.getContext('2d');
        if (!g) continue;
        const { w, h, dpr } = fitCanvas(c);
        g.clearRect(0, 0, w, h);
        g.fillStyle = 'rgba(255,255,255,0.06)';
        g.fillRect(0, h / 2, w, dpr);
        if (!ok || peak < 1e-5) continue;
        // start at a rising zero crossing so a steady tone stands still; 512 frames across
        let t0 = 0;
        for (let i = 1; i < 512; i++)
          if (buf[i - 1] < 0 && buf[i] >= 0) {
            t0 = i;
            break;
          }
        const gain = 0.9 / peak;
        g.strokeStyle = strokes[n.id] ||= getComputedStyle(c).getPropertyValue('--c').trim() || '#e7e9ee';
        g.lineWidth = 1.25 * dpr;
        g.beginPath();
        for (let i = 0; i < 512; i++) {
          const x = (i / 511) * w;
          const y = h / 2 - buf[t0 + i] * gain * (h / 2);
          if (i) g.lineTo(x, y);
          else g.moveTo(x, y);
        }
        g.stroke();
      }
      if (now - last > 100) {
        last = now;
        levels = lv;
        voices = synth.telemetry().voicesActive;
        readMods();
      }
    });
    return () => {
      offs.forEach((f) => f());
      release.forEach((f) => f());
      stop();
    };
  });

  const db = (rms: number) => (rms > 1e-5 ? `${(20 * Math.log10(rms)).toFixed(0)} dB` : '—');
  const port = (n: Node, side: 'in' | 'out'): [number, number] => [side === 'in' ? n.x : n.x + NW, n.y + NH / 2];
  const curve = (w: Wire) => {
    const [x0, y0] = port(byId[w.from], 'out');
    const [x1, y1] = port(byId[w.to], 'in');
    const mx = (x0 + x1) / 2;
    return `M${x0} ${y0}C${mx} ${y0} ${mx} ${y1} ${x1} ${y1}`;
  };
</script>

<div class="flow-page">
  <section class="panel graph" data-explain="flow" aria-label="Signal flow">
    <div class="area">
      <svg class="wires" viewBox="0 0 1010 590" preserveAspectRatio="none" aria-hidden="true">
        {#each wires as w, i (i)}
          <path d={curve(w)} style:stroke={w.color} style:stroke-width={0.8 + 3 * w.w} style:opacity={0.3 + 0.6 * Math.min(1, w.w)} stroke-dasharray={w.dashed ? '5 4' : undefined} />
        {/each}
      </svg>
      {#each NODES as n (n.id)}
        <button
          class="node"
          class:idle={idle[n.id]}
          class:live={(levels[n.id] ?? 0) > 1e-4}
          style:left={`${n.x}px`}
          style:top={`${n.y}px`}
          style:--c={n.color}
          data-explain={n.explain}
          data-node={n.id}
          aria-label={`${n.name}: ${db(levels[n.id] ?? 0)}${idle[n.id] ? ', nothing routed here' : ''}`}
          onclick={() => n.page && (nav.page = n.page)}
        >
          <span class="head"><b>{n.name}</b><span class="lv">{db(levels[n.id] ?? 0)}</span></span>
          <span class="scope"><canvas bind:this={canvases[n.id]}></canvas></span>
        </button>
      {/each}
    </div>
    <p class="hint">
      The newest voice's oscillators, filters and amp, then every voice together on the buses, through the FX racks to the master. Dashed wires are sends. Click a node to edit it. {voices} voice{voices === 1 ? '' : 's'} playing.
    </p>
  </section>
  <section class="panel mods" data-explain="matrix" aria-label="Modulation now">
    <h2>MODULATION</h2>
    {#if mods.length}
      <ul>
        {#each mods as m (m.slot)}
          <li>
            <span class="src" style:color={m.color}>{m.source}</span>
            <span class="arrow">→</span>
            <span class="dest" title={m.dest}>{m.dest}</span>
            <span class="bar" title={`${Math.round(m.value * 100)}%`}><i style:width={`${Math.round(m.value * 100)}%`} style:background={m.color}></i></span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="hint">No routings. Drag a source (an envelope, an LFO, a macro) onto any knob, and it shows here with the value it gives.</p>
    {/if}
  </section>
</div>

<style>
  .flow-page {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 222px;
    gap: 8px;
    height: 100%;
  }
  .graph {
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
    gap: 6px;
  }
  .area {
    position: relative;
    width: 1010px;
    height: 590px;
  }
  .wires {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .wires path {
    fill: none;
  }
  .node {
    position: absolute;
    width: 150px;
    height: 104px;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 3px;
    padding: 5px 6px 6px;
    text-align: left;
    font: inherit;
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid color-mix(in srgb, var(--c) 45%, var(--line));
    border-radius: 6px;
    cursor: pointer;
  }
  .node.live {
    border-color: var(--c);
    box-shadow: 0 0 8px color-mix(in srgb, var(--c) 30%, transparent);
  }
  .node.idle {
    opacity: 0.35;
  }
  .head {
    display: flex;
    justify-content: space-between;
    font-size: 10px;
  }
  .head b {
    font-weight: 600;
    letter-spacing: 0.06em;
    color: var(--c);
  }
  .lv {
    font: 9.5px var(--font-num);
    color: var(--text-dim);
  }
  .scope {
    position: relative;
    min-height: 0;
    background: var(--glass);
    border-radius: 3px;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }
  .hint {
    margin: 0;
    font-size: 10.5px;
    color: var(--text-faint);
  }
  .mods {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
  }
  h2 {
    margin: 0;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.1em;
    color: var(--text-dim);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 3px;
    overflow: auto;
    min-height: 0;
  }
  li {
    display: grid;
    grid-template-columns: auto auto minmax(0, 1fr);
    grid-template-rows: auto 4px;
    column-gap: 4px;
    font-size: 10.5px;
  }
  .src {
    font-weight: 600;
  }
  .arrow {
    color: var(--text-faint);
  }
  .dest {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar {
    grid-column: 1 / -1;
    background: var(--glass);
    border-radius: 2px;
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
  }
</style>
