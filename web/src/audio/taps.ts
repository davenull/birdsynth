// Recent history of each tap, kept on the main thread for scopes and the
// explainer. Every sample is stamped with its absolute audio frame, so a
// scope can draw what the listener is hearing now rather than what the
// worklet rendered last (the superhet site's approach).

import { TAP_NAMES } from '../gen/protocol';

const HISTORY = 16_384; // frames kept per tap, about 1/3 s at 48 kHz

export class TapStore {
  private readonly rings = TAP_NAMES.map(() => new Float32Array(HISTORY));
  /** Absolute frame just past the newest sample, per tap (-1: nothing yet). */
  private readonly end = TAP_NAMES.map(() => -1);

  push(tap: number, frame: number, data: Float32Array): void {
    const ring = this.rings[tap];
    const n = Math.min(data.length, HISTORY);
    const src = data.length > HISTORY ? data.subarray(data.length - HISTORY) : data;
    const first = frame + data.length - n;
    const at = first % HISTORY;
    const k = Math.min(n, HISTORY - at);
    ring.set(src.subarray(0, k), at);
    if (k < n) ring.set(src.subarray(k, n), 0);
    this.end[tap] = first + n;
  }

  /** Newest frame available for a tap, or -1. */
  latest(tap: number): number {
    return this.end[tap];
  }

  /**
   * Copy the frames [endFrame - out.length, endFrame) into out. Returns false
   * if that span isn't in the history (too old, or not recorded yet).
   */
  read(tap: number, endFrame: number, out: Float32Array): boolean {
    const n = out.length;
    const end = this.end[tap];
    if (end < 0 || endFrame > end || endFrame - n < end - HISTORY || n > HISTORY) return false;
    const ring = this.rings[tap];
    const start = (((endFrame - n) % HISTORY) + HISTORY) % HISTORY;
    const k = Math.min(n, HISTORY - start);
    out.set(ring.subarray(start, start + k), 0);
    if (k < n) out.set(ring.subarray(0, n - k), k);
    return true;
  }
}
