<!--
  ARP page: the arpeggiator (its settings and the step pattern it plays,
  lane by lane) and Voice Control (an eight-step sequence in every voice,
  feeding the Voice Mod 1 and 2 sources).
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
  import { TEL } from '../../gen/protocol';
  import type { Synth } from '../../synth';
  import { STEPS, type LaneName } from '../../state/seq';
  import { toNorm, toPlain } from '../../state/param-math';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import Bars from '../seq/Bars.svelte';
  import SourceHandle from '../mod/SourceHandle.svelte';
  import { SOURCE } from '../../state/matrix';
  import { onFrame } from '../frame';

  const synth = getContext<Synth>('synth');
  const color = 'var(--macro)';
  const vcColor = 'var(--misc)';
  const plain = (k: string) => toPlain(PARAMS[PARAM_ID[k as ParamKey]], synth.bank.get(PARAM_ID[k as ParamKey]));

  let bank = $state(0);
  let steps = $state(16);
  let step = $state(-1);
  let version = $state(0);
  let vc = $state<{ a: number[]; b: number[]; steps: number }>({ a: Array(8).fill(0), b: Array(8).fill(0), steps: 8 });

  onMount(() => {
    const read = () => {
      bank = plain('arp.bank') - 1;
      steps = plain('arp.steps');
    };
    read();
    const offs = [synth.bank.subscribe(PARAM_ID['arp.bank'], read), synth.bank.subscribe(PARAM_ID['arp.steps'], read), synth.arp.subscribe(() => version++)];
    const vcKeys = [...Array(8).keys()].flatMap((i) => [`vc.a${i + 1}`, `vc.b${i + 1}`]);
    const readVc = () => (vc = { a: [...Array(8).keys()].map((i) => plain(`vc.a${i + 1}`)), b: [...Array(8).keys()].map((i) => plain(`vc.b${i + 1}`)), steps: plain('vc.steps') });
    readVc();
    for (const k of [...vcKeys, 'vc.steps']) offs.push(synth.bank.subscribe(PARAM_ID[k as ParamKey], readVc));
    offs.push(
      onFrame(() => {
        const s = synth.host?.tel[TEL.arpStep] ?? -1;
        if (s !== step) step = s;
      }),
    );
    return () => offs.forEach((f) => f());
  });

  const lane = (name: LaneName) => {
    void version;
    return Array.from({ length: STEPS }, (_, s) => synth.arp.get(bank, name, s));
  };
  const setVc = (lane: 'a' | 'b', i: number, v: number) => {
    const p = PARAMS[PARAM_ID[`vc.${lane}${i + 1}` as ParamKey]];
    synth.bank.set(p.id, toNorm(p, v));
  };
</script>

<div class="arp-page">
  <section class="panel settings" data-explain="arp" aria-label="Arpeggiator">
    <header>
      <Toggle param="arp.enable" label="ARP" {color} power />
      <Toggle param="arp.hold" {color} />
    </header>
    <div class="row">
      <Select param="arp.shape" wide />
      <Select param="arp.rate" />
      <Select param="arp.retrigger" />
    </div>
    <div class="grid">
      <Knob param="arp.octaves" size={30} {color} />
      <Knob param="arp.shift" size={30} {color} />
      <Knob param="arp.gate" size={30} {color} />
      <Knob param="arp.chance" size={30} {color} />
      <Knob param="arp.repeats" size={30} {color} />
      <Knob param="arp.vel_ramp" size={30} {color} />
      <Knob param="arp.offset" size={30} {color} />
      <Knob param="arp.steps" size={30} {color} />
      <Knob param="arp.bank" size={30} {color} />
      <Knob param="global.swing" size={30} {color} />
      <Knob param="global.bpm" size={30} {color} />
    </div>
    <p class="hint">Hold keys to play the pattern. Each step's lanes shape its note; Pattern (the shape) picks notes by the Degree lane.</p>
  </section>

  <section class="panel lanes" data-explain="arp.lanes" aria-label={`Arp pattern ${bank + 1}`}>
    <div class="title">PATTERN {bank + 1} <span class="faint">· {steps} steps · step {step >= 0 ? step + 1 : '–'}</span></div>
    <div class="ons">
      <span class="label">STEP</span>
      <div class="buttons">
        {#each lane('on') as on, s (s)}
          <button class:on={on >= 0.5} class:now={s === step} class:off={s >= steps} aria-pressed={on >= 0.5} aria-label={`Step ${s + 1} ${on >= 0.5 ? 'on' : 'off'}`} onclick={() => synth.arp.set(bank, 'on', s, on >= 0.5 ? 0 : 1)}>{s + 1}</button>
        {/each}
      </div>
    </div>
    <Bars label="Velocity" values={lane('velocity')} active={step} count={steps} {color} height={96} onchange={(s, v) => synth.arp.set(bank, 'velocity', s, v)} format={(v) => `${Math.round(v * 100)}%`} />
    <Bars label="Gate" values={lane('gate')} active={step} count={steps} {color} height={96} onchange={(s, v) => synth.arp.set(bank, 'gate', s, v)} format={(v) => `${(v * 2).toFixed(2)}×`} />
    <Bars label="Chance" values={lane('chance')} active={step} count={steps} {color} height={80} onchange={(s, v) => synth.arp.set(bank, 'chance', s, v)} format={(v) => `${Math.round(v * 100)}%`} />
    <Bars label="Bend" values={lane('bend')} min={-12} max={12} bipolar active={step} count={steps} {color} height={96} onchange={(s, v) => synth.arp.set(bank, 'bend', s, Math.round(v))} format={(v) => `${v > 0 ? '+' : ''}${Math.round(v)} st`} />
    <Bars label="Strum" values={lane('strum')} active={step} count={steps} {color} height={64} onchange={(s, v) => synth.arp.set(bank, 'strum', s, v)} format={(v) => `${Math.round(v * 100)}%`} />
    <Bars label="Degree" values={lane('degree')} max={15} active={step} count={steps} {color} height={88} onchange={(s, v) => synth.arp.set(bank, 'degree', s, Math.round(v))} format={(v) => String(Math.round(v) + 1)} />
  </section>

  <section class="panel vc" data-explain="vc" aria-label="Voice Control">
    <header>
      <span class="title">VOICE CONTROL</span>
      <SourceHandle source={SOURCE['Voice Mod 1']} />
      <SourceHandle source={SOURCE['Voice Mod 2']} />
    </header>
    <div class="row">
      <Select param="vc.rate" />
      <Toggle param="vc.loop" color={vcColor} />
    </div>
    <div class="row">
      <Knob param="vc.steps" size={28} color={vcColor} />
      <Knob param="vc.smooth" size={28} color={vcColor} />
    </div>
    <Bars label="Mod 1" values={vc.a} min={-1} max={1} bipolar count={vc.steps} color={vcColor} height={140} onchange={(i, v) => setVc('a', i, v)} />
    <Bars label="Mod 2" values={vc.b} min={-1} max={1} bipolar count={vc.steps} color={vcColor} height={140} onchange={(i, v) => setVc('b', i, v)} />
    <p class="hint">Every note runs this sequence from its start. Drag the Voice Mod chips onto knobs to use it.</p>
  </section>
</div>

<style>
  .arp-page {
    display: grid;
    grid-template-columns: 300px minmax(0, 1fr) 260px;
    gap: 8px;
    height: 100%;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  header {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: end;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    justify-items: center;
    row-gap: 2px;
  }
  .title {
    font: 600 10px var(--font-ui);
    letter-spacing: 0.1em;
    color: var(--text-dim);
  }
  .faint {
    color: var(--text-faint);
    letter-spacing: 0;
    font-weight: 400;
  }
  .hint {
    margin: 0;
    font-size: 10.5px;
    color: var(--text-faint);
  }
  .ons {
    display: grid;
    grid-template-columns: 62px minmax(0, 1fr);
    align-items: center;
    gap: 6px;
  }
  .label {
    font: 600 9.5px var(--font-ui);
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .buttons {
    display: grid;
    grid-template-columns: repeat(16, 1fr);
    gap: 2px;
  }
  .buttons button {
    font: 9px var(--font-num);
    color: var(--text-faint);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 3px;
    padding: 3px 0;
    cursor: pointer;
  }
  .buttons button.on {
    color: #111;
    background: color-mix(in srgb, var(--macro) 70%, transparent);
    border-color: var(--macro);
  }
  .buttons button.now {
    outline: 2px solid #fff;
    outline-offset: -1px;
  }
  .buttons button.off {
    opacity: 0.35;
  }
</style>
