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
  wt_asset_alloc(bytes: number): number;
  wt_asset_free(ptr: number, bytes: number): void;
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

  /** Copy prepared data into a fresh engine asset; returns its pointer (the engine owns it once a command hands it over). */
  asset(data: Float32Array): number {
    const bytes = data.byteLength;
    const ptr = this.ex.wt_asset_alloc(bytes);
    if (!ptr) throw new Error('asset allocation failed');
    new Float32Array(this.buf, ptr, data.length).set(data);
    return ptr;
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

type ToolFn = (...a: number[]) => number;
let toolsEx: (Record<string, ToolFn> & { memory: WebAssembly.Memory }) | null = null;

/** The tools module, in Node (for test data such as impulse responses). */
export function toolsWasm() {
  if (!toolsEx) {
    const mod = new WebAssembly.Module(fs.readFileSync(path.join(ROOT, 'web', 'wasm', 'tools.wasm')));
    toolsEx = new WebAssembly.Instance(mod, {}).exports as unknown as Record<string, ToolFn> & { memory: WebAssembly.Memory };
  }
  return toolsEx;
}

/** A factory impulse response prepared for the convolver: [taps, data]. */
export function factoryIr(index: number, sr: number): [number, Float32Array] {
  const t = toolsWasm();
  const taps = t.tl_ir_taps(index, sr);
  const l = t.tl_alloc(taps * 4);
  const r = t.tl_alloc(taps * 4);
  t.tl_ir_build(index, sr, l, r);
  const len = t.tl_ir_prepared_len(taps);
  const dst = t.tl_alloc(len * 4);
  t.tl_ir_prepare(l, r, taps, dst);
  const data = new Float32Array(t.memory.buffer, dst, len).slice();
  t.tl_free(l, taps * 4);
  t.tl_free(r, taps * 4);
  t.tl_free(dst, len * 4);
  return [taps, data];
}

/** A factory wavetable, mip-mapped for the engine: [frames, mips]. */
export function factoryTable(name: string): [number, Float32Array] {
  const t = toolsWasm();
  const buf = t.tl_alloc(64);
  let index = -1;
  for (let i = 0; i < t.tl_factory_count() && index < 0; i++) {
    const len = t.tl_factory_name(i, buf, 64);
    if (String.fromCharCode(...new Uint8Array(t.memory.buffer, buf, len)) === name) index = i;
  }
  t.tl_free(buf, 64);
  if (index < 0) throw new Error(`no factory table ${name}`);
  const frames = t.tl_factory_frames(index);
  const raw = frames * t.tl_frame_len() * 4;
  const out = frames * t.tl_frame_stride() * 4;
  const src = t.tl_alloc(raw);
  const dst = t.tl_alloc(out);
  t.tl_factory_build(index, src);
  t.tl_mips(src, frames, dst);
  const mips = new Float32Array(t.memory.buffer, dst, out / 4).slice();
  t.tl_free(src, raw);
  t.tl_free(dst, out);
  return [frames, mips];
}

/** A factory noise: [rate, audio]. */
export function factoryNoise(index: number): [number, Float32Array] {
  const t = toolsWasm();
  const len = t.tl_noise_len();
  const dst = t.tl_alloc(len * 4);
  if (t.tl_noise_build(index, dst) !== 0) throw new Error(`no noise ${index}`);
  const data = new Float32Array(t.memory.buffer, dst, len).slice();
  t.tl_free(dst, len * 4);
  return [t.tl_noise_rate(), data];
}

/** Hand an engine a table, noise or impulse response (what the app's stores do). */
export function loadTable(e: Engine, osc: number, name: string): void {
  const [frames, mips] = factoryTable(name);
  e.w.loadTable(0, osc, frames, e.asset(mips), mips.byteLength);
}

export function loadNoise(e: Engine, index: number): void {
  const [rate, data] = factoryNoise(index);
  e.w.loadSample(0, 0, data.length, rate, e.asset(data), data.byteLength);
}

export function loadIr(e: Engine, inst: number, index: number): void {
  const [taps, data] = factoryIr(index, e.sr);
  e.w.loadIr(0, inst, taps, e.asset(data), data.byteLength);
}

/** Pack a recording with tools.wasm and hand it to an oscillator (Sample and Granular types). */
export function loadRecording(e: Engine, osc: number, x: Float32Array, rate: number): void {
  const t = toolsWasm();
  const n = x.length;
  const len = t.tl_rec_floats(n, 1, 0);
  const src = t.tl_alloc(n * 4);
  const dst = t.tl_alloc(len * 4);
  new Float32Array(t.memory.buffer, src, n).set(x);
  const levels = t.tl_rec_prepare(src, 0, n, src, 0, dst);
  const data = new Float32Array(t.memory.buffer, dst, len).slice();
  t.tl_free(src, n * 4);
  t.tl_free(dst, len * 4);
  e.w.loadOscSample(0, osc, 1, levels, 0, n, rate, e.asset(data), data.byteLength);
}

/** Analyse a recording with tools.wasm for an oscillator's Spectral type. */
export function loadSpectral(e: Engine, osc: number, x: Float32Array, rate: number): void {
  const t = toolsWasm();
  const n = x.length;
  const frames = t.tl_spec_frames(n);
  const len = frames * t.tl_spec_frame_floats();
  const src = t.tl_alloc(n * 4);
  const dst = t.tl_alloc(len * 4);
  new Float32Array(t.memory.buffer, src, n).set(x);
  t.tl_spec_analyze(src, n, rate, dst);
  const data = new Float32Array(t.memory.buffer, dst, len).slice();
  t.tl_free(src, n * 4);
  t.tl_free(dst, len * 4);
  e.w.loadSpectral(0, osc, frames, rate, e.asset(data), data.byteLength);
}

/** A factory multisample from tools.wasm onto an oscillator. */
export function loadMulti(e: Engine, osc: number, index: number): void {
  const t = toolsWasm();
  const len = t.tl_multi_floats(e.sr);
  const dst = t.tl_alloc(len * 4);
  const zones = t.tl_multi_build(index, e.sr, dst);
  const data = new Float32Array(t.memory.buffer, dst, len).slice();
  t.tl_free(dst, len * 4);
  e.w.loadMulti(0, osc, zones, e.asset(data), data.byteLength);
}

/** A few seconds of a bright, moving test recording. */
export function testRecording(rate: number, secs = 3): Float32Array {
  return Float32Array.from({ length: Math.round(rate * secs) }, (_, i) => {
    const t = i / rate;
    let v = 0;
    for (let h = 1; h <= 24; h++) v += Math.sin(2 * Math.PI * 110 * h * t * (1 + 0.001 * Math.sin(t))) / h;
    return v * 0.3 * (0.6 + 0.4 * Math.sin(2 * Math.PI * 0.5 * t));
  });
}
