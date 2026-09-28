// What the tools worker does with each request, given tools.wasm's exports
// (the worker passes its instance; tests pass one made in Node).

import type { ToolsReq } from './protocol';

export interface Exports {
  memory: WebAssembly.Memory;
  tl_alloc(bytes: number): number;
  tl_free(ptr: number, bytes: number): void;
  tl_frame_len(): number;
  tl_frame_stride(): number;
  tl_mips(src: number, frames: number, dst: number): number;
  tl_factory_count(): number;
  tl_factory_frames(i: number): number;
  tl_factory_name(i: number, out: number, cap: number): number;
  tl_factory_build(i: number, dst: number): number;
  tl_resample(src: number, len: number, dst: number): number;
  tl_preview(frame: number, w1: number, a1: number, w2: number, a2: number, remap: number, points: number, dst: number): number;
  tl_noise_len(): number;
  tl_noise_rate(): number;
  tl_noise_build(i: number, dst: number): number;
  tl_filter_response(kind: number, cutoff: number, res: number, v: number, sr: number, f0: number, f1: number, points: number, dst: number): number;
  tl_ir_taps(i: number, sr: number): number;
  tl_ir_build(i: number, sr: number, l: number, r: number): number;
  tl_ir_prepared_len(taps: number): number;
  tl_ir_prepare(l: number, r: number, taps: number, dst: number): number;
  tl_wt_process(op: number, a: number, b: number, frames: number, count: number): number;
  tl_wt_pwm(src: number, count: number, dst: number): number;
  tl_wt_sort(frames: number, count: number, order: number): number;
  tl_wt_morph(keys: number, k: number, target: number, mode: number, dst: number): number;
  tl_wt_analyze(frame: number, mag: number, phase: number): number;
  tl_wt_synthesize(mag: number, phase: number, frame: number): number;
  tl_formula(src: number, len: number, frames: number, count: number, apply: number, selected: number, seed: number): number;
  tl_formula_check(src: number, len: number): number;
  tl_formula_error(out: number, cap: number): number;
  tl_pitch(audio: number, len: number, sr: number): number;
  tl_import(mode: number, audio: number, len: number, sr: number, arg: number, max: number, dst: number): number;
}

const HARMONICS = 1024;

/** ASCII into wasm memory (the formula language is ASCII; anything else is caught there). */
function ascii(ex: Exports, s: string, ptr: number): void {
  const b = new Uint8Array(ex.memory.buffer, ptr, s.length);
  for (let i = 0; i < s.length; i++) b[i] = Math.min(255, s.charCodeAt(i));
}

function formulaError(ex: Exports): string {
  return withBuf(ex, 256, (p) => {
    const n = ex.tl_formula_error(p, 256);
    return String.fromCharCode(...new Uint8Array(ex.memory.buffer, p, n));
  });
}

/** The convolver's longest response (MAX_TAPS in crates/dsp/src/conv.rs); the tools cap anything longer. */
const MAX_IR_TAPS = 2048 + 186 * 1024;

/** Run `fn` with a scratch buffer of `bytes` in wasm memory. */
function withBuf<T>(ex: Exports, bytes: number, fn: (ptr: number) => T): T {
  const ptr = ex.tl_alloc(bytes);
  if (!ptr) throw new Error(`tools: out of memory for ${bytes} bytes`);
  try {
    return fn(ptr);
  } finally {
    ex.tl_free(ptr, bytes);
  }
}

const f32 = (ex: Exports, ptr: number, n: number) => new Float32Array(ex.memory.buffer, ptr, n);

export function handle(ex: Exports, req: ToolsReq): { result: unknown; transfer: Transferable[] } {
  const FL = ex.tl_frame_len();
  const FS = ex.tl_frame_stride();
  switch (req.op) {
    case 'mips': {
      const n = req.count;
      return withBuf(ex, n * FL * 4, (src) => {
        f32(ex, src, n * FL).set(req.frames.subarray(0, n * FL));
        return withBuf(ex, n * FS * 4, (dst) => {
          ex.tl_mips(src, n, dst);
          const out = f32(ex, dst, n * FS).slice();
          return { result: out, transfer: [out.buffer] };
        });
      });
    }
    case 'factoryList': {
      const list = [];
      for (let i = 0; i < ex.tl_factory_count(); i++) {
        const name = withBuf(ex, 64, (p) => {
          const len = ex.tl_factory_name(i, p, 64);
          return new TextDecoder().decode(new Uint8Array(ex.memory.buffer, p, len));
        });
        list.push({ name, frames: ex.tl_factory_frames(i) });
      }
      return { result: list, transfer: [] };
    }
    case 'factory': {
      const n = ex.tl_factory_frames(req.index);
      if (!n) throw new Error(`no factory table ${req.index}`);
      return withBuf(ex, n * FL * 4, (dst) => {
        ex.tl_factory_build(req.index, dst);
        const frames = f32(ex, dst, n * FL).slice();
        return { result: { frames, count: n }, transfer: [frames.buffer] };
      });
    }
    case 'resample': {
      const len = req.cycle.length;
      return withBuf(ex, len * 4, (src) => {
        f32(ex, src, len).set(req.cycle);
        return withBuf(ex, FL * 4, (dst) => {
          ex.tl_resample(src, len, dst);
          const out = f32(ex, dst, FL).slice();
          return { result: out, transfer: [out.buffer] };
        });
      });
    }
    case 'preview': {
      const remap = req.remap;
      return withBuf(ex, FL * 4, (src) => {
        f32(ex, src, FL).set(req.frame);
        return withBuf(ex, (remap?.length ?? 1) * 4, (rp) => {
          if (remap) f32(ex, rp, remap.length).set(remap);
          return withBuf(ex, req.points * 4, (dst) => {
            ex.tl_preview(src, req.w1[0], req.w1[1], req.w2[0], req.w2[1], remap ? rp : 0, req.points, dst);
            const out = f32(ex, dst, req.points).slice();
            return { result: out, transfer: [out.buffer] };
          });
        });
      });
    }
    case 'filterResponse': {
      return withBuf(ex, req.points * 4, (dst) => {
        ex.tl_filter_response(req.kind, req.cutoff, req.res, req.var, req.sr, req.f0, req.f1, req.points, dst);
        const out = f32(ex, dst, req.points).slice();
        return { result: out, transfer: [out.buffer] };
      });
    }
    case 'irFactory': {
      const n = ex.tl_ir_taps(req.index, req.sr);
      return withBuf(ex, n * 4, (lp) =>
        withBuf(ex, n * 4, (rp) => {
          ex.tl_ir_build(req.index, req.sr, lp, rp);
          const l = f32(ex, lp, n).slice();
          const r = f32(ex, rp, n).slice();
          return { result: { l, r }, transfer: [l.buffer, r.buffer] };
        }),
      );
    }
    case 'irPrepare': {
      const taps = Math.max(req.l.length, req.r.length);
      const len = ex.tl_ir_prepared_len(taps);
      return withBuf(ex, taps * 4, (lp) =>
        withBuf(ex, taps * 4, (rp) => {
          f32(ex, lp, taps).fill(0).set(req.l.subarray(0, taps));
          f32(ex, rp, taps).fill(0).set(req.r.subarray(0, taps));
          return withBuf(ex, len * 4, (dst) => {
            ex.tl_ir_prepare(lp, rp, taps, dst);
            const data = f32(ex, dst, len).slice();
            return { result: { data, taps: Math.min(taps, MAX_IR_TAPS) }, transfer: [data.buffer] };
          });
        }),
      );
    }
    case 'wtProcess': {
      const n = req.count;
      return withBuf(ex, n * FL * 4, (p) => {
        f32(ex, p, n * FL).set(req.frames.subarray(0, n * FL));
        if (ex.tl_wt_process(req.kind, req.a, req.b, p, n) !== 0) throw new Error(`unknown process ${req.kind}`);
        const out = f32(ex, p, n * FL).slice();
        return { result: out, transfer: [out.buffer] };
      });
    }
    case 'wtPwm': {
      const n = Math.max(1, req.count);
      return withBuf(ex, FL * 4, (src) => {
        f32(ex, src, FL).set(req.frame.subarray(0, FL));
        return withBuf(ex, n * FL * 4, (dst) => {
          ex.tl_wt_pwm(src, n, dst);
          const out = f32(ex, dst, n * FL).slice();
          return { result: out, transfer: [out.buffer] };
        });
      });
    }
    case 'wtSort': {
      const n = req.count;
      return withBuf(ex, n * FL * 4, (p) => {
        f32(ex, p, n * FL).set(req.frames.subarray(0, n * FL));
        return withBuf(ex, n * 4, (o) => {
          ex.tl_wt_sort(p, n, o);
          const out = new Uint32Array(ex.memory.buffer, o, n).slice();
          return { result: out, transfer: [out.buffer] };
        });
      });
    }
    case 'wtMorph': {
      const k = req.count;
      const target = Math.max(k, req.target);
      return withBuf(ex, k * FL * 4, (keys) => {
        f32(ex, keys, k * FL).set(req.keys.subarray(0, k * FL));
        return withBuf(ex, target * FL * 4, (dst) => {
          const n = ex.tl_wt_morph(keys, k, target, req.mode, dst);
          const out = f32(ex, dst, n * FL).slice();
          return { result: out, transfer: [out.buffer] };
        });
      });
    }
    case 'wtAnalyze': {
      const n = HARMONICS + 1;
      return withBuf(ex, FL * 4, (fp) =>
        withBuf(ex, n * 4, (mp) =>
          withBuf(ex, n * 4, (pp) => {
            f32(ex, fp, FL).set(req.frame.subarray(0, FL));
            ex.tl_wt_analyze(fp, mp, pp);
            const mag = f32(ex, mp, n).slice();
            const phase = f32(ex, pp, n).slice();
            return { result: { mag, phase }, transfer: [mag.buffer, phase.buffer] };
          }),
        ),
      );
    }
    case 'wtSynthesize': {
      const n = HARMONICS + 1;
      return withBuf(ex, FL * 4, (fp) =>
        withBuf(ex, n * 4, (mp) =>
          withBuf(ex, n * 4, (pp) => {
            f32(ex, mp, n).set(req.mag.subarray(0, n));
            f32(ex, pp, n).set(req.phase.subarray(0, n));
            ex.tl_wt_synthesize(mp, pp, fp);
            const out = f32(ex, fp, FL).slice();
            return { result: out, transfer: [out.buffer] };
          }),
        ),
      );
    }
    case 'formula': {
      const n = req.count;
      const len = req.src.length;
      return withBuf(ex, Math.max(1, len), (sp) =>
        withBuf(ex, n * FL * 4, (fp) =>
          withBuf(ex, n, (ap) =>
            withBuf(ex, n, (selp) => {
              ascii(ex, req.src, sp);
              f32(ex, fp, n * FL).set(req.frames.subarray(0, n * FL));
              new Uint8Array(ex.memory.buffer, ap, n).set(req.apply.subarray(0, n));
              new Uint8Array(ex.memory.buffer, selp, n).set(req.selected.subarray(0, n));
              const pos = ex.tl_formula(sp, len, fp, n, ap, selp, req.seed >>> 0);
              const frames = pos < 0 ? f32(ex, fp, n * FL).slice() : req.frames;
              return { result: { frames, error: pos < 0 ? null : formulaError(ex), pos: Math.max(0, pos) }, transfer: pos < 0 ? [frames.buffer] : [] };
            }),
          ),
        ),
      );
    }
    case 'formulaCheck': {
      const len = req.src.length;
      return withBuf(ex, Math.max(1, len), (sp) => {
        ascii(ex, req.src, sp);
        const pos = ex.tl_formula_check(sp, len);
        return { result: { error: pos < 0 ? null : formulaError(ex), pos: Math.max(0, pos) }, transfer: [] };
      });
    }
    case 'pitch': {
      const len = req.audio.length;
      return withBuf(ex, len * 4, (p) => {
        f32(ex, p, len).set(req.audio);
        return { result: ex.tl_pitch(p, len, req.sr), transfer: [] };
      });
    }
    case 'import': {
      const len = req.audio.length;
      const max = Math.max(1, Math.min(256, req.max));
      return withBuf(ex, len * 4, (ap) =>
        withBuf(ex, max * FL * 4, (dst) => {
          f32(ex, ap, len).set(req.audio);
          const count = ex.tl_import(req.mode, ap, len, req.sr, req.arg, max, dst);
          const frames = f32(ex, dst, count * FL).slice();
          return { result: { frames, count }, transfer: [frames.buffer] };
        }),
      );
    }
    case 'noise': {
      const n = ex.tl_noise_len();
      return withBuf(ex, n * 4, (dst) => {
        if (ex.tl_noise_build(req.index, dst) !== 0) throw new Error(`no noise ${req.index}`);
        const data = f32(ex, dst, n).slice();
        return { result: { data, rate: ex.tl_noise_rate() }, transfer: [data.buffer] };
      });
    }
  }
}

