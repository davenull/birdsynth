// Factory presets, written as plain values (Hz, ms, semitones) so they read
// like a patch sheet; `build` turns each into a Patch. All sounds are ours:
// our own wavetables, noises and impulse responses.

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';
import { CHAINS, FX_TYPES } from '../state/fx';
import { LFO_COUNT, type LfoPoint } from '../state/lfo';
import { toNorm } from '../state/param-math';
import { PATCH_FORMAT, PATCH_VERSION, emptyMeta, type MatrixRow, type Patch } from '../state/patch';

type Plain = Record<string, number>;
type Route = [source: string, dest: string, amount: number, opts?: { bipolar?: boolean; curve?: number; aux?: string }];
interface FxDef {
  chain?: number;
  type: string;
  p?: Plain;
}
export interface Def {
  name: string;
  category: string;
  tags: string[];
  /** Factory wavetable per oscillator. */
  tables?: [string?, string?, string?];
  p?: Plain;
  mod?: Route[];
  fx?: FxDef[];
  lfo?: Record<number, LfoPoint[]>;
}

const DEFS: Def[] = [
  { name: 'Init', category: 'Init', tags: ['init'] },
  // ------------------------------------------------------------------ bass
  {
    name: 'Sub Anchor',
    category: 'Bass',
    tags: ['sub', 'clean', 'mono'],
    tables: ['Sine'],
    p: {
      'master.volume': -8.9,
      'voice.mono': 1,
      'voice.legato': 1,
      'voice.glide': 40,
      'sub.enable': 1,
      'sub.level': 0.6,
      'sub.octave': -1,
      'env.1.attack': 2,
      'env.1.decay': 400,
      'env.1.sustain': 0.8,
      'env.1.release': 80,
    },
  },
  {
    name: 'Reese',
    category: 'Bass',
    tags: ['dark', 'detuned', 'moving'],
    tables: ['Saw', 'Saw'],
    p: {
      'master.volume': -2.9,
      'osc.a.unison': 4,
      'osc.a.detune': 0.18,
      'osc.b.enable': 1,
      'osc.b.fine': 12,
      'osc.b.unison': 4,
      'osc.b.detune': 0.22,
      'osc.b.octave': -1,
      'filter.1.enable': 1,
      'filter.1.type': 1,
      'filter.1.cutoff': 700,
      'filter.1.res': 0.25,
      'lfo.1.rate': 0.15,
      'voice.mono': 1,
      'env.1.release': 120,
    },
    mod: [['LFO 1', 'filter.1.cutoff', 0.15]],
    fx: [
      { type: 'distortion', p: { mode: 13, drive: 0.25 } },
      { type: 'compressor', p: { threshold: -20, ratio: 3, gain: 3 } },
    ],
  },
  {
    name: 'Acid Squelch',
    category: 'Bass',
    tags: ['acid', 'resonant', 'mono'],
    tables: ['Saw'],
    p: {
      'master.volume': -11.5,
      'voice.mono': 1,
      'voice.legato': 1,
      'voice.glide': 60,
      'filter.1.enable': 1,
      'filter.1.type': 35,
      'filter.1.cutoff': 380,
      'filter.1.res': 0.82,
      'filter.1.drive': 0.4,
      'env.2.attack': 0,
      'env.2.decay': 180,
      'env.2.sustain': 0,
      'env.1.sustain': 0.9,
    },
    mod: [
      ['Env 2', 'filter.1.cutoff', 0.35],
      ['Velocity', 'filter.1.cutoff', 0.12],
    ],
    fx: [
      { type: 'distortion', p: { mode: 0, drive: 0.3 } },
      { type: 'delay', p: { bpm: 1, sync_l: 7, feedback: 0.3, mix: 0.15 } },
    ],
  },
  {
    name: 'Growl',
    category: 'Bass',
    tags: ['growl', 'aggressive', 'wavetable'],
    tables: ['Growl'],
    p: {
      'master.volume': -7.4,
      'osc.a.octave': -1,
      'osc.a.unison': 2,
      'osc.a.detune': 0.1,
      'lfo.1.bpm': 1,
      'lfo.1.sync_rate': 6,
      'lfo.1.mode': 1,
      'sub.enable': 1,
      'sub.level': 0.5,
      'voice.mono': 1,
    },
    mod: [['LFO 1', 'osc.a.wt_pos', 0.8]],
    fx: [
      { type: 'distortion', p: { mode: 1, drive: 0.4 } },
      {
        type: 'compressor',
        p: { mode: 1, depth: 0.7, upward: 0.7, downward: 0.7 },
      },
    ],
  },
  {
    name: 'FM Punch',
    category: 'Bass',
    tags: ['fm', 'punchy'],
    tables: ['Sine', 'Sine'],
    p: {
      'master.volume': -3.1,
      'osc.b.enable': 1,
      'osc.b.level': 0,
      'osc.b.semi': 7,
      'osc.a.warp1_mode': 18,
      'osc.a.warp1_amount': 0.1,
      'env.2.decay': 250,
      'env.2.sustain': 0.1,
      'env.1.decay': 600,
      'env.1.sustain': 0.6,
      'voice.mono': 1,
      'global.quality': 1,
    },
    mod: [['Env 2', 'osc.a.warp1_amount', 0.35]],
    fx: [{ type: 'compressor', p: { threshold: -16, ratio: 4, gain: 4 } }],
  },
  {
    name: 'Wobble',
    category: 'Bass',
    tags: ['wobble', 'tempo'],
    tables: ['Basic Shapes'],
    p: {
      'master.volume': -12,
      'osc.a.wt_pos': 0.4,
      'osc.a.octave': -1,
      'filter.1.enable': 1,
      'filter.1.type': 1,
      'filter.1.cutoff': 220,
      'filter.1.res': 0.45,
      'lfo.1.bpm': 1,
      'lfo.1.sync_rate': 6,
      'lfo.1.mode': 1,
      'voice.mono': 1,
    },
    mod: [['LFO 1', 'filter.1.cutoff', 0.5]],
    fx: [{ type: 'distortion', p: { mode: 13, drive: 0.35 } }],
  },
  // ------------------------------------------------------------------ lead
  {
    name: 'Supersaw Lead',
    category: 'Lead',
    tags: ['supersaw', 'bright', 'wide'],
    tables: ['Saw'],
    p: {
      'master.volume': -3.9,
      'osc.a.unison': 7,
      'osc.a.detune': 0.32,
      'osc.a.width': 1,
      'filter.1.enable': 1,
      'filter.1.type': 0,
      'filter.1.cutoff': 6000,
      'env.1.attack': 5,
      'env.1.release': 250,
    },
    fx: [
      { type: 'chorus', p: { mix: 0.3 } },
      { type: 'delay', p: { bpm: 1, sync_l: 7, feedback: 0.35, mix: 0.2 } },
      { type: 'reverb', p: { algo: 0, decay: 2.2, mix: 0.2 } },
    ],
  },
  {
    name: 'Sync Scream',
    category: 'Lead',
    tags: ['sync', 'aggressive'],
    tables: ['Saw'],
    p: {
      'master.volume': -13.8,
      'osc.a.warp1_mode': 1,
      'osc.a.warp1_amount': 0.2,
      'env.2.attack': 0,
      'env.2.decay': 900,
      'env.2.sustain': 0.2,
      'global.quality': 2,
      'voice.mono': 1,
      'voice.glide': 50,
    },
    mod: [
      ['Env 2', 'osc.a.warp1_amount', 0.5],
      ['Mod Wheel', 'osc.a.warp1_amount', 0.3],
    ],
    fx: [
      { type: 'distortion', p: { mode: 14, drive: 0.3 } },
      { type: 'delay', p: { bpm: 1, sync_l: 6, feedback: 0.3, mix: 0.2 } },
    ],
  },
  {
    name: 'Pulse Lead',
    category: 'Lead',
    tags: ['pwm', 'classic'],
    tables: ['Square'],
    p: {
      'master.volume': -10.9,
      'osc.a.warp1_mode': 6,
      'osc.a.warp1_amount': 0.3,
      'lfo.1.rate': 0.6,
      'voice.glide': 30,
      'env.1.release': 200,
    },
    mod: [['LFO 1', 'osc.a.warp1_amount', 0.25]],
    fx: [
      { type: 'chorus', p: { mix: 0.25 } },
      { type: 'reverb', p: { algo: 1, decay: 1.6, mix: 0.2 } },
    ],
  },
  {
    name: 'Vowel Lead',
    category: 'Lead',
    tags: ['vocal', 'formant', 'expressive'],
    tables: ['Vowels'],
    p: {
      'master.volume': -2.9,
      'filter.1.enable': 1,
      'filter.1.type': 56,
      'filter.1.cutoff': 1000,
      'filter.1.res': 0.4,
      'voice.glide': 40,
    },
    mod: [
      ['Mod Wheel', 'osc.a.wt_pos', 1],
      ['LFO 1', 'filter.1.var', 0.3],
    ],
    fx: [
      {
        type: 'delay',
        p: { mode: 1, bpm: 1, sync_l: 9, feedback: 0.35, mix: 0.2 },
      },
    ],
  },
  {
    name: 'Fold Lead',
    category: 'Lead',
    tags: ['wavefold', 'harsh'],
    tables: ['Sine'],
    p: {
      'master.volume': -15,
      'osc.a.warp1_mode': 54,
      'osc.a.warp1_amount': 0.3,
      'lfo.1.rate': 0.3,
      'global.quality': 1,
      'env.1.release': 180,
    },
    mod: [
      ['LFO 1', 'osc.a.warp1_amount', 0.3],
      ['Velocity', 'osc.a.warp1_amount', 0.3],
    ],
    fx: [{ type: 'reverb', p: { algo: 0, decay: 1.8, mix: 0.2 } }],
  },
  // ------------------------------------------------------------------- pad
  {
    name: 'Glass Pad',
    category: 'Pad',
    tags: ['glassy', 'slow', 'wide'],
    tables: ['Harmonic Build', 'Sine'],
    p: {
      'master.volume': -6.8,
      'osc.a.unison': 4,
      'osc.a.detune': 0.15,
      'osc.b.enable': 1,
      'osc.b.octave': 1,
      'osc.b.level': 0.4,
      'env.1.attack': 900,
      'env.1.release': 2200,
      'lfo.1.rate': 0.08,
    },
    mod: [['LFO 1', 'osc.a.wt_pos', 0.5]],
    fx: [
      { type: 'chorus', p: { mix: 0.35 } },
      { type: 'reverb', p: { algo: 3, size: 0.8, decay: 6, mix: 0.4 } },
    ],
  },
  {
    name: 'Warm Strings',
    category: 'Pad',
    tags: ['strings', 'warm'],
    tables: ['Saw'],
    p: {
      'master.volume': -5.5,
      'osc.a.unison': 6,
      'osc.a.detune': 0.2,
      'filter.1.enable': 1,
      'filter.1.type': 11,
      'filter.1.cutoff': 2200,
      'env.1.attack': 500,
      'env.1.release': 1400,
    },
    fx: [
      { type: 'hyper', p: { hyper_mix: 0.3, dim_mix: 0.4 } },
      { type: 'reverb', p: { algo: 0, decay: 3.5, mix: 0.3 } },
    ],
  },
  {
    name: 'Dark Matter',
    category: 'Pad',
    tags: ['dark', 'noise', 'cinematic'],
    tables: ['Spectral Noise'],
    p: {
      'master.volume': 0.4,
      'noise.enable': 1,
      'noise.type': 2,
      'noise.level': 0.25,
      'filter.1.enable': 1,
      'filter.1.type': 1,
      'filter.1.cutoff': 900,
      'env.1.attack': 1500,
      'env.1.release': 3000,
      'lfo.1.rate': 0.05,
    },
    mod: [['LFO 1', 'osc.a.wt_pos', 0.6]],
    fx: [{ type: 'reverb', p: { algo: 4, decay: 14, mix: 0.45 } }],
  },
  {
    name: 'Evolving Vowels',
    category: 'Pad',
    tags: ['vocal', 'moving'],
    tables: ['Formant Sweep'],
    p: {
      'master.volume': -4,
      'osc.a.unison': 3,
      'osc.a.detune': 0.12,
      'env.1.attack': 700,
      'env.1.release': 1800,
      'lfo.1.rate': 0.1,
      'lfo.2.rate': 0.07,
    },
    mod: [
      ['LFO 1', 'osc.a.wt_pos', 0.8],
      ['LFO 2', 'osc.a.pan', 0.3, { bipolar: true }],
    ],
    fx: [{ type: 'reverb', p: { algo: 0, decay: 4, mix: 0.35 } }],
  },
  {
    name: 'Choir Pad',
    category: 'Pad',
    tags: ['choir', 'formant'],
    tables: ['Vowels', 'Vowels'],
    p: {
      'master.volume': 5.5,
      'osc.a.wt_pos': 0.3,
      'osc.a.level': 1,
      'osc.b.enable': 1,
      'osc.b.level': 1,
      'osc.b.wt_pos': 0.6,
      'osc.b.fine': -8,
      'filter.1.enable': 1,
      'filter.1.type': 57,
      'filter.1.cutoff': 900,
      'env.1.attack': 800,
      'env.1.release': 2000,
    },
    fx: [
      { type: 'chorus', p: { mix: 0.3 } },
      { type: 'reverb', p: { algo: 0, size: 0.7, decay: 4.5, mix: 0.35 } },
    ],
  },
  {
    name: 'Frozen Air',
    category: 'Pad',
    tags: ['airy', 'texture'],
    tables: ['Sine'],
    p: {
      'master.volume': -3.3,
      'osc.a.level': 0.4,
      'noise.enable': 1,
      'noise.type': 7,
      'noise.level': 0.35,
      'env.1.attack': 1200,
      'env.1.release': 3000,
    },
    fx: [
      { type: 'convolve', p: { ir: 9, mix: 0.6 } },
      { type: 'reverb', p: { algo: 3, decay: 8, mix: 0.3 } },
    ],
  },
  // ----------------------------------------------------------------- pluck
  {
    name: 'Glass Pluck',
    category: 'Pluck',
    tags: ['pluck', 'bright'],
    tables: ['Harmonic Build'],
    p: {
      'master.volume': -3.9,
      'osc.a.wt_pos': 0.6,
      'filter.1.enable': 1,
      'filter.1.cutoff': 800,
      'env.1.decay': 450,
      'env.1.sustain': 0,
      'env.1.release': 400,
      'env.2.decay': 250,
      'env.2.sustain': 0,
    },
    mod: [['Env 2', 'filter.1.cutoff', 0.4]],
    fx: [
      { type: 'delay', p: { bpm: 1, sync_l: 7, feedback: 0.4, mix: 0.25 } },
      { type: 'reverb', p: { algo: 1, decay: 1.5, mix: 0.2 } },
    ],
  },
  {
    name: 'Kalimba',
    category: 'Pluck',
    tags: ['fm', 'mallet'],
    tables: ['Sine', 'Sine'],
    p: {
      'master.volume': -5.4,
      'osc.b.enable': 1,
      'osc.b.level': 0,
      'osc.b.pitch_mode': 2,
      'osc.b.semi': 3,
      'osc.a.warp1_mode': 18,
      'osc.a.warp1_amount': 0.05,
      'env.1.decay': 700,
      'env.1.sustain': 0,
      'env.2.decay': 120,
      'env.2.sustain': 0,
    },
    mod: [['Env 2', 'osc.a.warp1_amount', 0.2]],
    fx: [
      { type: 'convolve', p: { ir: 10, mix: 0.2 } },
      { type: 'reverb', p: { algo: 0, decay: 1.4, mix: 0.2 } },
    ],
  },
  {
    name: 'Harp Pluck',
    category: 'Pluck',
    tags: ['soft', 'acoustic'],
    tables: ['Triangle'],
    p: {
      'master.volume': -6,
      'env.1.attack': 1,
      'env.1.decay': 900,
      'env.1.sustain': 0,
      'env.1.release': 900,
      'filter.1.enable': 1,
      'filter.1.cutoff': 3000,
      'env.2.decay': 300,
      'env.2.sustain': 0,
    },
    mod: [['Env 2', 'filter.1.cutoff', 0.25]],
    fx: [{ type: 'reverb', p: { algo: 1, decay: 2, mix: 0.25 } }],
  },
  {
    name: 'Chord Stab',
    category: 'Pluck',
    tags: ['stab', 'house'],
    tables: ['Saw', 'Square'],
    p: {
      'master.volume': -5.3,
      'osc.a.unison': 3,
      'osc.b.enable': 1,
      'osc.b.level': 0.5,
      'filter.1.enable': 1,
      'filter.1.type': 1,
      'filter.1.cutoff': 600,
      'filter.1.res': 0.3,
      'env.1.decay': 350,
      'env.1.sustain': 0,
      'env.2.decay': 200,
      'env.2.sustain': 0,
    },
    mod: [['Env 2', 'filter.1.cutoff', 0.35]],
    fx: [
      { type: 'delay', p: { bpm: 1, sync_l: 7, feedback: 0.3, mix: 0.2 } },
      { type: 'reverb', p: { algo: 0, decay: 2, mix: 0.25 } },
    ],
  },
  // ------------------------------------------------------------------ keys
  {
    name: 'Tonewheel',
    category: 'Keys',
    tags: ['organ', 'vintage'],
    tables: ['Organ'],
    p: {
      'master.volume': -9.6,
      'osc.a.wt_pos': 0.5,
      'env.1.attack': 2,
      'env.1.release': 60,
      'lfo.1.rate': 6.5,
    },
    mod: [['LFO 1', 'osc.a.pan', 0.08, { bipolar: true }]],
    fx: [
      { type: 'chorus', p: { rate: 5.5, depth: 0.15, mix: 0.4 } },
      { type: 'distortion', p: { mode: 0, drive: 0.15 } },
    ],
  },
  {
    name: 'Electric Piano',
    category: 'Keys',
    tags: ['ep', 'soft'],
    tables: ['FM Index'],
    p: {
      'master.volume': -7.5,
      'osc.a.wt_pos': 0.2,
      'env.1.decay': 1800,
      'env.1.sustain': 0.2,
      'env.1.release': 400,
      'lfo.1.rate': 4,
    },
    mod: [
      ['Velocity', 'osc.a.wt_pos', 0.3],
      ['LFO 1', 'osc.a.level', 0.1],
    ],
    fx: [
      { type: 'chorus', p: { mix: 0.3 } },
      { type: 'reverb', p: { algo: 1, decay: 1.8, mix: 0.2 } },
    ],
  },
  {
    name: 'Bell Keys',
    category: 'Keys',
    tags: ['bell', 'fm'],
    tables: ['FM Ratio'],
    p: {
      'master.volume': -5.6,
      'osc.a.wt_pos': 0.35,
      'env.1.decay': 2500,
      'env.1.sustain': 0,
      'env.1.release': 1500,
    },
    fx: [
      { type: 'convolve', p: { ir: 10, mix: 0.25 } },
      { type: 'reverb', p: { algo: 0, decay: 3, mix: 0.25 } },
    ],
  },
  // -------------------------------------------------------------------- fx
  {
    name: 'Riser',
    category: 'FX',
    tags: ['riser', 'noise', 'build'],
    tables: ['Saw'],
    p: {
      'master.volume': 1.5,
      'noise.enable': 1,
      'noise.level': 0.5,
      'osc.a.level': 0.3,
      'filter.1.enable': 1,
      'filter.1.type': 2,
      'filter.1.cutoff': 200,
      'lfo.1.mode': 2,
      'lfo.1.rate': 0.12,
      'env.1.attack': 6000,
    },
    lfo: {
      0: [
        { x: 0, y: -1, c: 0.4 },
        { x: 0.999, y: 1, c: 0 },
      ],
    },
    mod: [
      ['LFO 1', 'filter.1.cutoff', 0.7],
      ['LFO 1', 'osc.a.coarse', 0.12],
    ],
    fx: [{ type: 'reverb', p: { algo: 4, decay: 6, mix: 0.4 } }],
  },
  {
    name: 'Laser Zap',
    category: 'FX',
    tags: ['zap', 'short'],
    tables: ['Square'],
    p: {
      'master.volume': -6.3,
      'env.2.decay': 180,
      'env.2.sustain': 0,
      'env.1.decay': 250,
      'env.1.sustain': 0,
      'osc.a.warp1_mode': 1,
      'osc.a.warp1_amount': 0.5,
      'global.quality': 2,
    },
    mod: [
      ['Env 2', 'osc.a.coarse', 0.3],
      ['Env 2', 'osc.a.warp1_amount', 0.4],
    ],
    fx: [
      {
        type: 'delay',
        p: { mode: 1, bpm: 1, sync_l: 3, feedback: 0.4, mix: 0.3 },
      },
    ],
  },
  {
    name: 'Barber Pole',
    category: 'FX',
    tags: ['shifter', 'drone', 'weird'],
    tables: ['Harmonic Build'],
    p: {
      'master.volume': -7.6,
      'osc.a.wt_pos': 0.7,
      'env.1.attack': 800,
      'env.1.release': 2500,
    },
    fx: [
      { type: 'bode', p: { shift: 3, feedback: 0.8, mix: 0.6 } },
      { type: 'delay', p: { bpm: 0, time_l: 420, feedback: 0.5, mix: 0.3 } },
      { type: 'reverb', p: { algo: 4, decay: 10, mix: 0.4 } },
    ],
  },
  // ------------------------------------------------------------------ perc
  {
    name: 'Round Kick',
    category: 'Drums',
    tags: ['kick', 'drum'],
    tables: ['Sine'],
    p: {
      'master.volume': -4.3,
      'osc.a.octave': -2,
      'env.1.attack': 0,
      'env.1.decay': 320,
      'env.1.sustain': 0,
      'env.1.release': 60,
      'env.2.attack': 0,
      'env.2.decay': 45,
      'env.2.sustain': 0,
      'voice.mono': 1,
    },
    mod: [['Env 2', 'osc.a.coarse', 0.3]],
    fx: [
      { type: 'distortion', p: { mode: 0, drive: 0.2 } },
      {
        type: 'compressor',
        p: { threshold: -12, ratio: 4, attack: 5, gain: 3 },
      },
    ],
  },
  {
    name: 'Noise Snare',
    category: 'Drums',
    tags: ['snare', 'drum', 'noise'],
    tables: ['Triangle'],
    p: {
      'master.volume': 4.5,
      'osc.a.level': 0.6,
      'noise.enable': 1,
      'noise.level': 1,
      'filter.1.enable': 1,
      'filter.1.type': 2,
      'filter.1.cutoff': 900,
      'env.1.attack': 0,
      'env.1.decay': 180,
      'env.1.sustain': 0,
      'env.1.release': 80,
      'env.2.decay': 30,
      'env.2.sustain': 0,
    },
    mod: [['Env 2', 'osc.a.coarse', 0.12]],
    fx: [{ type: 'reverb', p: { algo: 1, decay: 0.9, mix: 0.15 } }],
  },
];

function norm(key: string, plain: number): [number, number] | null {
  const id = PARAM_ID[key as ParamKey];
  if (id === undefined) throw new Error(`factory preset: unknown parameter ${key}`);
  return [id, toNorm(PARAMS[id], plain)];
}

/** A preset definition as a patch (also used for the explainer's teaching patches). */
export function patchOf(d: Def): Patch {
  const params: Record<string, number> = {};
  const set = (key: string, plain: number) => {
    const r = norm(key, plain);
    if (r) params[key] = r[1];
  };
  for (const [k, v] of Object.entries(d.p ?? {})) set(k, v);
  const matrix: MatrixRow[] = (d.mod ?? []).map(([source, dest, amount, o], slot) => ({
    slot,
    source,
    aux: o?.aux ?? 'None',
    dest,
    amount,
    curve: o?.curve ?? 0,
    output: 1,
    bipolar: o?.bipolar ?? false,
    bypass: false,
  }));
  const chains: { type: string; inst: number }[][] = Array.from({ length: CHAINS }, () => []);
  const used: Record<string, number> = {};
  for (const f of d.fx ?? []) {
    if (!FX_TYPES.some((t) => t.key === f.type)) throw new Error(`factory preset: unknown effect ${f.type}`);
    const inst = used[f.type] ?? 0;
    used[f.type] = inst + 1;
    chains[f.chain ?? 0].push({ type: f.type, inst });
    for (const [k, v] of Object.entries(f.p ?? {})) set(`fx.${f.type}.${inst + 1}.${k}`, v);
  }
  const curves: (LfoPoint[] | null)[] = Array(LFO_COUNT).fill(null);
  for (const [i, pts] of Object.entries(d.lfo ?? {})) curves[Number(i)] = pts;
  return {
    format: PATCH_FORMAT,
    version: PATCH_VERSION,
    meta: {
      ...emptyMeta(d.name),
      author: 'birdsynth',
      category: d.category,
      tags: d.tags,
    },
    params,
    matrix,
    lfo: { curves, paths: Array(LFO_COUNT).fill(null) },
    remap: [null, null, null],
    fx: { chains },
    tables: [0, 1, 2].map((o) => (d.tables?.[o] ? { name: d.tables[o]!, source: `factory:${d.tables[o]}`, count: 0 } : { name: 'Saw', source: 'factory:Saw', count: 1 })),
    irs: [null, null, null, null],
    recordings: [null, null, null],
    multis: [null, null, null],
    specFilter: [null, null, null],
  };
}

export const FACTORY: Patch[] = DEFS.map(patchOf);
