// How the filter types are listed: their sections (mirroring `group` in
// crates/dsp/src/filter.rs) and what the Var control means for each.

export function filterGroup(kind: number): string {
  if (kind <= 7 || (kind >= 12 && kind <= 21)) return 'Normal';
  if (kind <= 25 && kind >= 22) return 'EQ';
  if (kind >= 26 && kind <= 29) return 'Multi';
  if ((kind >= 8 && kind <= 11) || (kind >= 30 && kind <= 37)) return 'Ladders';
  if (kind >= 38 && kind <= 42) return 'Sallen-Key';
  if (kind >= 43 && kind <= 55) return 'Flanges';
  if (kind >= 56 && kind <= 58) return 'Formant';
  return 'Misc';
}

/** A short label for the Var knob on a type ('' when the type doesn't use it). */
export function varLabel(kind: number): string {
  if (kind >= 22 && kind <= 25) return 'Q';
  if (kind >= 26 && kind <= 29) return 'Morph';
  if (kind >= 43 && kind <= 48) return 'Tone';
  if (kind >= 50 && kind <= 55) return 'Spread';
  if (kind >= 56 && kind <= 58) return 'Vowel';
  if (kind === 59) return 'Wave';
  if (kind === 62) return 'Width';
  return '';
}

/** What Resonance does on a type, when it isn't resonance. */
export function resLabel(kind: number): string {
  if (kind >= 22 && kind <= 25) return 'Gain';
  if (kind >= 43 && kind <= 49) return 'Feedback';
  if (kind >= 50 && kind <= 55) return 'Feedback';
  if (kind === 59) return 'Dry';
  if (kind === 60) return 'Smooth';
  if (kind === 61) return 'Diffuse';
  return '';
}
