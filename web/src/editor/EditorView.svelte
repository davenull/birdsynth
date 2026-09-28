<!--
  The wavetable editor, in place of the page. Toolbar: pen or line, the
  grid and snap, the Process and Morph menus, frames (add, duplicate,
  remove, sort, reverse), importing a recording, exporting a WAV, undo and
  redo. Below: the frame to draw on, its harmonics, the formula bar, and a
  strip of every frame. Changes play straight away; the keyboard keeps
  working, so you can play while you edit.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { Synth } from '../synth';
  import { editorView } from './editor.svelte';
  import { IMPORT_MODES, MORPHS, PROCESS, type Scope } from './model';
  import FrameView from './FrameView.svelte';
  import BinsView from './BinsView.svelte';
  import FrameStrip from './FrameStrip.svelte';
  import { pick } from '../ui/browser/files';

  const synth = getContext<Synth>('synth');
  const ed = editorView.editor(synth.tables);
  const L = $derived('abc'[editorView.osc ?? 0]);
  const color = $derived(`var(--osc-${L})`);

  let version = $state(0);
  onMount(() => ed.subscribe(() => (version = ed.version)));
  // the model isn't reactive itself: `version` bumps when it changes
  const read = <T,>(fn: () => T) => () => {
    void version;
    return fn();
  };
  const count = $derived.by(read(() => ed.count));
  const current = $derived.by(read(() => ed.current));
  const nsel = $derived.by(read(() => ed.selected.size));
  const busy = $derived.by(read(() => ed.busy));
  const tableName = $derived.by(read(() => ed.name));
  const canUndo = $derived.by(read(() => ed.canUndo));
  const canRedo = $derived.by(read(() => ed.canRedo));

  interface Item {
    label: string;
    run: () => void;
  }

  let tool = $state<'pen' | 'line'>('pen');
  let gridX = $state(16);
  let gridY = $state(4);
  let snap = $state(false);
  let shown = $state(64);
  let scope = $state<Scope>('selection');
  let message = $state('');

  function say(m: string): void {
    message = m;
    setTimeout(() => {
      if (message === m) message = '';
    }, 4000);
  }

  async function guard(fn: () => Promise<unknown> | unknown, done?: string): Promise<void> {
    try {
      await fn();
      if (done) say(done);
    } catch (e) {
      say(String((e as Error).message ?? e));
    }
  }

  // --- a small form for operations that take settings
  interface Field {
    key: string;
    label: string;
    value: number;
    min: number;
    max: number;
    step?: number;
  }
  let form = $state<{ title: string; fields: Field[]; run: (v: Record<string, number>) => Promise<unknown> | unknown } | null>(null);
  function ask(title: string, fields: Field[], run: (v: Record<string, number>) => Promise<unknown> | unknown): void {
    form = { title, fields, run };
  }
  async function submit(): Promise<void> {
    if (!form) return;
    const v = Object.fromEntries(form.fields.map((f) => [f.key, Math.min(f.max, Math.max(f.min, Number(f.value)))]));
    const f = form;
    form = null;
    await guard(() => f.run(v), `${f.title}: done`);
  }

  const PROCESS_MENU: Item[] = [
    { label: 'Normalize each frame', run: () => guard(() => ed.process(PROCESS.normalizeEach, scope)) },
    { label: 'Normalize all together', run: () => guard(() => ed.process(PROCESS.normalizeAll, scope)) },
    { label: 'Remove DC offset', run: () => guard(() => ed.process(PROCESS.removeDc, scope)) },
    { label: 'Flip left to right', run: () => guard(() => ed.process(PROCESS.flipH, scope)) },
    { label: 'Flip upside down', run: () => guard(() => ed.process(PROCESS.flipV, scope)) },
    { label: 'Fade in across frames', run: () => guard(() => ed.process(PROCESS.fadeIn, scope)) },
    { label: 'Fade out across frames', run: () => guard(() => ed.process(PROCESS.fadeOut, scope)) },
    {
      label: 'Low-pass…',
      run: () =>
        ask(
          'Low-pass',
          [
            { key: 'cutoff', label: 'Cutoff harmonic', value: 16, min: 1, max: 1024 },
            { key: 'order', label: 'Slope (×6 dB/oct)', value: 2, min: 1, max: 8 },
          ],
          (v) => ed.process(PROCESS.lowPass, scope, v.cutoff, v.order),
        ),
    },
    {
      label: 'High-pass…',
      run: () =>
        ask(
          'High-pass',
          [
            { key: 'cutoff', label: 'Cutoff harmonic', value: 4, min: 1, max: 1024 },
            { key: 'order', label: 'Slope (×6 dB/oct)', value: 2, min: 1, max: 8 },
          ],
          (v) => ed.process(PROCESS.highPass, scope, v.cutoff, v.order),
        ),
    },
    { label: 'Downsample…', run: () => ask('Downsample', [{ key: 'factor', label: 'Hold each sample for', value: 8, min: 1, max: 512 }], (v) => ed.process(PROCESS.downsample, scope, v.factor)) },
    { label: 'Remove fundamental', run: () => guard(() => ed.process(PROCESS.removeFundamental, scope)) },
    { label: 'Blur spectrum…', run: () => ask('Blur spectrum', [{ key: 'width', label: 'Width (harmonics)', value: 2, min: 0.5, max: 64, step: 0.5 }], (v) => ed.process(PROCESS.blurSpectrum, scope, v.width)) },
    { label: 'Create PWM from this frame…', run: () => ask('Create PWM', [{ key: 'n', label: 'Frames', value: 64, min: 2, max: 256 }], (v) => ed.pwm(v.n)) },
  ];

  const HARMONIC_MENU: Item[] = [
    { label: 'Clear highs above…', run: () => ask('Clear highs', [{ key: 'k', label: 'Keep harmonics up to', value: 32, min: 1, max: 1024 }], (v) => ed.process(PROCESS.clearAbove, scope, v.k)) },
    { label: 'Clear lows below…', run: () => ask('Clear lows', [{ key: 'k', label: 'Remove harmonics below', value: 2, min: 1, max: 1024 }], (v) => ed.process(PROCESS.clearBelow, scope, v.k)) },
    { label: 'Randomize phases', run: () => guard(() => ed.process(PROCESS.randomizePhases, scope, Math.floor(Math.random() * 1e6))) },
    { label: 'Octave up', run: () => guard(() => ed.process(PROCESS.octaveUp, scope)) },
    { label: 'Octave down', run: () => guard(() => ed.process(PROCESS.octaveDown, scope)) },
    { label: 'Odd harmonics only', run: () => guard(() => ed.process(PROCESS.oddOnly, scope)) },
    { label: 'Even harmonics only', run: () => guard(() => ed.process(PROCESS.evenOnly, scope)) },
  ];

  function morph(mode: number): void {
    ask(`Morph: ${MORPHS[mode]}`, [{ key: 'n', label: 'Frames in the new table', value: 64, min: 2, max: 256 }], (v) => ed.morph(mode, v.n));
  }

  let menuOpen = $state<'process' | 'harmonics' | 'morph' | null>(null);
  const MENUS: { id: 'process' | 'harmonics'; label: string; items: Item[] }[] = [
    { id: 'process', label: 'Process', items: PROCESS_MENU },
    { id: 'harmonics', label: 'Harmonics', items: HARMONIC_MENU },
  ];

  // --- formula
  const EXAMPLES = [
    ['Saw to sine', 'lerp(saw(w), sin(w * tau), y)'],
    ['Pulse width sweep', 'pulse(w, 0.5 - 0.45 * y)'],
    ['Sync sweep', 'sin(w * tau * (1 + 7 * y))'],
    ['Sine fold', 'sin(x * pi * (1 + 5 * y))'],
    ['Soft to hard clip', 'tanh(x * (1 + 20 * y) * 3) / tanh((1 + 20 * y) * 3)'],
    ['Harmonic comb', 'sin(w * tau) + (y > 0.5 ? sin(3 * w * tau) / 3 : 0)'],
    ['Squash what’s there', 'sign(in) * abs(in) ^ (1 - 0.8 * y)'],
  ] as const;
  let src = $state('lerp(saw(w), sin(w * tau), y)');
  let check = $state<{ error: string | null; pos: number }>({ error: null, pos: 0 });
  let checkTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const s = src;
    if (checkTimer) clearTimeout(checkTimer);
    checkTimer = setTimeout(async () => (check = s.trim() ? await ed.checkFormula(s) : { error: null, pos: 0 }), 150);
  });
  async function runFormula(): Promise<void> {
    const r = await ed.formula(src, scope);
    if (r) check = r;
    else say(`Formula applied to ${scope === 'all' ? 'every frame' : 'the selected frames'}`);
  }

  // --- import
  let imp = $state<{ name: string; audio: Float32Array; sr: number; mode: number; period: number; fft: number; max: number; hz: number | null } | null>(null);
  async function chooseAudio(): Promise<void> {
    const [f] = await pick('audio/*,.wav,.aif,.aiff,.mp3,.flac,.ogg');
    if (!f) return;
    await guard(async () => {
      const ctx = new OfflineAudioContext(1, 1, 48_000);
      const buf = await ctx.decodeAudioData(await f.arrayBuffer());
      const n = buf.length;
      const audio = new Float32Array(n);
      for (let c = 0; c < buf.numberOfChannels; c++) {
        const d = buf.getChannelData(c);
        for (let i = 0; i < n; i++) audio[i] += d[i] / buf.numberOfChannels;
      }
      imp = { name: f.name.replace(/\.[^.]+$/, ''), audio, sr: buf.sampleRate, mode: IMPORT_MODES.constant, period: 2048, fft: 2048, max: 256, hz: null };
      await estimate();
    });
  }
  async function estimate(): Promise<void> {
    if (!imp) return;
    const hz = await ed.pitch(imp.audio, imp.sr);
    imp.hz = hz || null;
    if (hz) imp.period = Math.round((imp.sr / hz) * 1000) / 1000;
  }
  async function runImport(): Promise<void> {
    const i = imp;
    if (!i) return;
    imp = null;
    await guard(async () => {
      const arg = i.mode === IMPORT_MODES.fft ? i.fft : i.period;
      await ed.importAudio(i.audio, i.sr, i.mode, arg, i.max);
      ed.name = i.name;
    }, `Imported ${i.name}`);
  }
  const noteName = (hz: number) => {
    const n = 69 + 12 * Math.log2(hz / 440);
    const r = Math.round(n);
    return `${['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'][((r % 12) + 12) % 12]}${Math.floor(r / 12) - 1} ${n - r >= 0 ? '+' : ''}${Math.round((n - r) * 100)}¢`;
  };

  function exportWav(): void {
    const blob = synth.tables.exportWav(ed.osc);
    if (!blob) return;
    const a = document.createElement('a');
    a.href = URL.createObjectURL(blob);
    a.download = `${ed.name || 'wavetable'}.wav`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  }

  // the editor's own undo (Cmd/Ctrl+Z) while it's open
  onMount(() => {
    const key = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA')) return;
      if (!(e.metaKey || e.ctrlKey)) return;
      const k = e.key.toLowerCase();
      if (k === 'z' && !e.shiftKey) ed.undo();
      else if ((k === 'z' && e.shiftKey) || (k === 'y' && e.ctrlKey)) ed.redo();
      else if (k === 'a') ed.selectAll();
      else return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener('keydown', key, true);
    const away = (e: PointerEvent) => {
      if (!(e.target as Element).closest('.menu-wrap')) menuOpen = null;
    };
    window.addEventListener('pointerdown', away, true);
    return () => {
      window.removeEventListener('keydown', key, true);
      window.removeEventListener('pointerdown', away, true);
    };
  });
</script>

<section class="editor panel" aria-label={`Wavetable editor: Osc ${L.toUpperCase()}`} data-explain="editor" style:--accent={color}>
  <div class="toolbar">
    <b class="title">EDIT · OSC {L.toUpperCase()}</b>
    <input class="name" aria-label="Table name" value={tableName} oninput={(e) => (ed.name = e.currentTarget.value)} />
    <div class="seg" role="group" aria-label="Drawing tool">
      <button class:on={tool === 'pen'} onclick={() => (tool = 'pen')} title="Draw freehand">Pen</button>
      <button class:on={tool === 'line'} onclick={() => (tool = 'line')} title="Draw straight lines">Line</button>
    </div>
    <label class="small"
      >Grid <select bind:value={gridX} aria-label="Grid columns">{#each [0, 4, 8, 16, 32, 64] as g (g)}<option value={g}>{g || 'off'}</option>{/each}</select>
      × <select bind:value={gridY} aria-label="Grid rows">{#each [1, 2, 4, 8, 16] as g (g)}<option value={g}>{g}</option>{/each}</select></label
    >
    <label class="small"><input type="checkbox" bind:checked={snap} /> Snap</label>
    <span class="sep"></span>
    <label class="small"
      >Apply to <select bind:value={scope} aria-label="Apply operations to"><option value="selection">selected frames ({nsel})</option><option value="all">all {count} frames</option></select></label
    >
    {#each MENUS as { id, label, items } (id)}
      <div class="menu-wrap">
        <button aria-expanded={menuOpen === id} onclick={() => (menuOpen = menuOpen === id ? null : id)}>{label} ▾</button>
        {#if menuOpen === id}
          <div class="menu" role="menu">
            {#each items as { label: l, run } (l)}
              <button
                role="menuitem"
                onclick={() => {
                  menuOpen = null;
                  run();
                }}>{l}</button
              >
            {/each}
          </div>
        {/if}
      </div>
    {/each}
    <div class="menu-wrap">
      <button aria-expanded={menuOpen === 'morph'} onclick={() => (menuOpen = menuOpen === 'morph' ? null : 'morph')} title="Fill a new table between keyframes: the selected frames, or all of them">Morph ▾</button>
      {#if menuOpen === 'morph'}
        <div class="menu" role="menu">
          {#each MORPHS as m, i (m)}
            <button
              role="menuitem"
              onclick={() => {
                menuOpen = null;
                morph(i);
              }}>{m}…</button
            >
          {/each}
        </div>
      {/if}
    </div>
    <span class="sep"></span>
    <button onclick={() => ed.addFrame(true)} title="Copy this frame after itself" disabled={count >= 256}>Duplicate</button>
    <button onclick={() => ed.addFrame(false)} title="Add a silent frame after this one" disabled={count >= 256}>Add</button>
    <button onclick={() => ed.removeFrames()} disabled={count <= 1}>Remove</button>
    <button onclick={() => guard(() => ed.sortByBrightness())} title="Order the frames from darkest to brightest">Sort</button>
    <button onclick={() => ed.reverse()}>Reverse</button>
    <span class="sep"></span>
    <button onclick={chooseAudio} title="Make a table from a recording">Import audio…</button>
    <button onclick={exportWav}>Export WAV</button>
    <span class="sep"></span>
    <button class="icon" aria-label="Undo edit" disabled={!canUndo} onclick={() => ed.undo()}>↶</button>
    <button class="icon" aria-label="Redo edit" disabled={!canRedo} onclick={() => ed.redo()}>↷</button>
    <button class="done" onclick={() => editorView.close()}>Done</button>
  </div>

  {#if form}
    <div class="form" role="dialog" aria-label={form.title}>
      <b>{form.title}</b>
      {#each form.fields as f (f.key)}
        <label>{f.label} <input type="number" min={f.min} max={f.max} step={f.step ?? 1} bind:value={f.value} /></label>
      {/each}
      <button class="primary" onclick={submit}>Apply</button>
      <button onclick={() => (form = null)}>Cancel</button>
    </div>
  {/if}

  {#if imp}
    <div class="form" role="dialog" aria-label="Import audio">
      <b>Import “{imp.name}”</b>
      <span class="faint">{(imp.audio.length / imp.sr).toFixed(2)} s · {imp.hz ? `pitch ${imp.hz.toFixed(2)} Hz (${noteName(imp.hz)})` : 'no clear pitch'}</span>
      <select bind:value={imp.mode} aria-label="Import method">
        <option value={IMPORT_MODES.constant}>Cycles of a fixed length</option>
        <option value={IMPORT_MODES.dynamic}>Follow the pitch</option>
        <option value={IMPORT_MODES.dynamicSnap}>Follow the pitch, cut at zero crossings</option>
        <option value={IMPORT_MODES.fft}>Spectra of blocks (FFT)</option>
      </select>
      {#if imp.mode === IMPORT_MODES.constant}
        <label>Cycle length (samples) <input type="number" min="16" max="48000" step="0.001" bind:value={imp.period} /></label>
        <button onclick={estimate} title="Set the length to one period of the recording's pitch">From pitch</button>
      {:else if imp.mode === IMPORT_MODES.fft}
        <label>Block <select bind:value={imp.fft}>{#each [256, 512, 1024, 2048] as n (n)}<option value={n}>{n}</option>{/each}</select></label>
      {/if}
      <label>Up to <input type="number" min="1" max="256" bind:value={imp.max} /> frames</label>
      <button class="primary" onclick={runImport}>Import</button>
      <button onclick={() => (imp = null)}>Cancel</button>
    </div>
  {/if}

  <div class="views">
    <div class="frame">
      <div class="label">Frame {current + 1} of {count}{#if busy}<span class="busy"> · working…</span>{/if}</div>
      <FrameView {ed} {tool} {gridX} {gridY} {snap} {color} />
    </div>
    <div class="bins-wrap">
      <div class="label">
        Harmonics
        <select bind:value={shown} aria-label="Harmonics shown">{#each [16, 32, 64, 128, 256] as n (n)}<option value={n}>1–{n}</option>{/each}</select>
      </div>
      <BinsView {ed} {shown} {color} />
    </div>
  </div>

  <div class="formula">
    <span class="fx">f =</span>
    <div class="src">
      <input
        bind:value={src}
        aria-label="Formula"
        spellcheck="false"
        class:bad={!!check.error}
        onkeydown={(e) => {
          e.stopPropagation();
          if (e.key === 'Enter') void runFormula();
        }}
      />
      {#if check.error}<span class="caret" style:left={`calc(${check.pos}ch + 7px)`}>^</span>{/if}
    </div>
    <button class="primary" disabled={!!check.error || !src.trim()} onclick={runFormula}>Run</button>
    <select
      aria-label="Example formulas"
      onchange={(e) => {
        const v = e.currentTarget.value;
        if (v) src = v;
        e.currentTarget.value = '';
      }}
    >
      <option value="">Examples…</option>
      {#each EXAMPLES as [name, f] (name)}<option value={f}>{name}</option>{/each}
    </select>
    <span class="hint" class:err={!!check.error}
      >{check.error ? `${check.error} (at ${check.pos + 1})` : 'x: -1…1 across the cycle · w: 0…1 · y: 0…1 across the table · z: -1…1 · q: frame number · in: what’s there · sel · rand'}</span
    >
  </div>

  <FrameStrip {ed} {color} />
  {#if message}<p class="msg" role="status">{message}</p>{/if}
</section>

<style>
  .editor {
    height: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    position: relative;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }
  .title {
    font-size: 10px;
    letter-spacing: 0.1em;
    color: var(--accent);
    margin-right: 4px;
  }
  .name {
    width: 96px;
  }
  button,
  select,
  input {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 3px 7px;
  }
  button {
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    border-color: var(--text-faint);
  }
  button:disabled {
    color: var(--text-faint);
    cursor: default;
  }
  input[type='checkbox'] {
    padding: 0;
  }
  .seg {
    display: flex;
    gap: 2px;
  }
  .on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .small {
    display: flex;
    align-items: center;
    gap: 3px;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .sep {
    width: 1px;
    height: 18px;
    background: var(--line);
    margin: 0 3px;
  }
  .icon {
    width: 24px;
    padding: 3px 0;
  }
  .done {
    margin-left: auto;
    border-color: var(--accent);
  }
  .menu-wrap {
    position: relative;
  }
  .menu {
    position: absolute;
    top: 26px;
    left: 0;
    z-index: 30;
    min-width: 210px;
    display: grid;
    padding: 4px;
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.55);
  }
  .menu button {
    text-align: left;
    border: 0;
    background: none;
    padding: 5px 9px;
  }
  .menu button:hover {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
  }
  .form {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 6px 8px;
    background: var(--panel-2);
    border: 1px solid var(--accent);
    border-radius: 6px;
    font-size: 11px;
  }
  .form label {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-dim);
  }
  .form input[type='number'] {
    width: 80px;
  }
  .faint {
    color: var(--text-faint);
  }
  .primary {
    border-color: var(--accent);
  }
  .editor > :global(*) {
    min-width: 0;
  }
  .views {
    flex: 1;
    display: grid;
    grid-template-columns: minmax(0, 1.7fr) minmax(0, 1fr);
    gap: 8px;
    min-height: 0;
  }
  .frame,
  .bins-wrap {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 3px;
    min-height: 0;
  }
  .label {
    display: flex;
    align-items: center;
    gap: 6px;
    font: 600 10px var(--font-ui);
    letter-spacing: 0.06em;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .label select {
    text-transform: none;
    padding: 1px 4px;
  }
  .busy {
    color: var(--macro);
    text-transform: none;
    font-weight: 400;
  }
  .formula {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .fx {
    font: 12px var(--font-num);
    color: var(--text-dim);
  }
  .src {
    position: relative;
    flex: 0 1 460px;
    min-width: 200px;
  }
  .src input {
    width: 100%;
    font: 12px var(--font-num);
    padding: 4px 6px;
    background: var(--glass);
  }
  .src input.bad {
    border-color: var(--clip);
  }
  .caret {
    position: absolute;
    top: 21px;
    font: 12px var(--font-num);
    color: var(--clip);
    pointer-events: none;
  }
  .hint {
    font-size: 10.5px;
    color: var(--text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .hint.err {
    color: var(--clip);
  }
  .msg {
    position: absolute;
    right: 12px;
    bottom: 70px;
    margin: 0;
    padding: 3px 9px;
    font-size: 11px;
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
  }
</style>
