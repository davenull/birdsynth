// The filter's magnitude response, for the filter graph. It mirrors
// `filter::response` in crates/engine/src/filter/mod.rs: both cores are exact
// bilinear transforms of their analog prototypes, so this is the true
// digital response, not an approximation.

const SQRT2 = Math.SQRT2;

export const LADDER6 = 8;
const is24 = (k: number) => k === 1 || k === 3 || k === 5 || k === 7;

const svfK = (res: number) => SQRT2 * (1 - res) + 0.02 * res;
const ladderK = (res: number) => 3.98 * res;
const gOf = (cutoff: number, sr: number) => Math.tan((Math.PI * Math.min(Math.max(cutoff, 5), sr * 0.49)) / sr);

type C = [number, number];
const mul = (a: C, b: C): C => [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]];
const add = (a: C, b: C): C => [a[0] + b[0], a[1] + b[1]];
const scale = (a: C, k: number): C => [a[0] * k, a[1] * k];
const div = (a: C, b: C): C => {
  const d = b[0] * b[0] + b[1] * b[1];
  return [(a[0] * b[0] + a[1] * b[1]) / d, (a[1] * b[0] - a[0] * b[1]) / d];
};
const abs = (a: C) => Math.hypot(a[0], a[1]);

/** Linear magnitude of filter `kind` at frequency f (Hz). */
export function response(kind: number, cutoff: number, res: number, sr: number, f: number): number {
  const w = Math.tan((Math.PI * Math.min(Math.max(f, 0), sr * 0.4999)) / sr) / gOf(cutoff, sr);
  const s: C = [0, w];
  const one: C = [1, 0];
  if (kind >= LADDER6) {
    const k = ladderK(res);
    const g = div(one, add(one, s));
    const g4 = mul(mul(g, g), mul(g, g));
    let gn = one;
    for (let i = 0; i <= kind - LADDER6; i++) gn = mul(gn, g);
    return abs(div(gn, add(one, scale(g4, k)))) * (1 + 0.5 * k);
  }
  const svf = (k: number): number => {
    const den = add(add(mul(s, s), scale(s, k)), one);
    const num = kind <= 1 ? one : kind <= 3 ? mul(s, s) : kind <= 5 ? scale(s, k) : add(mul(s, s), one);
    return abs(div(num, den));
  };
  const h = svf(svfK(res));
  return is24(kind) ? h * svf(SQRT2) : h;
}
