<!-- GLOBAL page: quality, tempo and rate scaling, tuning, the keyboard, velocity, bend range, voices, MIDI and master. -->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID } from '../../gen/params';
  import { TEL } from '../../gen/protocol';
  import type { Synth } from '../../synth';
  import type { Mapping } from '../../input/learn';
  import type { ParamBank } from '../../state/bank';
  import { toPlain } from '../../state/param-math';
  import Knob from '../primitives/Knob.svelte';
  import Select from '../primitives/Select.svelte';
  import MidiButton from '../primitives/MidiButton.svelte';
  import Toggle from '../primitives/Toggle.svelte';
  import SourceHandle from '../mod/SourceHandle.svelte';
  import { SOURCE } from '../../state/matrix';
  import { pick } from '../browser/files';
  import { guardText } from '../../audio/guard';
  import { onFrame } from '../frame';

  const synth = getContext<Synth>('synth');
  const bank = getContext<ParamBank>('bank');
  let os = $state(1);
  let tuningName = $state('');
  let tuningError = $state('');
  let tuned = $state(false);
  let maps = $state<Mapping[]>([]);
  let velCurve = $state(0);
  let dropping = $state(false);
  let followClock = $state(synth.followClock);
  // linking with other tabs
  const linkState = () => ({ on: synth.link.on, linked: synth.link.linked, name: synth.link.name, nudge: synth.link.nudge, members: synth.link.members.map((m) => ({ ...m })) });
  let link = $state(linkState());
  const linkText = $derived(
    !link.on
      ? 'Plays in step with birdsynth in your other tabs: one transport, tempo and bar position for all of them, and any of them can start or stop the rest.'
      : !link.linked
        ? 'Linking once audio starts…'
        : link.members.length < 2
          ? 'Linked. Open birdsynth in another tab (with Link on there too) to play in step.'
          : `Linked with ${link.members.length - 1} other tab${link.members.length > 2 ? 's' : ''}.`,
  );
  // the session: whether it's kept, and a reset that asks first
  let session = $state({ state: synth.sessionState, saved: synth.sessionSaved });
  let now = $state(Date.now());
  let confirmReset = $state(false);
  let resetting = $state(false);
  const ago = (t: number) => {
    const s = Math.round((now - t) / 1000);
    return s < 5 ? 'just now' : s < 60 ? `${s} s ago` : s < 3600 ? `${Math.round(s / 60)} min ago` : new Date(t).toLocaleString();
  };
  const sessionText = $derived(
    session.state === 'on'
      ? `Your sound is kept in this browser as you work and comes back next time${session.saved ? ` (saved ${ago(session.saved)})` : ''}.`
      : session.state === 'memory'
        ? 'This browser isn’t keeping data for this site (a private window?), so the sound is lost on reload. Export presets you want to keep.'
        : session.state === 'failed'
          ? 'The last session couldn’t be read, so this one isn’t being kept (the old one is left as it was).'
          : 'Restoring your last session…',
  );
  async function reset(): Promise<void> {
    if (!confirmReset) {
      confirmReset = true;
      return;
    }
    confirmReset = false;
    resetting = true;
    try {
      await synth.resetSession();
    } finally {
      resetting = false;
    }
  }
  let guardOn = $state(synth.guardOn);
  let guard = $state(0);

  onMount(() => {
    const offFrame = onFrame(() => {
      const t = synth.host?.tel;
      if (t && t[TEL.oversample] !== os) os = t[TEL.oversample];
      if (t && t[TEL.guard] !== guard) guard = t[TEL.guard];
    });
    const readTuning = () => {
      tuningName = synth.tuning.name;
      tuned = synth.tuning.table !== null;
    };
    readTuning();
    const offTuning = synth.tuning.subscribe(readTuning);
    const readMaps = () => (maps = synth.learn.maps.map((m) => ({ ...m })));
    readMaps();
    const offLearn = synth.learn.subscribe(readMaps);
    const vc = PARAMS[PARAM_ID['voice.vel_curve']];
    velCurve = toPlain(vc, bank.get(vc.id));
    const offVel = bank.subscribe(vc.id, (v) => (velCurve = toPlain(vc, v)));
    const offSession = synth.onSession(() => (session = { state: synth.sessionState, saved: synth.sessionSaved }));
    const offLink = synth.link.subscribe(() => (link = linkState()));
    const tick = setInterval(() => (now = Date.now()), 5000);
    return () => {
      offFrame();
      offTuning();
      offLearn();
      offVel();
      offSession();
      offLink();
      clearInterval(tick);
    };
  });

  async function loadTuning(files: File[]): Promise<void> {
    if (!files.length) return;
    try {
      await synth.tuning.load(files);
      tuningError = '';
    } catch (e) {
      tuningError = (e as Error).message;
    }
  }

  // the curve the engine applies: velocity ^ 4^-curve
  const velPath = $derived.by(() => {
    const k = 4 ** -velCurve;
    let d = '';
    for (let i = 0; i <= 32; i++) {
      const v = i / 32;
      d += `${i ? 'L' : 'M'}${(v * 60).toFixed(1)} ${(60 - v ** k * 60).toFixed(1)}`;
    }
    return d;
  });

  const color = 'var(--text-dim)';
</script>

<div class="global-page">
  <section class="panel" data-explain="global.quality" aria-label="Quality">
    <h2>QUALITY</h2>
    <Select param="global.quality" wide />
    <p>Oversampling runs only while a warp that needs it (sync, FM, PD, AM, RM) is in use. Now: <b>{os}×</b></p>
    <label class="check" title="When the engine takes over 70% of real time, turn oversampling off and then cap unison until the load falls again"
      ><input type="checkbox" checked={guardOn} onchange={(e) => synth.setGuardOn((guardOn = e.currentTarget.checked))} /> CPU guard{guard ? ` (now: ${guardText(guard)})` : ''}</label
    >
  </section>
  <section class="panel" data-explain="global.tempo" aria-label="Tempo">
    <h2>TEMPO</h2>
    <div class="row">
      <Knob param="global.bpm" size={34} {color} />
      <Knob param="global.env_rate" size={30} {color} />
      <Knob param="global.lfo_rate" size={30} {color} />
    </div>
    <p>BPM-synced LFOs and envelopes follow this tempo; the rate knobs speed every envelope or LFO up or down together.</p>
  </section>
  <section
    class="panel"
    class:dropping
    data-explain="tuning"
    aria-label="Tuning"
    ondragover={(e) => {
      e.preventDefault();
      dropping = true;
    }}
    ondragleave={() => (dropping = false)}
    ondrop={(e) => {
      e.preventDefault();
      dropping = false;
      void loadTuning([...(e.dataTransfer?.files ?? [])]);
    }}
  >
    <h2>TUNING</h2>
    <div class="row">
      <Knob param="global.tune" size={34} {color} />
      <div class="tuning">
        <div class="tname" title={tuningName}>{tuningName}</div>
        <div class="buttons">
          <button onclick={async () => loadTuning(await pick('.scl,.kbm,.tun', true))}>Load .scl / .kbm / .tun…</button>
          <button disabled={!tuned} onclick={() => synth.tuning.reset()}>Standard</button>
        </div>
      </div>
    </div>
    {#if tuningError}<p class="error" role="alert">{tuningError}</p>{:else}<p>Drop a Scala scale (with its keyboard map, if it has one) or an AnaMark .tun here. The tuning stays when you change presets.</p>{/if}
  </section>
  <section class="panel" data-explain="keys" aria-label="Keyboard">
    <h2>KEYBOARD</h2>
    <div class="row">
      <Knob param="keys.transpose" size={30} {color} />
      <Select param="keys.scale" wide />
      <Select param="keys.root" />
    </div>
    <p>Moves what you play, and with a scale, puts every key on its nearest note (the arpeggiator and clips play what comes out).</p>
  </section>
  <section class="panel" data-explain="voice.vel_curve" aria-label="Velocity">
    <h2>VELOCITY</h2>
    <div class="row">
      <Knob param="voice.vel_curve" size={30} {color} />
      <svg class="vel" viewBox="-2 -2 64 64" aria-label="Velocity curve: how hard you play (across) to the velocity the synth uses (up)">
        <rect x="0" y="0" width="60" height="60" />
        <path class="diag" d="M0 60L60 0" />
        <path class="curve" d={velPath} />
      </svg>
    </div>
  </section>
  <section class="panel" data-explain="voice.bend" aria-label="Pitch bend">
    <h2>PITCH BEND</h2>
    <div class="row">
      <Knob param="voice.bend_up" size={30} {color} />
      <Knob param="voice.bend_down" size={30} {color} />
    </div>
  </section>
  <section class="panel" data-explain="voice" aria-label="Voices">
    <h2>VOICES</h2>
    <div class="row">
      <Knob param="voice.polyphony" size={30} {color} />
      <Select param="voice.steal" wide />
    </div>
  </section>
  <section class="panel" data-explain="mpe" aria-label="MPE">
    <h2>MPE</h2>
    <div class="row">
      <Toggle param="voice.mpe" label="MPE" color="var(--mpe)" power />
      <Knob param="voice.mpe_range" size={30} color="var(--mpe)" />
      <SourceHandle source={SOURCE['MPE X']} label="X" />
      <SourceHandle source={SOURCE['MPE Y']} label="Y" />
      <SourceHandle source={SOURCE['MPE Z']} label="Z" />
    </div>
    <p>For MPE controllers: each note bends (X), slides (Y) and presses (Z) on its own. Drag X, Y or Z onto a knob.</p>
  </section>
  <section class="panel" data-explain="midi" aria-label="MIDI">
    <h2>MIDI</h2>
    <div class="row">
      <MidiButton {synth} />
      <button
        class="clock"
        aria-pressed={followClock}
        title="Follow an external MIDI clock: its tempo sets the BPM, and its start and stop run the transport"
        onclick={() => {
          followClock = !followClock;
          synth.setFollowClock(followClock);
        }}>Sync to MIDI clock</button
      >
    </div>
    {#if maps.length}
      <ul class="maps">
        {#each maps as m (m.cc)}
          <li>
            <span class="cc">CC {m.cc}</span>
            <span class="pname">{PARAMS[PARAM_ID[m.param]].name}</span>
            <button aria-label={`Forget CC ${m.cc}`} onclick={() => synth.learn.clear(m.param)}>×</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p>Right-click any knob and choose MIDI learn, then move a control on your controller.</p>
    {/if}
  </section>
  <section class="panel" data-explain="link" aria-label="Link">
    <h2>LINK</h2>
    <div class="row">
      <button class="clock" aria-pressed={link.on} onclick={() => synth.link.setOn(!link.on)}>Link tabs</button>
      <input class="lname" value={link.name} aria-label="This tab's name in the link" onchange={(e) => synth.link.setName(e.currentTarget.value)} />
    </div>
    <p role="status">{linkText}</p>
    {#if link.on}
      <label class="nudge" title="If this tab sounds early or late against the others (Bluetooth speakers, some embedded browsers delay the sound without saying so), move it until they line up by ear"
        >Nudge <input type="range" min="-300" max="300" step="1" value={link.nudge} aria-label="Nudge this tab later or earlier, in ms" oninput={(e) => synth.link.setNudge(Number(e.currentTarget.value))} /><span
          >{link.nudge > 0 ? `+${link.nudge}` : link.nudge} ms</span
        ></label
      >
    {/if}
    {#if link.members.length > 1}
      <ul class="members">
        {#each link.members as m (m.id)}
          <li class:self={m.self}>{m.name}{m.self ? ' (this tab)' : ''}{#if m.keeper}<span class="keeper" title="Keeps time for the group; if it closes, the next one takes over"> · keeps time</span>{/if}</li>
        {/each}
      </ul>
    {/if}
  </section>
  <section class="panel" data-explain="session" aria-label="Session">
    <h2>SESSION</h2>
    <p role="status">{sessionText}</p>
    <div class="row">
      <button class="danger" class:confirm={confirmReset} disabled={resetting} onclick={reset} title="Go back to the Init sound. Your saved presets and this browser's settings stay."
        >{resetting ? 'Resetting…' : confirmReset ? 'Really reset? Unsaved changes go' : 'Reset synth…'}</button
      >
      {#if confirmReset}<button onclick={() => (confirmReset = false)}>Keep my sound</button>{/if}
    </div>
  </section>
  <section class="panel" data-explain="master" aria-label="Master">
    <h2>MASTER</h2>
    <div class="row"><Knob param="master.volume" size={34} {color} /></div>
  </section>
</div>

<style>
  .global-page {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    grid-auto-rows: min-content;
    gap: 8px;
    align-content: start;
  }
  section {
    display: grid;
    gap: 8px;
    align-content: start;
  }
  section.dropping {
    border-color: var(--accent);
  }
  h2 {
    margin: 0;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.1em;
    color: var(--text-dim);
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: end;
  }
  p {
    margin: 0;
    font-size: 11px;
    color: var(--text-dim);
  }
  .error {
    color: var(--clip);
  }
  .check {
    display: flex;
    gap: 6px;
    align-items: center;
    font-size: 11px;
    color: var(--text-dim);
  }
  .tuning {
    display: grid;
    gap: 5px;
    min-width: 0;
    flex: 1;
  }
  .tname {
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .buttons {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
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
  .lname {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 3px 6px;
    min-width: 0;
    flex: 1;
  }
  .nudge {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) 48px;
    gap: 6px;
    align-items: center;
    font-size: 11px;
    color: var(--text-dim);
  }
  .nudge span {
    font: 11px var(--font-num);
    text-align: right;
  }
  .members {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .members .self {
    color: var(--text);
  }
  .keeper {
    color: var(--macro);
  }
  .danger.confirm {
    color: #fff;
    background: var(--clip);
    border-color: var(--clip);
  }
  .clock[aria-pressed='true'] {
    color: #111;
    background: var(--accent);
    border-color: var(--accent);
  }
  .vel {
    width: 64px;
    height: 64px;
  }
  .vel rect {
    fill: var(--glass);
    stroke: var(--line);
  }
  .vel .diag {
    stroke: var(--line);
    stroke-dasharray: 2 2;
  }
  .vel .curve {
    fill: none;
    stroke: var(--vel);
    stroke-width: 2;
  }
  .maps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
    font-size: 11px;
    max-height: 110px;
    overflow: auto;
  }
  .maps li {
    display: grid;
    grid-template-columns: 44px 1fr auto;
    gap: 6px;
    align-items: center;
  }
  .maps .cc {
    font-family: var(--font-num);
    color: var(--macro);
  }
  .maps .pname {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .maps button {
    padding: 0 6px;
  }
</style>
