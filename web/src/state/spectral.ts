// The Spectral type's drawn filter (a gain at 64 points from 20 Hz to
// 20 kHz, per oscillator), and turning a picture into a spectrogram to play.

import { CONST } from '../gen/protocol';

export const FILTER_POINTS = 64;
const SIZE = 2048;
const HOP = 512;
const PRE = 2;
const BINS = SIZE / 2 + 1;
const FRAME_FLOATS = 2 * BINS + 1;

export interface FilterSink {
  setSpectralFilter(osc: number, points: Float32Array): void;
}

export class SpectralFilters {
  readonly points: Float32Array[] = Array.from({ length: CONST.oscCount }, () => new Float32Array(FILTER_POINTS).fill(1));
  private sink: FilterSink | null = null;
  private readonly subs = new Set<(osc: number) => void>();

  subscribe(fn: (osc: number) => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  attach(sink: FilterSink | null): void {
    this.sink = sink;
  }

  resync(): void {
    this.points.forEach((p, o) => this.sink?.setSpectralFilter(o, p));
  }

  set(o: number, pts: ArrayLike<number>): void {
    const p = this.points[o];
    for (let i = 0; i < FILTER_POINTS; i++) p[i] = Math.max(0, Math.min(1, pts[i] ?? 1));
    this.sink?.setSpectralFilter(o, p);
    for (const fn of this.subs) fn(o);
  }

  reset(o: number): void {
    this.set(o, new Float32Array(FILTER_POINTS).fill(1));
  }

  isFlat(o: number): boolean {
    return this.points[o].every((v) => v === 1);
  }
}

/**
 * A picture as a spectral analysis: columns are time (spread over
 * `seconds`), rows are frequency on a log scale from 20 Hz (bottom) to
 * 20 kHz (top), brightness is level. Phases are random, which is what
 * makes a still image sound like a texture rather than a click.
 */
export function pictureToAnalysis(img: { width: number; height: number; data: Uint8ClampedArray }, seconds: number, rate: number, seed = 1): { frames: number; rate: number; data: Float32Array } {
  const len = Math.max(HOP, Math.round(seconds * rate));
  const frames = PRE + Math.ceil(len / HOP) + 1;
  const data = new Float32Array(frames * FRAME_FLOATS);
  let s = seed >>> 0 || 1;
  const rand = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  // full brightness, before the level is set at the end
  const full = SIZE / 4;
  const logLo = Math.log(20);
  const logHi = Math.log(20_000);
  for (let m = PRE; m < frames - 1; m++) {
    const col = Math.min(img.width - 1, Math.floor(((m - PRE) / (frames - PRE - 1)) * img.width));
    const f = data.subarray(m * FRAME_FLOATS, (m + 1) * FRAME_FLOATS);
    for (let k = 1; k < BINS - 1; k++) {
      const hz = (k * rate) / SIZE;
      if (hz < 20 || hz > 20_000) continue;
      const y = (Math.log(hz) - logLo) / (logHi - logLo);
      const row = Math.min(img.height - 1, Math.max(0, Math.round((1 - y) * (img.height - 1))));
      const i = (row * img.width + col) * 4;
      const lum = (0.2126 * img.data[i] + 0.7152 * img.data[i + 1] + 0.0722 * img.data[i + 2]) / 255;
      // brightness to level on a 60 dB scale, so dark greys are quiet, not silent
      const db = (lum - 1) * 60;
      f[k] = lum > 0.02 ? full * 10 ** (db / 20) : 0;
      f[BINS + k] = (rand() * 2 - 1) * Math.PI;
    }
  }
  // many lit bins add up: scale the loudest column to about -14 dBFS RMS
  let loudest = 0;
  for (let m = 0; m < frames; m++) {
    let e = 0;
    for (let k = 1; k < BINS - 1; k++) e += (data[m * FRAME_FLOATS + k] / (SIZE / 4)) ** 2 * 0.5;
    loudest = Math.max(loudest, Math.sqrt(e));
  }
  if (loudest > 0) {
    const g = 0.2 / loudest;
    for (let m = 0; m < frames; m++) for (let k = 0; k < BINS; k++) data[m * FRAME_FLOATS + k] *= g;
  }
  return { frames, rate, data };
}
