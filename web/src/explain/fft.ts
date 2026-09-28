// A small FFT for the explainer's spectrum view: magnitudes in dB of one
// windowed block. Radix-2, in place, tables made once per size.

const cache = new Map<number, { cos: Float64Array; sin: Float64Array; rev: Uint32Array; win: Float64Array; gain: number }>();

function tables(n: number) {
  let t = cache.get(n);
  if (t) return t;
  const cos = new Float64Array(n / 2);
  const sin = new Float64Array(n / 2);
  for (let i = 0; i < n / 2; i++) {
    cos[i] = Math.cos((-2 * Math.PI * i) / n);
    sin[i] = Math.sin((-2 * Math.PI * i) / n);
  }
  const bits = Math.log2(n);
  const rev = new Uint32Array(n);
  for (let i = 0; i < n; i++) {
    let r = 0;
    for (let b = 0; b < bits; b++) r |= ((i >> b) & 1) << (bits - 1 - b);
    rev[i] = r;
  }
  // 4-term Blackman-Harris: sidelobes below -92 dB, so a strong harmonic
  // doesn't hide the quiet aliases between its neighbours
  const win = new Float64Array(n);
  let sum = 0;
  for (let i = 0; i < n; i++) {
    const x = (2 * Math.PI * i) / (n - 1);
    win[i] = 0.35875 - 0.48829 * Math.cos(x) + 0.14128 * Math.cos(2 * x) - 0.01168 * Math.cos(3 * x);
    sum += win[i];
  }
  t = { cos, sin, rev, win, gain: 2 / sum };
  cache.set(n, t);
  return t;
}

/** dB magnitude (0 dB = a full-scale sine) of bins 0..n/2 of `x` (length n, a power of two). */
export function spectrumDb(x: Float32Array, out: Float32Array): void {
  const n = x.length;
  const { cos, sin, rev, win, gain } = tables(n);
  const re = new Float64Array(n);
  const im = new Float64Array(n);
  for (let i = 0; i < n; i++) re[rev[i]] = x[i] * win[i];
  for (let len = 2; len <= n; len *= 2) {
    const half = len / 2;
    const step = n / len;
    for (let s = 0; s < n; s += len) {
      for (let k = 0; k < half; k++) {
        const c = cos[k * step];
        const si = sin[k * step];
        const a = s + k;
        const b = a + half;
        const tr = re[b] * c - im[b] * si;
        const ti = re[b] * si + im[b] * c;
        re[b] = re[a] - tr;
        im[b] = im[a] - ti;
        re[a] += tr;
        im[a] += ti;
      }
    }
  }
  for (let k = 0; k <= n / 2; k++) out[k] = 20 * Math.log10(Math.hypot(re[k], im[k]) * gain + 1e-12);
}
