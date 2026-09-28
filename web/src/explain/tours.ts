// Tours: short scripted lessons. Each step can load a teaching patch, play
// notes, flip a switch, turn to a page and point at a part of the synth,
// with a live view to watch. Leaving a tour puts your sound back.

import type { TapName } from '../gen/protocol';
import type { PageId } from '../ui/nav.svelte';
import { PARAMS, PARAM_ID } from '../gen/params';
import { patchOf } from '../presets/factory';
import { toNorm } from '../state/param-math';
import type { Synth } from '../synth';

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
  /** What an automated run checks while the step shows (read through __synth.tour). */
  expect?: { inharmonicBelow?: number; inharmonicAbove?: number };
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
];
