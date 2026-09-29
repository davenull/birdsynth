// Preset previews: a short phrase to hear a preset by. A preset with a clip
// in its first slot previews with that; the others get a phrase for their
// category (a riff for basses, a line for leads, chords for pads and keys).

import type { Patch } from './patch';

export interface PreviewNote {
  /** Beats from the start. */
  at: number;
  len: number;
  key: number;
  velocity: number;
}

export interface Phrase {
  notes: PreviewNote[];
  /** How long the phrase lasts, in beats (then it stops, or loops). */
  beats: number;
}

const n = (at: number, len: number, key: number, velocity = 0.8): PreviewNote => ({ at, len, key, velocity });
const chord = (at: number, len: number, keys: number[], velocity = 0.7) => keys.map((k) => n(at, len, k, velocity));

const PHRASES: Record<string, Phrase> = {
  Bass: { beats: 4, notes: [n(0, 0.5, 36), n(0.75, 0.25, 36, 0.6), n(1.5, 0.5, 39), n(2, 0.75, 31), n(3, 0.5, 34), n(3.5, 0.5, 36, 0.9)] },
  Lead: { beats: 4, notes: [n(0, 0.5, 60), n(0.5, 0.5, 63), n(1, 1, 67), n(2, 0.5, 70, 0.9), n(2.5, 0.5, 67), n(3, 1, 65)] },
  Pad: { beats: 4, notes: chord(0, 3.5, [48, 55, 58, 62, 63]) },
  Keys: { beats: 4, notes: [...chord(0, 1.75, [48, 51, 55, 58]), ...chord(2, 1.75, [44, 48, 51, 55])] },
  Pluck: { beats: 4, notes: [60, 63, 67, 72, 67, 63, 60, 67].map((k, i) => n(i * 0.5, 0.4, k, i % 2 ? 0.6 : 0.85)) },
  Drums: { beats: 4, notes: [0, 1, 2, 2.5, 3].map((at) => n(at, 0.2, 48, at % 1 ? 0.6 : 0.9)) },
  FX: { beats: 4, notes: [n(0, 3, 60)] },
};
const DEFAULT: Phrase = { beats: 3, notes: [n(0, 2, 48)] };

/** The phrase for a patch: its own first clip (up to 8 beats), or its category's. */
export function phraseFor(patch: Pick<Patch, 'meta' | 'clips'>): Phrase {
  const c = patch.clips?.[0];
  if (c && c.notes.length) {
    const beats = Math.min(8, c.length);
    return { beats, notes: c.notes.filter((x) => x.start < beats).map((x) => n(x.start, Math.min(x.length, beats - x.start), x.key, x.velocity)) };
  }
  return PHRASES[patch.meta.category] ?? DEFAULT;
}
