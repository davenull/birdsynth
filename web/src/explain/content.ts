// What explain mode says about each part of the synth. Every element with a
// data-explain key gets a callout: a parameter key uses that parameter's
// text from the spec (params/*.toml); a section key uses an entry here.
// Entries can name a live view (the tap to show, as a scope or spectrum)
// and "try this" changes, which go through the normal stores and so can be
// undone like any edit.

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';
import { SOURCE } from '../state/matrix';
import type { TapName } from '../gen/protocol';
import { FX_TYPES } from '../state/fx';
import { toNorm } from '../state/param-math';
import type { Synth } from '../synth';

export interface Try {
  label: string;
  run(s: Synth): void | Promise<void>;
}

export interface Explanation {
  title: string;
  text: string;
  view?: 'scope' | 'spectrum';
  tap?: TapName;
  tries?: Try[];
}

const set = (s: Synth, key: string, plain: number) => s.setParam(key as ParamKey, toNorm(PARAMS[PARAM_ID[key as ParamKey]], plain));
const setAll = (vals: Record<string, number>) => (s: Synth) => {
  for (const [k, v] of Object.entries(vals)) set(s, k, v);
};

const OSC_TEXT: Record<string, string> = {
  a: 'reads its wavetable',
  b: 'is a second wavetable oscillator; turn it on to layer or detune against A',
  c: 'is a third wavetable oscillator, handy for an octave layer or as a modulator for FM',
};

function osc(l: 'a' | 'b' | 'c'): Explanation {
  const O = l.toUpperCase();
  return {
    title: `Oscillator ${O}`,
    text: `Osc ${O} ${OSC_TEXT[l]}: a stack of single-cycle waves (frames) it can morph through with WT Position. Unison stacks up to 16 copies, detuned and spread across the stereo field. The two warp slots bend the wave's phase or shape before you hear it.`,
    view: 'scope',
    tap: `focus.osc.${l}` as TapName,
    tries: [
      { label: 'Make it a supersaw (7 voices)', run: setAll({ [`osc.${l}.enable`]: 1, [`osc.${l}.unison`]: 7, [`osc.${l}.detune`]: 0.3, [`osc.${l}.width`]: 1 }) },
      { label: 'Hard-sync it', run: setAll({ [`osc.${l}.enable`]: 1, [`osc.${l}.warp1_mode`]: 1, [`osc.${l}.warp1_amount`]: 0.45 }) },
    ],
  };
}

function filter(n: 1 | 2): Explanation {
  return {
    title: `Filter ${n}`,
    text:
      n === 1
        ? 'Filter 1 takes away (or boosts) part of the spectrum. Cutoff sets where, resonance adds a peak there, drive adds grit before it. The graph shows the response; the scope shows the focused voice after the filter.'
        : 'Filter 2 runs after Filter 1 (series) or beside it (parallel), set on the MIX page. Two filters make band-passes you can shape, or two characters to blend.',
    view: 'scope',
    tap: n === 1 ? 'focus.filter' : 'focus.filter2',
    tries: [{ label: 'Close it with resonance', run: setAll({ [`filter.${n}.enable`]: 1, [`filter.${n}.type`]: 1, [`filter.${n}.cutoff`]: 500, [`filter.${n}.res`]: 0.7 }) }],
  };
}

const SOURCE_NAMES: Record<string, string> = { 'osc.a': 'Osc A', 'osc.b': 'Osc B', 'osc.c': 'Osc C', sub: 'the sub oscillator', noise: 'the noise oscillator' };

function mix(key: string): Explanation {
  return {
    title: `${SOURCE_NAMES[key].replace(/^the /, '').replace(/^./, (c) => c.toUpperCase())} in the mix`,
    text: `Where ${SOURCE_NAMES[key]} goes: its level and pan, then Route (through the filters, straight to the amp, or nowhere), the balance between Filter 1 and 2, and sends to the two FX buses.`,
  };
}

const FX_TEXT: Record<string, [string, Record<string, number>?]> = {
  hyper: ['Hyper stacks detuned copies of the sound (like unison after the fact); Dimension widens it with short, modulated delays.'],
  distortion: ['Distortion bends the waveform, adding harmonics. It runs at 4× the sample rate, so the new harmonics above the audible range are filtered out before they can fold back down as aliasing.'],
  flanger: ['A flanger mixes in a copy delayed by a few milliseconds and sweeps the delay, so a comb of notches moves through the sound: the jet-plane whoosh.'],
  phaser: ['A phaser sweeps a chain of all-pass filters, making notches that move without the metallic comb of a flanger.'],
  chorus: ['Chorus adds a few slightly delayed, slowly wobbling copies, like several players who are nearly in tune.'],
  delay: ['Delay repeats the sound. Feedback sends repeats back in; the filter in the loop darkens or thins each one. Ping-pong bounces them left and right.'],
  compressor: ['A compressor turns loud parts down, so the sound is more even; multiband mode does that separately for lows, mids and highs, and can also lift quiet detail.'],
  reverb: ['Reverb imitates a space: thousands of reflections that blur into a tail. Size and decay set the room; damping darkens it the way air and soft walls do.'],
  eq: ['EQ lifts or cuts frequency bands. The low and high shelves tilt the ends, the peak works around one frequency.'],
  filter: ['The same filters as the voice has, placed on the mixed sound, so every note shares one sweep.'],
  bode: ['A frequency shifter moves every partial by the same number of hertz (not by the same ratio, as pitch shifting does), so harmonics stop lining up: a clangy, inharmonic sound. With feedback it makes endless-rising "barber pole" sweeps.'],
  convolve: ['Convolution puts the sound in a recorded (or here, generated) space or body: every sample triggers a copy of the impulse response. It runs with no added delay.'],
  utility: ['Gain, width, pan and a mono switch for the low end: the plumbing at the end of a chain.'],
  splitter: ['A splitter divides the sound into bands (low/high, low/mid/high) or mid/side, each with its own chain of effects, then adds them back together. The crossovers sum back flat.'],
};

function fx(key: string, index: number): Explanation {
  const t = FX_TYPES[index];
  return {
    title: t.name,
    text: FX_TEXT[key]?.[0] ?? '',
    view: 'spectrum',
    tap: 'master.l',
    tries: [
      {
        label: `Add a ${t.name.toLowerCase()} to the main rack`,
        run: (s) => {
          s.fx.add(0, index);
        },
      },
    ],
  };
}

export const CONTENT: Record<string, Explanation> = {
  presets: {
    title: 'Presets',
    text: 'The current preset. The arrows step through the browser’s results; the name opens the browser; * means you’ve changed it since loading or saving. Undo and redo cover every edit since the preset loaded.',
  },
  browser: {
    title: 'Preset browser',
    text: 'Search by name, author, tag or notes; narrow by category and rating; click a tag to require it, again to exclude it. Factory presets are read-only, so saving one makes a copy that is yours. Export writes one file with any wavetables and impulse responses the preset uses.',
  },
  master: {
    title: 'Master',
    text: 'The final level, and a meter that goes red when the output clips. The synth doesn’t limit its output, so big chords can clip; the factory presets are all levelled to about the same loudness.',
    view: 'scope',
    tap: 'master.l',
  },
  voice: {
    title: 'Voicing',
    text: 'How notes become voices: how many can sound at once, which one to take when you run out, mono and legato playing, and glide (portamento) between notes.',
    tries: [{ label: 'Mono with a slow glide', run: setAll({ 'voice.mono': 1, 'voice.legato': 1, 'voice.glide': 300 }) }],
  },
  'voice.bend': { title: 'Pitch bend range', text: 'How far the pitch wheel bends, up and down separately, in semitones.' },
  wheels: {
    title: 'Pitch and mod wheels',
    text: 'On-screen wheels for playing without a controller: pitch springs back to the middle, mod stays where you leave it. A connected MIDI controller’s wheels move them too. Drag the WHEEL or BEND chip onto a knob to use them as modulation sources.',
  },
  tuning: {
    title: 'Tuning',
    text: 'Master Tune moves everything (A4 = 440 Hz by default). A tuning file retunes each key: Scala .scl scales (with an optional .kbm keyboard map) or AnaMark .tun files. Tuning belongs to your setup, not a preset, so it stays when presets change.',
    tries: [
      {
        label: 'Try just intonation (C major)',
        run: async (s) => {
          await s.tuning.load([{ name: 'just.scl', text: async () => 'Just intonation, 5-limit\n12\n16/15\n9/8\n6/5\n5/4\n4/3\n45/32\n3/2\n8/5\n5/3\n9/5\n15/8\n2/1\n' }]);
        },
      },
      { label: 'Back to standard tuning', run: (s) => s.tuning.reset() },
    ],
  },
  midi: {
    title: 'MIDI',
    text: 'Plays from any MIDI controller in Chrome, Edge and Firefox (Safari has no Web MIDI). MIDI learn ties a controller knob to any parameter: right-click a knob, choose MIDI learn, move the control. Mappings stay across presets and reloads.',
  },
  sub: {
    title: 'Sub oscillator',
    text: 'A simple wave (sine, triangle, saw or square) one or two octaves down, to give a sound weight without muddying the main oscillators. It can skip the filters (Direct) to keep the low end clean.',
    view: 'scope',
    tap: 'focus.sub',
    tries: [{ label: 'Add a sub an octave down', run: setAll({ 'sub.enable': 1, 'sub.octave': -1, 'sub.level': 0.6 }) }],
  },
  noise: {
    title: 'Noise oscillator',
    text: 'Plays one of the built-in noise recordings (all generated here): white and coloured noise, crackle, air, textures. Keytrack plays it faster for higher notes; one-shot plays it once, for attacks.',
    view: 'spectrum',
    tap: 'focus.noise',
    tries: [{ label: 'Add some breath', run: setAll({ 'noise.enable': 1, 'noise.level': 0.3 }) }],
  },
  sources: {
    title: 'Modulation sources',
    text: 'Everything that can move a knob on its own: velocity, the note, the wheels, aftertouch, randoms per note, and more. Drag a chip onto any knob to route it; the ring on the knob shows the range it moves through.',
  },
  env: {
    title: 'Envelopes',
    text: 'Four envelopes shape a value over the life of a note: attack, hold, decay, sustain, release. Env 1 is always the volume; the others can be dragged onto anything. The curves bend each stage.',
    view: 'scope',
    tap: 'focus.out',
    tries: [
      { label: 'Make it a pluck', run: setAll({ 'env.1.attack': 1, 'env.1.decay': 350, 'env.1.sustain': 0, 'env.1.release': 300 }) },
      { label: 'Make it swell', run: setAll({ 'env.1.attack': 900, 'env.1.sustain': 1, 'env.1.release': 1500 }) },
    ],
  },
  lfo: {
    title: 'LFOs',
    text: 'Ten low-frequency oscillators: repeating shapes you draw, XY paths, chaotic attractors or random steps. Sync them to the tempo, restart them with each note, or run one shared LFO for every voice (turn Poly off).',
    tries: [
      {
        label: 'Wobble the filter with LFO 1',
        run: (s) => {
          setAll({ 'filter.1.enable': 1, 'filter.1.cutoff': 400, 'filter.1.res': 0.4, 'lfo.1.bpm': 1, 'lfo.1.sync_rate': 6 })(s);
          s.matrix.add(SOURCE['LFO 1'], PARAM_ID['filter.1.cutoff'], 0.5);
        },
      },
    ],
  },
  macro: {
    title: 'Macros',
    text: 'Eight knobs that do nothing on their own: route a macro to several parameters (in the matrix, or by dragging its chip) and one turn moves them all together. Presets use them for “brightness”, “space” and so on.',
    tries: [
      {
        label: 'Tie Macro 1 to cutoff and drive',
        run: (s) => {
          set(s, 'filter.1.enable', 1);
          s.matrix.add(SOURCE['Macro 1'], PARAM_ID['filter.1.cutoff'], 0.6);
          s.matrix.add(SOURCE['Macro 1'], PARAM_ID['filter.1.drive'], 0.5);
        },
      },
    ],
  },
  matrix: {
    title: 'Modulation matrix',
    text: 'Every routing in one table: source, amount, destination, plus a curve, an aux source that scales it (for example velocity scaling an envelope), and bypass. Up to 64 routings.',
  },
  mix: {
    title: 'Mixer',
    text: 'Each source’s level, pan and route. The diagram shows the signal path: sources into the two filters (in series or in parallel), then the amp, then the FX racks.',
  },
  'mix.filter_routing': {
    title: 'Filter routing',
    text: 'Series: Filter 1 feeds Filter 2, so their effects multiply (two low-passes make a steeper slope). Parallel: each source is split between them by its balance knob and the results are added.',
  },
  fx: {
    title: 'Effects',
    text: 'Three racks: Main, and two buses that sources can send to. Add as many effects as you like, reorder them by dragging, bypass any of them. Switching and reordering fade smoothly, so there are no clicks.',
    view: 'spectrum',
    tap: 'master.l',
  },
  editor: {
    title: 'Wavetable editor',
    text: 'Draw a frame with the pen or line (snap to the grid for steps), set harmonics directly, or type a formula. Process, Harmonics and Morph change the selected frames or the whole table; Import builds a table from a recording, following its pitch. Everything you change plays at once, and Cmd/Ctrl+Z undoes it.',
    view: 'scope',
    tap: 'focus.osc.a',
  },
  'global.quality': {
    title: 'Quality (oversampling)',
    text: 'Warps that bend the wave hard (sync, FM, phase distortion, ring mod) create harmonics above half the sample rate, which fold back down as aliasing. Oversampling runs those voices at 2× or 4× and filters the extra away. It only runs while such a warp is in use.',
    view: 'spectrum',
    tap: 'master.l',
    tries: [{ label: 'Set 4× oversampling', run: setAll({ 'global.quality': 2 }) }],
  },
  'global.tempo': {
    title: 'Tempo and rates',
    text: 'The tempo that synced LFOs, envelopes and delays follow, and two knobs that speed up or slow down every envelope or every LFO at once.',
  },
  ...Object.fromEntries((['a', 'b', 'c'] as const).map((l) => [`osc.${l}`, osc(l)])),
  'filter.1': filter(1),
  'filter.2': filter(2),
  ...Object.fromEntries(Object.keys(SOURCE_NAMES).map((k) => [`mix.${k}`, mix(k)])),
  ...Object.fromEntries(FX_TYPES.map((t, i) => [`fx.${t.key}`, fx(t.key, i)])),
};

/** What to say about a data-explain key, or null if nothing (a gap the lint test catches). */
export function explain(key: string): Explanation | null {
  const c = CONTENT[key];
  if (c) return c;
  const id = PARAM_ID[key as ParamKey];
  if (id !== undefined) {
    const p = PARAMS[id];
    if (p.explain) return { title: p.name, text: p.explain };
  }
  return null;
}
