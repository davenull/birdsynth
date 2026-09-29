<!--
  Where every source goes: the filters (split by each source's balance),
  the main and direct buses, and the sends to the two FX buses. Line weight
  follows the level; nodes glow with the newest voice's signal.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import type { TapName } from '../../gen/protocol';
  import { toPlain } from '../../state/param-math';
  import type { Synth } from '../../synth';
  import { onFrame } from '../frame';

  const synth = getContext<Synth>('synth');
  const W = 1220;
  const H = 150;

  const SOURCES = [
    { key: 'osc.a', name: 'OSC A', color: 'var(--osc-a)', tap: 'focus.osc.a' },
    { key: 'osc.b', name: 'OSC B', color: 'var(--osc-b)', tap: 'focus.osc.b' },
    { key: 'osc.c', name: 'OSC C', color: 'var(--osc-c)', tap: 'focus.osc.c' },
    { key: 'sub', name: 'SUB', color: 'var(--sub)', tap: 'focus.sub' },
    { key: 'noise', name: 'NOISE', color: 'var(--noise)', tap: 'focus.noise' },
  ] as const;
  const NODE = {
    f1: { x: 470, y: 42, name: 'FILTER 1', tap: 'focus.filter' },
    f2: { x: 470, y: 108, name: 'FILTER 2', tap: 'focus.filter2' },
    main: { x: 800, y: 24, name: 'MAIN' },
    direct: { x: 800, y: 60, name: 'DIRECT' },
    bus1: { x: 800, y: 96, name: 'BUS 1' },
    bus2: { x: 800, y: 132, name: 'BUS 2' },
    out: { x: 1110, y: 75, name: 'MASTER', tap: 'focus.out' },
  } as const;
  const srcY = (i: number) => 15 + i * 30;

  const plain = (key: string) => {
    const p = PARAMS[PARAM_ID[key as ParamKey]];
    return toPlain(p, synth.bank.get(p.id));
  };

  interface Wire {
    from: [number, number];
    to: [number, number];
    color: string;
    w: number;
  }
  let wires = $state<Wire[]>([]);
  let off = $state<Record<string, boolean>>({});
  let glow = $state<Record<string, number>>({});

  function layout(): void {
    const ws: Wire[] = [];
    const f1On = plain('filter.1.enable') >= 0.5;
    const f2On = plain('filter.2.enable') >= 0.5;
    const serial = plain('mix.filter_routing') < 0.5;
    const o: Record<string, boolean> = { f1: !f1On, f2: !f2On };
    const L = (x: number, y: number): [number, number] => [x, y];
    SOURCES.forEach((s, i) => {
      const on = plain(`${s.key}.enable`) >= 0.5;
      o[s.key] = !on;
      if (!on) return;
      const lv = Math.min(1, plain(`${s.key}.level`));
      let route = plain(`${s.key}.route`);
      if (route === 0 && !f1On && !f2On) route = 1; // nothing to filter: straight to main
      const bal = plain(`${s.key}.balance`);
      const a = L(150, srcY(i));
      if (route === 0) {
        if (bal < 1) ws.push({ from: a, to: L(NODE.f1.x - 50, NODE.f1.y), color: s.color, w: lv * (1 - bal) });
        if (bal > 0) ws.push({ from: a, to: L(NODE.f2.x - 50, NODE.f2.y), color: s.color, w: lv * bal });
      } else if (route === 1) ws.push({ from: a, to: L(NODE.main.x - 44, NODE.main.y), color: s.color, w: lv });
      else if (route === 2) ws.push({ from: a, to: L(NODE.direct.x - 44, NODE.direct.y), color: s.color, w: lv });
      const s1 = plain(`${s.key}.send1`);
      const s2 = plain(`${s.key}.send2`);
      if (s1 > 0) ws.push({ from: a, to: L(NODE.bus1.x - 44, NODE.bus1.y), color: s.color, w: lv * s1 });
      if (s2 > 0) ws.push({ from: a, to: L(NODE.bus2.x - 44, NODE.bus2.y), color: s.color, w: lv * s2 });
    });
    const fc = 'var(--filter)';
    if (f1On || f2On) {
      // a filter that's off passes its input through (dim: nothing filtered on that wire),
      // but in series Filter 1's output carries on through Filter 2 to the main bus either way
      if (serial) ws.push({ from: L(NODE.f1.x + 50, NODE.f1.y), to: L(NODE.f2.x + 20, NODE.f2.y - 14), color: fc, w: f1On ? 0.8 : 0.3 });
      else ws.push({ from: L(NODE.f1.x + 50, NODE.f1.y), to: L(NODE.main.x - 44, NODE.main.y), color: fc, w: f1On ? 0.8 : 0.3 });
      ws.push({ from: L(NODE.f2.x + 50, NODE.f2.y), to: L(NODE.main.x - 44, NODE.main.y), color: fc, w: f2On || (serial && f1On) ? 0.8 : 0.3 });
    }
    for (const b of [NODE.main, NODE.direct, NODE.bus1, NODE.bus2]) ws.push({ from: L(b.x + 44, b.y), to: L(NODE.out.x - 50, NODE.out.y), color: 'var(--text-dim)', w: 0.6 });
    wires = ws;
    off = o;
  }

  const WATCH = [
    ...SOURCES.flatMap((s) => ['enable', 'level', 'route', 'balance', 'send1', 'send2'].map((k) => `${s.key}.${k}`)),
    'filter.1.enable',
    'filter.2.enable',
    'mix.filter_routing',
  ];

  onMount(() => {
    layout();
    const offs = WATCH.map((k) => synth.bank.subscribe(PARAM_ID[k as ParamKey], layout));
    const taps: TapName[] = [...SOURCES.map((s) => s.tap), NODE.f1.tap, NODE.f2.tap, NODE.out.tap];
    const release = taps.map((t) => synth.useTap(t));
    const buf = new Float32Array(1024);
    let last = 0;
    const stop = onFrame((now) => {
      if (now - last < 60) return;
      last = now;
      const g: Record<string, number> = {};
      for (const t of taps) {
        const v = synth.tap(t, buf.length);
        let e = 0;
        for (let i = 0; i < v.length; i++) e += v[i] * v[i];
        const rms = Math.sqrt(e / v.length);
        g[t] = rms > 1e-5 ? Math.max(0, 1 + Math.log10(rms) / 3) : 0; // -60 dB .. 0 dB -> 0..1
      }
      glow = g;
    });
    return () => {
      offs.forEach((f) => f());
      release.forEach((f) => f());
      stop();
    };
  });

  const curve = (w: Wire) => {
    const [x0, y0] = w.from;
    const [x1, y1] = w.to;
    const mx = (x0 + x1) / 2;
    return `M${x0} ${y0}C${mx} ${y0} ${mx} ${y1} ${x1} ${y1}`;
  };
</script>

<svg class="routing" viewBox={`0 0 ${W} ${H}`} width={W} height={H} role="img" aria-label="Signal routing">
  {#each wires as w, i (i)}
    <path class="wire" d={curve(w)} style:stroke={w.color} style:stroke-width={0.6 + 3.4 * w.w} style:opacity={0.25 + 0.6 * Math.min(1, w.w)} />
  {/each}
  {#each SOURCES as s, i (s.key)}
    <g class="node" class:off={off[s.key]} data-explain={`${s.key}`} transform={`translate(70 ${srcY(i)})`}>
      <rect x="-60" y="-11" width="120" height="22" rx="5" style:stroke={s.color} style:--glow={glow[s.tap] ?? 0} />
      <text text-anchor="middle" dy="4" style:fill={s.color}>{s.name}</text>
    </g>
  {/each}
  {#each ['f1', 'f2'] as k (k)}
    {@const n = NODE[k as 'f1' | 'f2']}
    <g class="node" class:off={off[k]} data-explain={k === 'f1' ? 'filter.1' : 'filter.2'} transform={`translate(${n.x} ${n.y})`}>
      <rect x="-50" y="-14" width="100" height="28" rx="6" style:stroke="var(--filter)" style:--glow={glow[n.tap] ?? 0} />
      <text text-anchor="middle" dy="4" style:fill="var(--filter)">{n.name}{off[k] ? ' (off)' : ''}</text>
    </g>
  {/each}
  {#each ['main', 'direct', 'bus1', 'bus2'] as k (k)}
    {@const n = NODE[k as 'main' | 'direct' | 'bus1' | 'bus2']}
    <g class="node bus" transform={`translate(${n.x} ${n.y})`}>
      <rect x="-44" y="-11" width="88" height="22" rx="5" />
      <text text-anchor="middle" dy="4">{n.name}</text>
    </g>
  {/each}
  <g class="node" data-explain="master" transform={`translate(${NODE.out.x} ${NODE.out.y})`}>
    <rect x="-50" y="-16" width="100" height="32" rx="6" style:stroke="var(--text)" style:--glow={glow[NODE.out.tap] ?? 0} />
    <text text-anchor="middle" dy="4">{NODE.out.name}</text>
  </g>
  <text class="note" x={NODE.bus2.x + 60} y={H - 4}>Main, Bus 1 and Bus 2 run through their FX racks on the way (the FLOW page shows every stage)</text>
</svg>

<style>
  .routing {
    display: block;
    width: 100%;
    height: auto;
  }
  .wire {
    fill: none;
  }
  .node rect {
    fill: color-mix(in srgb, var(--panel-2) calc(100% - var(--glow, 0) * 40%), #fff);
    stroke: var(--line);
    stroke-width: 1.5;
  }
  .node.off {
    opacity: 0.35;
  }
  .node text {
    font: 600 10px var(--font-ui);
    letter-spacing: 0.06em;
    fill: var(--text);
  }
  .bus rect {
    fill: var(--panel-2);
  }
  .note {
    font: 9.5px var(--font-ui);
    fill: var(--text-faint);
  }
</style>
