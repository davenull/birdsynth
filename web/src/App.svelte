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
  import Scope from './ui/primitives/Scope.svelte';
  import OscPanel from './ui/panels/OscPanel.svelte';
  import FilterPanel from './ui/panels/FilterPanel.svelte';
  import EnvPanel from './ui/panels/EnvPanel.svelte';
  import VoicingPanel from './ui/panels/VoicingPanel.svelte';
  import { onFrame } from './ui/frame';

  const synth = getContext<Synth>('synth');
  const W = 1280;
  const H = 800;

  let status = $state<SynthStatus>('idle');
  let qwerty = $state<QwertyState>({ octave: 4, velocity: 0.8 });
  let voices = $state(0);
  let cpu = $state(0);
  let scale = $state(1);
  let page = $state('osc');

  const PAGES = [
    { id: 'osc', name: 'OSC' },
    { id: 'mix', name: 'MIX', later: 'P2' },
    { id: 'fx', name: 'FX', later: 'P3' },
    { id: 'matrix', name: 'MATRIX', later: 'P2' },
    { id: 'global', name: 'GLOBAL', later: 'P4' },
  ];

  onMount(() => {
    const fit = () => (scale = Math.min(window.innerWidth / W, window.innerHeight / H));
    fit();
    window.addEventListener('resize', fit);
    const offStatus = synth.onStatus((s) => (status = s));
    const offKeys = installQwerty({ noteOn: (n, v) => synth.noteOn(n, v), noteOff: (n) => synth.noteOff(n) }, (s) => (qwerty = s));
    let t = 0;
    const offFrame = onFrame((now) => {
      if (now - t < 250) return;
      t = now;
      const tel = synth.telemetry();
      voices = tel.voicesActive;
      cpu = tel.cpuPct;
    });
    const unlock = () => synth.resume();
    window.addEventListener('pointerdown', unlock);
    window.addEventListener('keydown', unlock);
    return () => {
      window.removeEventListener('resize', fit);
      offStatus();
      offKeys();
      offFrame();
      window.removeEventListener('pointerdown', unlock);
      window.removeEventListener('keydown', unlock);
    };
  });
</script>

<div class="viewport">
  <div class="stage" style:width={`${W}px`} style:height={`${H}px`} style:transform={`scale(${scale})`}>
    <header class="topbar">
      <div class="brand">birdsynth</div>
      <nav class="tabs" aria-label="Pages">
        {#each PAGES as p (p.id)}
          <button class:on={page === p.id} disabled={!!p.later} title={p.later ? `Arrives in ${p.later}` : ''} onclick={() => (page = p.id)}>{p.name}</button>
        {/each}
      </nav>
      <div class="readout" aria-live="polite">
        <span>{voices} voice{voices === 1 ? '' : 's'}</span>
        <span>CPU {cpu.toFixed(1)}%</span>
        <span class="state {status}">{status}</span>
      </div>
      <div class="master" data-explain="master">
        <Meter {synth} />
        <Knob param="master.volume" size={30} color="var(--text)" />
      </div>
    </header>

    <main class="page">
      <OscPanel osc={0} />
      <OscPanel osc={1} />
      <OscPanel osc={2} />
      <FilterPanel n={1} />
    </main>

    <section class="strip" aria-label="Modulation and voicing">
      <EnvPanel />
      <VoicingPanel />
      <div class="panel scope"><Scope {synth} tap="focus.out" label="Newest voice" color="var(--osc-a)" /></div>
    </section>

    <footer class="keys">
      <div class="hint">
        Play with <kbd>A</kbd>–<kbd>'</kbd> (black keys <kbd>W</kbd> <kbd>E</kbd> <kbd>T</kbd> <kbd>Y</kbd> <kbd>U</kbd> <kbd>O</kbd> <kbd>P</kbd>) · octave <kbd>Z</kbd>/<kbd>X</kbd> (A = C{qwerty.octave}) · velocity <kbd>C</kbd>/<kbd>V</kbd> ({Math.round(qwerty.velocity * 100)}%) · drop a wavetable WAV on an oscillator
      </div>
      <Keyboard {synth} low={36} high={96} />
    </footer>
  </div>

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
    overflow: hidden;
  }
  .stage {
    flex: none;
    transform-origin: top center;
    display: grid;
    grid-template-rows: 44px 1fr 188px auto;
    gap: 8px;
    padding: 10px 12px 12px;
    box-sizing: border-box;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 18px;
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
    padding: 6px 10px;
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
  .readout {
    display: flex;
    gap: 14px;
    font: 11px var(--font-num);
    color: var(--text-dim);
    margin-left: auto;
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
  .page {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr 250px;
    gap: 8px;
    min-height: 0;
  }
  .strip {
    display: grid;
    grid-template-columns: 1.35fr 1fr 0.8fr;
    gap: 8px;
    min-height: 0;
  }
  .scope {
    display: grid;
  }
  .keys {
    display: grid;
    gap: 4px;
  }
  .hint {
    font-size: 10.5px;
    color: var(--text-dim);
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
