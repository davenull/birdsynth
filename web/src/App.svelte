<!--
  birdsynth faceplate: a fixed 1280×800 panel scaled to fit the window,
  laid out like Serum. Top bar, page tabs, the page, the always-visible
  modulation strip, then the keyboard.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { Synth, SynthStatus } from './synth';
  import { installQwerty, type QwertyState } from './input/qwerty';
  import Knob from './ui/primitives/Knob.svelte';
  import Keyboard from './ui/primitives/Keyboard.svelte';
  import Meter from './ui/primitives/Meter.svelte';
  import EnvPanel from './ui/panels/EnvPanel.svelte';
  import LfoPanel from './ui/panels/LfoPanel.svelte';
  import MacroPanel from './ui/panels/MacroPanel.svelte';
  import VoicingPanel from './ui/panels/VoicingPanel.svelte';
  import SourceChips from './ui/mod/SourceChips.svelte';
  import OscPage from './ui/pages/OscPage.svelte';
  import MixPage from './ui/pages/MixPage.svelte';
  import MatrixPage from './ui/pages/MatrixPage.svelte';
  import GlobalPage from './ui/pages/GlobalPage.svelte';
  import FxPage from './ui/pages/FxPage.svelte';
  import ArpPage from './ui/pages/ArpPage.svelte';
  import ClipPage from './ui/pages/ClipPage.svelte';
  import FlowPage from './ui/pages/FlowPage.svelte';
  import PresetBar from './ui/browser/PresetBar.svelte';
  import Browser from './ui/browser/Browser.svelte';
  import { browse } from './ui/browser/browse.svelte';
  import ContextMenu from './ui/primitives/ContextMenu.svelte';
  import Wheels from './ui/primitives/Wheels.svelte';
  import MidiButton from './ui/primitives/MidiButton.svelte';
  import Overlay from './explain/Overlay.svelte';
  import { explainMode } from './explain/explain.svelte';
  import { nav, type PageId } from './ui/nav.svelte';
  import EditorView from './editor/EditorView.svelte';
  import { editorView } from './editor/editor.svelte';
  import SampleEditor from './sampler/SampleEditor.svelte';
  import { sampleEditor } from './sampler/sampler.svelte';
  import { onFrame } from './ui/frame';
  import { guardText } from './audio/guard';

  const synth = getContext<Synth>('synth');
  const SOURCE_URL = 'https://github.com/davenull/birdsynth';
  const W = 1280;
  const H = 800;

  let status = $state<SynthStatus>('idle');
  let qwerty = $state<QwertyState>({ octave: 4, velocity: 0.8 });
  let voices = $state(0);
  let cpu = $state(0);
  let scale = $state(1);
  let routings = $state(0);
  let guard = $state(0);

  // the modulation strip (envelopes, LFOs, macros, voicing) is on the OSC page only; the other pages and views get its room
  const main = $derived(nav.page === 'osc' && !browse.open && editorView.osc === null && sampleEditor.osc === null);

  const PAGES: { id: PageId; name: string; later?: string }[] = [
    { id: 'osc', name: 'OSC' },
    { id: 'mix', name: 'MIX' },
    { id: 'fx', name: 'FX' },
    { id: 'matrix', name: 'MATRIX' },
    { id: 'arp', name: 'ARP' },
    { id: 'clip', name: 'CLIP' },
    { id: 'global', name: 'GLOBAL' },
    { id: 'flow', name: 'FLOW' },
  ];

  onMount(() => {
    const fit = () => (scale = Math.min(window.innerWidth / W, window.innerHeight / H));
    fit();
    window.addEventListener('resize', fit);
    const offStatus = synth.onStatus((s) => (status = s));
    const offMatrix = synth.matrix.subscribe(() => (routings = synth.matrix.used));
    const offKeys = installQwerty({ noteOn: (n, v) => synth.noteOn(n, v), noteOff: (n) => synth.noteOff(n) }, (s) => (qwerty = s));
    let t = 0;
    const offFrame = onFrame((now) => {
      if (now - t < 250) return;
      t = now;
      const tel = synth.telemetry();
      voices = tel.voicesActive;
      cpu = tel.cpuPct;
      guard = synth.guard.level;
    });
    const unlock = () => synth.resume();
    window.addEventListener('pointerdown', unlock);
    window.addEventListener('keydown', unlock);
    return () => {
      window.removeEventListener('resize', fit);
      offStatus();
      offMatrix();
      offKeys();
      offFrame();
      window.removeEventListener('pointerdown', unlock);
      window.removeEventListener('keydown', unlock);
    };
  });
</script>

<div class="viewport">
  <div class="stage" class:full={!main} style:width={`${W}px`} style:height={`${H}px`} style:transform={`scale(${scale})`}>
    <header class="topbar">
      <div class="brand">birdsynth</div>
      <PresetBar />
      <nav class="tabs" aria-label="Pages">
        {#each PAGES as p (p.id)}
          <button
            class:on={nav.page === p.id && !browse.open && editorView.osc === null && sampleEditor.osc === null}
            disabled={!!p.later}
            title={p.later ? `Arrives in ${p.later}` : ''}
            aria-current={nav.page === p.id && !browse.open && editorView.osc === null && sampleEditor.osc === null ? 'page' : undefined}
            onclick={() => {
              nav.page = p.id;
              browse.open = false;
              editorView.close();
              sampleEditor.close();
            }}
            >{p.name}{#if p.id === 'matrix' && routings}<span class="badge">{routings}</span>{/if}</button
          >
        {/each}
      </nav>
      <MidiButton {synth} />
      <div class="aside">
        <button class="help explain-ui" aria-pressed={explainMode.on} title="Explain mode (?): click any part of the synth to learn what it does" onclick={() => explainMode.toggle()}>?</button>
        <a class="help source explain-ui" href={SOURCE_URL} target="_blank" rel="noopener" title="birdsynth's source code on GitHub (BSD 3-Clause)" aria-label="Source code on GitHub">
          <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
            ><path
              fill="currentColor"
              d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"
            /></svg
          >
        </a>
      </div>
      <div class="readout" aria-live="polite">
        <span class="state {status}" title={`Audio ${status}`} aria-label={`Audio ${status}`}>{status === 'running' ? '●' : status}</span>
        <!-- two short lines, so a busy moment (32 voices, CPU 100%) can't widen the bar -->
        <span class="counts">
          <span>{voices} voice{voices === 1 ? '' : 's'}</span>
          <span class:guarded={guard > 0} title={guard ? `CPU guard: ${guardText(guard)} while the load is high` : 'Share of real time the engine takes'}>CPU {cpu.toFixed(1)}%{guard ? ' ⚠' : ''}</span>
        </span>
      </div>
      <div class="master" data-explain="master">
        <Meter {synth} />
        <Knob param="master.volume" size={28} color="var(--text)" inline />
      </div>
    </header>

    <main class="page">
      {#if browse.open}
        <Browser />
      {:else if editorView.osc !== null}
        <EditorView />
      {:else if sampleEditor.osc !== null}
        <SampleEditor />
      {:else if nav.page === 'osc'}
        <OscPage />
      {:else if nav.page === 'mix'}
        <MixPage />
      {:else if nav.page === 'fx'}
        <FxPage />
      {:else if nav.page === 'matrix'}
        <MatrixPage />
      {:else if nav.page === 'arp'}
        <ArpPage />
      {:else if nav.page === 'clip'}
        <ClipPage />
      {:else if nav.page === 'global'}
        <GlobalPage />
      {:else if nav.page === 'flow'}
        <FlowPage />
      {/if}
    </main>

    {#if main}
      <section class="strip" aria-label="Modulation and voicing">
        <EnvPanel />
        <LfoPanel />
        <MacroPanel />
        <VoicingPanel />
      </section>
    {/if}

    <footer class="keys">
      <div class="hint-row">
        <SourceChips panels={!main} />
        {#if main}
          <div class="hint">
            Keys <kbd>A</kbd>–<kbd>'</kbd> · octave <kbd>Z</kbd>/<kbd>X</kbd> (A = C{qwerty.octave}) · velocity <kbd>C</kbd>/<kbd>V</kbd> ({Math.round(qwerty.velocity * 100)}%) · drag a handle onto a knob to modulate it
          </div>
        {/if}
      </div>
      <div class="play">
        <Wheels {synth} />
        <div class="kb"><Keyboard {synth} low={36} high={96} /></div>
      </div>
    </footer>
  </div>

  <ContextMenu />
  <Overlay />

  {#if status === 'suspended' || status === 'idle'}
    <button class="unlock" onclick={() => synth.resume()}>Click or press a key to start audio</button>
  {:else if status === 'error'}
    <div class="unlock error" role="alert">The engine could not start: {synth.error}</div>
  {/if}
</div>

<style>
  .viewport {
    position: fixed;
    inset: 0;
    display: flex;
    justify-content: center;
    /* clip, not hidden: a hidden box can still be scrolled by focus() or
       scrollIntoView, which would slide the faceplate sideways */
    overflow: clip;
  }
  .stage {
    flex: none;
    transform-origin: top center;
    display: grid;
    grid-template-rows: 44px minmax(0, 1fr) 200px auto;
    /* one column exactly the faceplate's width, whatever a page's content wants */
    grid-template-columns: minmax(0, 1fr);
    gap: 8px;
    padding: 10px 12px 12px;
    box-sizing: border-box;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
  }
  .brand {
    font-weight: 600;
    font-size: 18px;
    letter-spacing: 0.04em;
  }
  .tabs {
    display: flex;
    gap: 2px;
  }
  .tabs button {
    font: 600 11px var(--font-ui);
    letter-spacing: 0.08em;
    color: var(--text-dim);
    background: transparent;
    border: 0;
    border-bottom: 2px solid transparent;
    padding: 6px 5px;
    cursor: pointer;
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tabs button:disabled {
    color: var(--text-faint);
    cursor: default;
  }
  .help {
    flex: none;
    width: 22px;
    height: 22px;
    padding: 0;
    font: 600 12px var(--font-ui);
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 50%;
    cursor: pointer;
  }
  .aside {
    display: flex;
    gap: 4px;
  }
  .source {
    display: grid;
    place-items: center;
  }
  .source:hover {
    color: var(--text);
    border-color: var(--text-dim);
  }
  .help[aria-pressed='true'] {
    color: #111;
    background: var(--accent);
    border-color: var(--accent);
  }
  .readout {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    font: 10.5px/1.3 var(--font-num);
    color: var(--text-dim);
    margin-left: auto;
  }
  .counts {
    display: grid;
    min-width: 11ch;
  }
  .guarded {
    color: #ffb142;
  }
  .state.running {
    color: #2ecc71;
  }
  .state.error {
    color: var(--clip);
  }
  .master {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .stage.full {
    grid-template-rows: 44px minmax(0, 1fr) auto;
  }
  .page {
    min-height: 0;
    min-width: 0;
  }
  .strip {
    display: grid;
    grid-template-columns: 318px 1fr 236px 236px;
    gap: 8px;
    min-height: 0;
  }
  .badge {
    margin-left: 5px;
    padding: 0 5px;
    border-radius: 7px;
    font: 600 9px var(--font-num);
    color: #111;
    background: var(--lfo);
  }
  .hint-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .keys {
    display: grid;
    gap: 4px;
  }
  .play {
    display: flex;
    gap: 8px;
    align-items: stretch;
  }
  .kb {
    flex: 1;
    min-width: 0;
  }
  .hint {
    font-size: 10.5px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  kbd {
    font: 10px var(--font-num);
    padding: 0 4px;
    border: 1px solid var(--line);
    border-radius: 3px;
    background: var(--panel-2);
  }
  :global(.panel) {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 8px;
    min-width: 0;
    min-height: 0;
  }
  .unlock {
    position: fixed;
    left: 50%;
    top: 40%;
    transform: translate(-50%, -50%);
    padding: 14px 22px;
    font: inherit;
    font-size: 14px;
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--accent);
    border-radius: 8px;
    cursor: pointer;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.5);
  }
  .unlock.error {
    border-color: var(--clip);
    cursor: default;
    max-width: 80vw;
  }
</style>
