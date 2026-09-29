// Tours: short scripted lessons. Each step can load a teaching patch, play
// notes, flip a switch, turn to a page and point at a part of the synth,
// with a live view to watch. Leaving a tour puts your sound back.

import type { TapName } from '../gen/protocol';
import type { PageId } from '../ui/nav.svelte';
import { PARAMS, PARAM_ID } from '../gen/params';
import { patchOf } from '../presets/factory';
import { toNorm } from '../state/param-math';
import type { Synth } from '../synth';
import type { Expect } from './check';
import { SOURCE } from '../state/matrix';
import { FX_TYPES } from '../state/fx';
import { ROUTE } from '../state/routing';

/** What a step can use while it's showing; timers stop when it ends. */
export interface StepCtx {
  every(ms: number, fn: () => void): void;
  after(ms: number, fn: () => void): void;
}

export interface Step {
  title: string;
  text: string;
  /** data-explain key of the part to point at. */
  target?: string;
  page?: PageId;
  view?: 'spectrum' | 'scope';
  tap?: TapName;
  enter?(s: Synth, cx: StepCtx): void | Promise<void>;
  leave?(s: Synth): void;
  /** What an automated run checks while the step shows (see check.ts; run through __synth.tour.check). */
  expect?: Expect;
}

export interface Tour {
  id: string;
  title: string;
  summary: string;
  steps: Step[];
}

// A bare saw: one mono voice, no filter, no effects, glide ready for later.
const SAW = patchOf({
  name: 'Tour: bare saw',
  category: 'Tour',
  tags: [],
  tables: ['Saw'],
  p: { 'voice.mono': 1, 'voice.legato': 1, 'env.1.attack': 5, 'env.1.sustain': 1, 'env.1.release': 200, 'master.volume': -18 },
});

const HIGH = 84; // C6, 1047 Hz: its harmonics pass 24 kHz by the 23rd
const TOP = 96;

/** Hold the high note and glide an octave up and back, every few seconds. */
function glideLoop(s: Synth, cx: StepCtx): void {
  s.setParam('voice.glide', toNorm(PARAMS[PARAM_ID['voice.glide']], 2000));
  let up = false;
  cx.every(3000, () => {
    up = !up;
    if (up) s.noteOn(TOP, 0.8);
    else s.noteOff(TOP);
  });
}

const set = (s: Synth, vals: Record<string, number>) => {
  for (const [k, v] of Object.entries(vals)) s.setParam(k as keyof typeof PARAM_ID, toNorm(PARAMS[PARAM_ID[k as keyof typeof PARAM_ID]], v));
};

/** Play `note` for `len` ms every `ms` ms while the step shows (and once now). */
function pulse(s: Synth, cx: StepCtx, note: number, ms: number, len: number, velocity = 0.8): void {
  const hit = () => {
    s.noteOn(note, velocity);
    cx.after(len, () => s.noteOff(note));
  };
  hit();
  cx.every(ms, hit);
}

// The basics: a saw at C3, played mono so each step changes one voice.
const BASIC = patchOf({
  name: 'Tour: basics',
  category: 'Tour',
  tags: [],
  tables: ['Saw'],
  p: { 'voice.mono': 1, 'env.1.attack': 5, 'env.1.sustain': 1, 'env.1.release': 150, 'master.volume': -16 },
});

// For modulation: a saw through a low-pass, some resonance.
const MOVING = patchOf({
  name: 'Tour: moving',
  category: 'Tour',
  tags: [],
  tables: ['Saw'],
  p: { 'voice.mono': 1, 'env.1.attack': 5, 'env.1.sustain': 1, 'env.1.release': 150, 'filter.1.enable': 1, 'filter.1.type': 1, 'filter.1.cutoff': 500, 'filter.1.res': 0.35, 'master.volume': -14 },
});

// For warps: a sine (osc B a sine an octave up, silent, for FM).
const SINE = patchOf({
  name: 'Tour: sine',
  category: 'Tour',
  tags: [],
  tables: ['Sine', 'Sine'],
  p: { 'voice.mono': 1, 'env.1.attack': 5, 'env.1.sustain': 1, 'env.1.release': 150, 'osc.b.enable': 1, 'osc.b.octave': 1, 'osc.b.route': ROUTE.none, 'master.volume': -14 },
});

// For patterns: a short pluck.
const PLUCK = patchOf({
  name: 'Tour: pluck',
  category: 'Tour',
  tags: [],
  tables: ['Basic Shapes'],
  p: { 'osc.a.wt_pos': 0.4, 'env.1.attack': 1, 'env.1.decay': 260, 'env.1.sustain': 0.1, 'env.1.release': 120, 'filter.1.enable': 1, 'filter.1.cutoff': 2400, 'master.volume': -13 },
  fx: [{ type: 'delay', p: { mix: 0.2 } }],
});

// For the signal flow: every path in use (oscillators, sub and noise, both
// filters, a direct source, sends to both buses, effects in all three racks).
const EVERYTHING = patchOf({
  name: 'Tour: everything',
  category: 'Tour',
  tags: [],
  tables: ['Saw', 'Basic Shapes', 'Square'],
  p: {
    'voice.mono': 1,
    'env.1.attack': 5,
    'env.1.sustain': 1,
    'env.1.release': 200,
    'osc.a.level': 0.55,
    'osc.b.enable': 1,
    'osc.b.level': 0.5,
    'osc.b.route': ROUTE.f2,
    'osc.c.enable': 1,
    'osc.c.level': 0.4,
    'osc.c.send1': 0.6,
    'sub.enable': 1,
    'sub.level': 0.4,
    'sub.send2': 0.5,
    'noise.enable': 1,
    'noise.level': 0.2,
    'noise.route': ROUTE.direct,
    'filter.1.enable': 1,
    'filter.1.cutoff': 3000,
    'filter.2.enable': 1,
    'filter.2.cutoff': 1500,
    'rack.bus2_to': 1,
    'master.volume': -16,
  },
  fx: [
    { type: 'distortion', chain: 0, p: { mix: 0.3 } },
    { type: 'reverb', chain: 1, p: { mix: 1 } },
    { type: 'delay', chain: 2, p: { mix: 1 } },
  ],
});

const fxIndex = (key: string) => FX_TYPES.findIndex((t) => t.key === key);

export const TOURS: Tour[] = [
  {
    id: 'aliasing',
    title: 'Aliasing',
    summary: 'Why a digital oscillator has to band-limit, and what it sounds like when it doesn’t.',
    steps: [
      {
        title: 'A saw and its harmonics',
        text: 'This is one saw wave at C6, 1047 Hz. A saw holds every harmonic of its note: 1047, 2093, 3140 Hz and on up, each a little quieter. The yellow ticks on the spectrum mark where harmonics belong, and every peak sits on one.',
        target: 'osc.a',
        page: 'osc',
        view: 'spectrum',
        tap: 'master.l',
        async enter(s) {
          s.allNotesOff();
          await s.loadPatch(SAW);
          s.noteOn(HIGH, 0.8);
        },
        expect: { inharmonicBelow: -60 },
      },
      {
        title: 'A ceiling at half the sample rate',
        text: 'Digital audio at 48 kHz can only hold frequencies up to 24 kHz (the shaded band starts there). A saw’s harmonics go on forever, so birdsynth keeps every wavetable at 11 sizes, one per octave, and plays the one whose harmonics all fit under the ceiling.',
        target: 'osc.a',
        view: 'spectrum',
        tap: 'master.l',
        expect: { inharmonicBelow: -60 },
      },
      {
        title: 'Band-limiting off',
        text: 'Now the oscillator reads its full-size table at every pitch (a teaching switch: normal playing never does this). Harmonics above the ceiling don’t disappear. They fold back down, mirrored around it, as aliases: the new peaks between the ticks. Listen for the thin, glassy whistle on top.',
        target: 'osc.a',
        view: 'spectrum',
        tap: 'master.l',
        enter(s) {
          s.setBandlimit(false);
        },
        expect: { inharmonicAbove: -45 },
      },
      {
        title: 'Aliases move the wrong way',
        text: 'The note now glides up an octave and back. The harmonics rise with it, but many aliases fall: a harmonic climbing past the ceiling folds back as one coming down. That backwards, clangy motion is the sound of aliasing.',
        target: 'osc.a',
        view: 'spectrum',
        tap: 'master.l',
        enter(s, cx) {
          glideLoop(s, cx);
        },
        leave(s) {
          s.noteOff(TOP);
        },
      },
      {
        title: 'Band-limiting on again',
        text: 'Switched back on, the same glide is clean: every peak stays on a tick. Warps that make new harmonics as they bend the wave, like sync and FM, can alias too; for those the voice runs at 2× or 4× the sample rate (Quality, on this page) and filters the extra away.',
        target: 'global.quality',
        page: 'global',
        view: 'spectrum',
        tap: 'master.l',
        enter(s, cx) {
          s.setBandlimit(true);
          glideLoop(s, cx);
        },
        leave(s) {
          s.noteOff(TOP);
        },
      },
      {
        title: 'That’s the tour',
        text: 'Leaving puts your own sound back. Explain mode (the ? button) has notes on every other part of the synth.',
        enter(s) {
          s.allNotesOff();
        },
      },
    ],
  },
  {
    id: 'basics',
    title: 'Basics',
    summary: 'From one bare wave to a sound: the wavetable, the filter, the amp envelope, unison and space.',
    steps: [
      {
        title: 'One oscillator',
        text: 'Every sound here starts as oscillators: a single cycle of a wave, repeated at the note’s pitch. This one is a saw at C3 (131 Hz). On the spectrum, each peak is a harmonic: whole-number multiples of the note, which is what makes it bright and buzzy.',
        target: 'osc.a',
        page: 'osc',
        view: 'spectrum',
        tap: 'master.l',
        async enter(s) {
          s.allNotesOff();
          await s.loadPatch(BASIC);
          s.noteOn(48, 0.8);
        },
        expect: { note: 48, levelAbove: -40 },
      },
      {
        title: 'A wavetable',
        text: 'A wavetable is a row of those single cycles, and Position picks which one plays. Here it sweeps back and forth through Basic Shapes (sine to triangle to saw to square), moved by an LFO: watch the 3D view and hear the tone change while the pitch stays put.',
        target: 'osc.a',
        view: 'scope',
        tap: 'master.l',
        async enter(s) {
          await s.tables.loadFactory(0, 'Basic Shapes');
          set(s, { 'lfo.1.rate': 0.35, 'osc.a.wt_pos': 0 });
          s.matrix.add(SOURCE['LFO 1'], PARAM_ID['osc.a.wt_pos'], 1);
        },
        expect: { levelAbove: -40, moves: ['oscWtPos', 0.2, 0] },
      },
      {
        title: 'The filter',
        text: 'A filter takes harmonics away. This low-pass keeps what’s under its cutoff (600 Hz) and cuts what’s above, 24 dB for every octave: the spectrum tilts down and the sound darkens. Resonance boosts the harmonics right at the cutoff, for that vocal edge.',
        target: 'filter.1',
        view: 'spectrum',
        tap: 'master.l',
        enter(s) {
          s.matrix.clear();
          s.tables.loadFactory(0, 'Saw');
          set(s, { 'filter.1.enable': 1, 'filter.1.type': 1, 'filter.1.cutoff': 600, 'filter.1.res': 0.3 });
        },
        expect: { levelAbove: -45, highBelow: [3000, -30] },
      },
      {
        title: 'The amp envelope',
        text: 'Envelope 1 shapes every note’s volume: how fast it rises (attack), falls (decay) to a level it holds (sustain), and fades once the key is up (release). Short attack, no sustain: a pluck. The scope shows each note’s shape.',
        target: 'env',
        view: 'scope',
        tap: 'focus.out',
        enter(s, cx) {
          s.allNotesOff();
          set(s, { 'env.1.attack': 2, 'env.1.decay': 350, 'env.1.sustain': 0, 'env.1.release': 250 });
          pulse(s, cx, 48, 700, 300);
        },
        leave(s) {
          s.allNotesOff();
        },
        expect: { moves: ['focusEnv', 0.3, 0] },
      },
      {
        title: 'Unison',
        text: 'Unison stacks copies of the oscillator, each detuned a little and spread across the stereo field. They drift in and out of phase, which is the thick, chorused sound of a supersaw. Width spreads them left to right.',
        target: 'osc.a',
        view: 'spectrum',
        tap: 'master.l',
        enter(s) {
          set(s, { 'env.1.attack': 5, 'env.1.sustain': 1, 'osc.a.unison': 7, 'osc.a.detune': 0.25, 'osc.a.width': 1, 'filter.1.cutoff': 2500 });
          s.noteOn(48, 0.8);
        },
        expect: { levelAbove: -40, widthAbove: -15 },
      },
      {
        title: 'Space',
        text: 'Effects work on the sum of every voice. A reverb in the Main rack adds a room: listen to the tail after each note. The FX page holds three racks of as many effects as you like.',
        target: 'fx',
        page: 'fx',
        view: 'scope',
        tap: 'master.l',
        enter(s, cx) {
          s.allNotesOff();
          const r = s.fx.add(0, fxIndex('reverb'));
          if (r) set(s, { [`fx.reverb.${r.inst + 1}.mix`]: 0.35 });
          set(s, { 'env.1.sustain': 0.3 });
          pulse(s, cx, 48, 1000, 450);
        },
        leave(s) {
          s.allNotesOff();
        },
        expect: { levelAbove: -62 },
      },
      {
        title: 'That’s the tour',
        text: 'Oscillator, filter, amp, unison, effects: most sounds are those five things, moved over time. The Modulation tour is next.',
        enter(s) {
          s.allNotesOff();
        },
      },
    ],
  },
  {
    id: 'modulation',
    title: 'Modulation',
    summary: 'Making sounds move: LFOs, envelopes, macros and the matrix.',
    steps: [
      {
        title: 'An LFO on the cutoff',
        text: 'An LFO is a slow oscillator you don’t hear: it moves a knob instead. Here LFO 1, synced to a quarter note, sweeps the filter’s cutoff up and down. Drag any source’s handle onto a knob to do the same; the ring shows how far it moves.',
        target: 'lfo',
        page: 'osc',
        view: 'spectrum',
        tap: 'master.l',
        async enter(s) {
          s.allNotesOff();
          await s.loadPatch(MOVING);
          set(s, { 'lfo.1.bpm': 1, 'lfo.1.sync_rate': 5 });
          s.matrix.add(SOURCE['LFO 1'], PARAM_ID['filter.1.cutoff'], 0.5);
          s.noteOn(45, 0.8);
        },
        expect: { levelAbove: -45, moves: ['focusCutoff', 400] },
      },
      {
        title: 'An envelope on the cutoff',
        text: 'Envelopes can move anything too. Envelope 2 opens the filter at each note’s start and closes it as it decays: the classic filter pluck. Unlike an LFO, it starts again with every note.',
        target: 'env',
        view: 'scope',
        tap: 'master.l',
        enter(s, cx) {
          s.allNotesOff();
          s.matrix.clear();
          set(s, { 'env.2.attack': 1, 'env.2.decay': 300, 'env.2.sustain': 0 });
          s.matrix.add(SOURCE['Env 2'], PARAM_ID['filter.1.cutoff'], 0.55);
          pulse(s, cx, 45, 700, 350);
        },
        leave(s) {
          s.allNotesOff();
        },
        expect: { moves: ['focusCutoff', 400] },
      },
      {
        title: 'Macros',
        text: 'A macro is a knob that does nothing until you route it, and then moves everything it’s routed to at once. Macro 1 now opens the cutoff, raises the resonance and adds drive: one turn, three changes. It’s turning by itself for this step.',
        target: 'macro',
        view: 'spectrum',
        tap: 'master.l',
        enter(s, cx) {
          s.allNotesOff();
          s.matrix.clear();
          s.matrix.add(SOURCE['Macro 1'], PARAM_ID['filter.1.cutoff'], 0.5);
          s.matrix.add(SOURCE['Macro 1'], PARAM_ID['filter.1.res'], 0.4);
          s.matrix.add(SOURCE['Macro 1'], PARAM_ID['filter.1.drive'], 0.5);
          s.noteOn(45, 0.8);
          let t = 0;
          cx.every(50, () => {
            t += 0.05;
            set(s, { 'macro.1.value': 0.5 - 0.5 * Math.cos(t * Math.PI) });
          });
        },
        expect: { routings: 3, moves: ['macroValue', 0.4, 0] },
      },
      {
        title: 'The matrix',
        text: 'Every routing lives here, in one table: source, amount, destination, a curve, and an aux source that scales it (velocity scaling an envelope, say). Up to 64 of them. Bypass one to hear what it does.',
        target: 'matrix',
        page: 'matrix',
        expect: { routings: 3 },
      },
      {
        title: 'That’s the tour',
        text: 'Anything with a handle can move anything with a knob. The Warps tour shows what happens inside the oscillator itself.',
        enter(s) {
          s.allNotesOff();
        },
      },
    ],
  },
  {
    id: 'warps',
    title: 'Warps',
    summary: 'Bending the wave inside the oscillator: sync, bend, PWM and FM.',
    steps: [
      {
        title: 'A sine',
        text: 'A sine is the plainest wave there is: one harmonic, nothing above it. Every step here adds harmonics without a filter or another wave, by bending how the oscillator reads its cycle.',
        target: 'osc.a',
        page: 'osc',
        view: 'spectrum',
        tap: 'master.l',
        async enter(s) {
          s.allNotesOff();
          await s.loadPatch(SINE);
          s.noteOn(60, 0.8);
        },
        expect: { note: 60, highBelow: [1000, -40] },
      },
      {
        title: 'Sync',
        text: 'Sync restarts the cycle faster than the note, so the wave is cut off and begins again: sharp edges, and harmonics that sweep as the amount moves (it’s sweeping now). The pitch stays the note’s.',
        target: 'osc.a',
        view: 'spectrum',
        tap: 'master.l',
        enter(s, cx) {
          set(s, { 'osc.a.warp1_mode': 1, 'osc.a.warp1_amount': 0.4 });
          let t = 0;
          cx.every(50, () => {
            t += 0.05;
            set(s, { 'osc.a.warp1_amount': 0.35 + 0.3 * Math.sin(t * 1.5) });
          });
        },
        expect: { levelAbove: -45, highAbove: [1000, -30] },
      },
      {
        title: 'Bend and PWM',
        text: 'Bend speeds up one half of the cycle and slows the other, leaning the wave; PWM squeezes it into part of the cycle and leaves the rest flat, like a pulse wave’s width. Both add harmonics smoothly as they grow.',
        target: 'osc.a',
        view: 'scope',
        tap: 'master.l',
        enter(s) {
          set(s, { 'osc.a.warp1_mode': 3, 'osc.a.warp1_amount': 0.6, 'osc.a.warp2_mode': 6, 'osc.a.warp2_amount': 0.4 });
        },
        expect: { levelAbove: -45, highAbove: [1000, -35] },
      },
      {
        title: 'FM from another oscillator',
        text: 'FM lets one oscillator wobble another’s phase at audio rate. Oscillator B, a silent sine an octave up, now modulates A: the spectrum fills with sidebands, bell- and metal-like tones. More amount, more of them.',
        target: 'osc.b',
        view: 'spectrum',
        tap: 'master.l',
        enter(s) {
          set(s, { 'osc.a.warp1_mode': 18, 'osc.a.warp1_amount': 0.5, 'osc.a.warp2_mode': 0, 'osc.a.warp2_amount': 0 });
        },
        expect: { levelAbove: -45, highAbove: [1000, -30] },
      },
      {
        title: 'Keeping it clean',
        text: 'Warps that make sharp edges can make harmonics above half the sample rate, which fold back as aliasing (the Aliasing tour shows it). Quality runs those voices at 2× or 4× and filters the extra away: it’s at 2× now.',
        target: 'global.quality',
        page: 'global',
        view: 'spectrum',
        tap: 'master.l',
        enter(s) {
          set(s, { 'global.quality': 1 });
        },
        expect: { levelAbove: -45 },
      },
      {
        title: 'That’s the tour',
        text: 'Two warp slots per oscillator, on every oscillator type: warps are the quickest way to move a sound without a filter.',
        enter(s) {
          s.allNotesOff();
        },
      },
    ],
  },
  {
    id: 'patterns',
    title: 'Patterns',
    summary: 'The arpeggiator, its step lanes, and clips that play in time.',
    steps: [
      {
        title: 'The arpeggiator',
        text: 'Hold a chord and the arpeggiator plays its notes one at a time, in time: here C minor, going up in sixteenths at 120 BPM. Shapes change the order; Range repeats it higher.',
        target: 'arp',
        page: 'arp',
        view: 'scope',
        tap: 'master.l',
        async enter(s) {
          s.allNotesOff();
          await s.loadPatch(PLUCK);
          set(s, { 'arp.enable': 1, 'arp.rate': 4, 'arp.octaves': 2, 'global.bpm': 120 });
          for (const n of [48, 51, 55]) s.noteOn(n, 0.8);
        },
        leave(s) {
          for (const n of [48, 51, 55]) s.noteOff(n);
        },
        expect: { arp: true, levelAbove: -50 },
      },
      {
        title: 'Step lanes',
        text: 'Each step has its own velocity, gate, chance, bend and strum. Here every fourth step is accented and the others shorter, and a few steps rest: the same chord becomes a groove.',
        target: 'arp.lanes',
        view: 'scope',
        tap: 'master.l',
        enter(s) {
          for (let st = 0; st < 16; st++) {
            s.arp.set(0, 'velocity', st, st % 4 === 0 ? 1 : 0.55);
            s.arp.set(0, 'gate', st, st % 4 === 0 ? 0.6 : 0.25);
            s.arp.set(0, 'on', st, [3, 7, 14].includes(st) ? 0 : 1);
          }
          for (const n of [48, 51, 55]) s.noteOn(n, 0.8);
        },
        leave(s) {
          for (const n of [48, 51, 55]) s.noteOff(n);
          s.arp.load(0, null);
        },
        expect: { arp: true },
      },
      {
        title: 'A clip',
        text: 'Clips are short note sequences that play while the transport runs: twelve of them, each with its own notes and automation. This one is a little bass line, looping every bar. Draw notes in the piano roll, or record what you play.',
        target: 'clip.roll',
        page: 'clip',
        view: 'scope',
        tap: 'master.l',
        enter(s) {
          set(s, { 'arp.enable': 0, 'clip.enable': 1, 'clip.slot': 1, 'clip.quantize': 0 });
          s.clips.edit(0, (c) => {
            c.length = 4;
            c.notes = [0, 0.75, 1.5, 2, 2.75, 3.5].map((start, i) => ({ start, length: 0.3, key: [36, 36, 43, 39, 41, 43][i], velocity: 0.8, chance: 1, bend: 0 }));
          });
          s.transport(true);
        },
        expect: { clip: true, levelAbove: -50 },
      },
      {
        title: 'Swing',
        text: 'Swing delays every second sixteenth, for a shuffled feel. It moves the arpeggiator and the clips alike, so they stay together.',
        target: 'transport',
        view: 'scope',
        tap: 'master.l',
        enter(s) {
          set(s, { 'global.swing': 0.55 });
        },
        expect: { clip: true },
      },
      {
        title: 'That’s the tour',
        text: 'Stopping the transport ends every note it started. Trigger keys (C1 to B1) can launch the twelve clips as you play.',
        enter(s) {
          s.transport(false);
          s.allNotesOff();
        },
      },
    ],
  },
  {
    id: 'flow',
    title: 'Signal flow',
    summary: 'Follow the sound from the oscillators through the filters, amp, buses and effects to the output.',
    steps: [
      {
        title: 'The whole path',
        text: 'The FLOW page draws the synth as it’s wired right now, and every box is a live scope. This patch uses every path: three oscillators, sub and noise, both filters, sends to both FX buses and effects in all three racks.',
        target: 'flow',
        page: 'flow',
        async enter(s) {
          s.allNotesOff();
          await s.loadPatch(EVERYTHING);
          s.noteOn(48, 0.8);
        },
        expect: { flowLive: true, levelAbove: -50 },
      },
      {
        title: 'Per voice, then all together',
        text: 'The boxes on the left show one voice: the newest note’s oscillators, filters and amp. From the buses on, they show every voice added together, which is where the effects work.',
        target: 'flow.buses',
        enter(s) {
          s.noteOn(55, 0.7);
        },
        leave(s) {
          s.noteOff(55);
        },
        expect: { flowLive: true },
      },
      {
        title: 'The amp',
        text: 'Every path goes through the amp, Envelope 1: notes starting and stopping now, so the boxes from the amp on pulse with them while the oscillators keep running.',
        target: 'flow.amp',
        enter(s, cx) {
          s.allNotesOff();
          set(s, { 'env.1.attack': 2, 'env.1.decay': 300, 'env.1.sustain': 0.2 });
          pulse(s, cx, 48, 800, 350);
        },
        leave(s) {
          s.allNotesOff();
        },
        expect: { moves: ['focusEnv', 0.3, 0] },
      },
      {
        title: 'That’s the tour',
        text: 'Click any box on the FLOW page to go and edit that part.',
        enter(s) {
          s.allNotesOff();
        },
      },
    ],
  },
];
