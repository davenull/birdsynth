<!--
  birdsynth faceplate. P0 shows the tracer bullet: Osc A, Env 1, the master
  section, a scope on the focused voice, and the keyboard.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { Synth, SynthStatus } from './synth';
  import { installQwerty, type QwertyState } from './input/qwerty';
  import Knob from './ui/primitives/Knob.svelte';
  import Keyboard from './ui/primitives/Keyboard.svelte';
  import Meter from './ui/primitives/Meter.svelte';
  import Scope from './ui/primitives/Scope.svelte';
  import { onFrame } from './ui/frame';

  const synth = getContext<Synth>('synth');

  let status = $state<SynthStatus>('idle');
  let qwerty = $state<QwertyState>({ octave: 4, velocity: 0.8 });
  let voices = $state(0);
  let cpu = $state(0);

  onMount(() => {
    const offStatus = synth.onStatus((s) => (status = s));
    const offKeys = installQwerty(
      { noteOn: (n, v) => synth.noteOn(n, v), noteOff: (n) => synth.noteOff(n) },
      (s) => (qwerty = s),
    );
    let t = 0;
    const offFrame = onFrame((now) => {
      if (now - t < 250) return;
      t = now;
      const tel = synth.telemetry();
      voices = tel.voicesActive;
      cpu = tel.cpuPct;
    });
    // any first gesture unlocks audio in browsers that start it suspended
    const unlock = () => synth.resume();
    window.addEventListener('pointerdown', unlock);
    window.addEventListener('keydown', unlock);
    return () => {
      offStatus();
      offKeys();
      offFrame();
      window.removeEventListener('pointerdown', unlock);
      window.removeEventListener('keydown', unlock);
    };
  });

  const noteName = (octave: number) => `C${octave}`;
</script>

<div class="app">
  <header class="topbar">
    <div class="brand">birdsynth</div>
    <div class="readout" aria-live="polite">
      <span>{voices} voice{voices === 1 ? '' : 's'}</span>
      <span>CPU {cpu.toFixed(1)}%</span>
      <span class="state {status}">{status}</span>
    </div>
    <div class="master" data-explain="master">
      <Meter {synth} />
      <Knob param="master.volume" size={36} color="var(--text)" />
    </div>
  </header>

  <main class="face">
    <section class="panel osc" data-explain="osc.a" aria-labelledby="osc-a-title">
      <h2 id="osc-a-title" style:color="var(--osc-a)">Osc A</h2>
      <div class="row">
        <Knob param="osc.a.level" color="var(--osc-a)" />
        <Knob param="osc.a.pan" color="var(--osc-a)" />
        <Knob param="osc.a.octave" color="var(--osc-a)" />
        <Knob param="osc.a.semi" color="var(--osc-a)" />
        <Knob param="osc.a.fine" color="var(--osc-a)" />
        <Knob param="osc.a.coarse" color="var(--osc-a)" />
      </div>
    </section>

    <section class="panel env" data-explain="env.1" aria-labelledby="env-1-title">
      <h2 id="env-1-title" style:color="var(--env)">Env 1 · amp</h2>
      <div class="row">
        <Knob param="env.1.attack" color="var(--env)" />
        <Knob param="env.1.hold" color="var(--env)" />
        <Knob param="env.1.decay" color="var(--env)" />
        <Knob param="env.1.sustain" color="var(--env)" />
        <Knob param="env.1.release" color="var(--env)" />
      </div>
    </section>

    <section class="panel voicing" data-explain="voice" aria-labelledby="voicing-title">
      <h2 id="voicing-title">Voicing</h2>
      <div class="row">
        <Knob param="voice.polyphony" color="var(--text-dim)" />
      </div>
    </section>

    <section class="panel scopes" aria-label="Scopes">
      <Scope {synth} tap="focus.osc" label="Osc A · newest voice" color="var(--osc-a)" />
      <Scope {synth} tap="master.l" label="Output (left)" color="var(--text)" />
    </section>
  </main>

  <footer class="keys">
    <div class="hint">
      Play with <kbd>A</kbd>–<kbd>'</kbd> (black keys <kbd>W</kbd> <kbd>E</kbd> <kbd>T</kbd> <kbd>Y</kbd> <kbd>U</kbd> <kbd>O</kbd> <kbd>P</kbd>) ·
      octave <kbd>Z</kbd>/<kbd>X</kbd> (A = {noteName(qwerty.octave)}) · velocity <kbd>C</kbd>/<kbd>V</kbd> ({Math.round(qwerty.velocity * 100)}%)
    </div>
    <Keyboard {synth} low={36} high={96} />
  </footer>

  {#if status === 'suspended' || status === 'idle'}
    <button class="unlock" onclick={() => synth.resume()}>Click or press a key to start audio</button>
  {:else if status === 'error'}
    <div class="unlock error" role="alert">The engine could not start: {synth.error}</div>
  {/if}
</div>

<style>
  .app {
    display: grid;
    grid-template-rows: auto 1fr auto;
    gap: 10px;
    max-width: 1180px;
    margin: 0 auto;
    padding: 12px 16px 16px;
    min-height: 100vh;
    box-sizing: border-box;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 6px 12px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
  }
  .brand {
    font-weight: 600;
    font-size: 18px;
    letter-spacing: 0.04em;
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
  .face {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 10px;
    align-content: start;
  }
  .panel {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 8px 10px 10px;
  }
  .panel h2 {
    margin: 0 0 6px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
  }
  .scopes {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    height: 170px;
  }
  .keys {
    display: grid;
    gap: 6px;
  }
  .hint {
    font-size: 11px;
    color: var(--text-dim);
  }
  kbd {
    font: 10px var(--font-num);
    padding: 0 4px;
    border: 1px solid var(--line);
    border-radius: 3px;
    background: var(--panel-2);
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
  @media (max-width: 640px) {
    .scopes {
      grid-template-columns: 1fr;
      height: 300px;
    }
  }
</style>
