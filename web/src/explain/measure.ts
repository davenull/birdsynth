// Numbers the explainer (and its automated tour check) reads off a signal:
// how far the strongest partial that isn't a harmonic of the note sits
// below the strongest harmonic. Aliasing shows up as exactly that.

import { spectrumDb } from './fft';

/**
 * dB of the loudest bin more than `guard` Hz from every harmonic of f0,
 * relative to the loudest harmonic (a negative number; lower is cleaner).
 */
export function inharmonicDbc(x: Float32Array, sr: number, f0: number, guard = 3 * (sr / x.length)): number {
  const db = new Float32Array(x.length / 2 + 1);
  spectrumDb(x, db);
  const binHz = sr / x.length;
  let harm = -Infinity;
  let other = -Infinity;
  // ignore DC and the lowest bins (window leakage of DC, not a partial)
  for (let k = 3; k < db.length; k++) {
    const f = k * binHz;
    const n = Math.max(1, Math.round(f / f0));
    const near = Math.abs(f - n * f0) <= guard + binHz;
    if (near) harm = Math.max(harm, db[k]);
    else other = Math.max(other, db[k]);
  }
  return other - harm;
}
