// Loads the real release engine.wasm in Node and drives it through the same
// ABI the worklet uses.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { CmdWriter } from '../../web/src/gen/protocol';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
export const WASM_PATH = path.join(ROOT, 'web', 'wasm', 'engine.wasm');

export interface Exports {
  memory: WebAssembly.Memory;
  wt_abi_hash(): number;
  wt_init(sr: number): number;
  wt_cmd_ptr(): number;
  wt_cmd_cap(): number;
  wt_apply(len: number): number;
  wt_render(frames: number, start: number): number;
  wt_out_ptr(ch: number): number;
  wt_tel_ptr(): number;
  wt_tel_len(): number;
  wt_tap_ptr(i: number): number;
  wt_tap_count(): number;
  wt_max_block(): number;
  wt_panic_ptr(): number;
  wt_panic_len(): number;
  wt_alloc_count(): number;
  wt_param_count(): number;
  wt_param_plain(id: number, norm: number): number;
}

let cached: WebAssembly.Module | null = null;
export function module(): WebAssembly.Module {
  if (!cached) cached = new WebAssembly.Module(fs.readFileSync(WASM_PATH));
  return cached;
}

export class Engine {
  readonly ex: Exports;
  readonly w = new CmdWriter();
  frame = 0;

  constructor(readonly sr = 48_000) {
    this.ex = new WebAssembly.Instance(module(), {}).exports as unknown as Exports;
    if (this.ex.wt_init(sr) !== 0) throw new Error('wt_init failed');
  }

  private get buf(): ArrayBuffer {
    return this.ex.memory.buffer;
  }

  /** Apply whatever has been written to `w`. */
  apply(): number {
    const bytes = this.w.bytes();
    new Uint8Array(this.buf, this.ex.wt_cmd_ptr(), bytes.length).set(bytes);
    const n = this.ex.wt_apply(bytes.length);
    this.w.clear();
    return n;
  }

  /** Render `frames` (multiple of 128 for convenience) and return the left channel (and right). */
  render(frames: number): { l: Float32Array; r: Float32Array } {
    const l = new Float32Array(frames);
    const r = new Float32Array(frames);
    const q = 128;
    for (let f = 0; f < frames; f += q) {
      if (this.w.length) this.apply();
      const n = Math.min(q, frames - f);
      const rc = this.ex.wt_render(Math.ceil(n / 16) * 16, this.frame);
      if (rc !== 0) throw new Error(`wt_render returned ${rc}`);
      l.set(new Float32Array(this.buf, this.ex.wt_out_ptr(0), n), f);
      r.set(new Float32Array(this.buf, this.ex.wt_out_ptr(1), n), f);
      this.frame += Math.ceil(n / 16) * 16;
    }
    return { l, r };
  }

  tel(): Float32Array {
    return new Float32Array(this.buf, this.ex.wt_tel_ptr(), this.ex.wt_tel_len()).slice();
  }
}

/**
 * Frequency from rising zero crossings: a least-squares line through every
 * crossing time (interpolated between samples). Using only the first and last
 * crossings scatters by about ±0.01 Hz on 8192 frames; the fit is ~50× tighter.
 */
export function pitch(x: Float32Array, sr: number): number {
  const z: number[] = [];
  for (let i = 1; i < x.length; i++) if (x[i - 1] < 0 && x[i] >= 0) z.push(i - 1 + -x[i - 1] / (x[i] - x[i - 1]));
  const n = z.length;
  const mi = (n - 1) / 2;
  const mz = z.reduce((a, b) => a + b, 0) / n;
  let num = 0;
  let den = 0;
  for (let i = 0; i < n; i++) {
    num += (i - mi) * (z[i] - mz);
    den += (i - mi) ** 2;
  }
  return sr / (num / den);
}
