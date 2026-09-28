// The tools Web Worker: runs tools.wasm (mip building, factory tables,
// resampling, previews) off the main thread.

import toolsUrl from '../../wasm/tools.wasm?url';
import type { ToolsReq, ToolsRes } from './protocol';

interface Exports {
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
  tl_preview(frame: number, w1: number, a1: number, w2: number, a2: number, points: number, dst: number): number;
  tl_noise_len(): number;
  tl_noise_rate(): number;
  tl_noise_build(i: number, dst: number): number;
}

const ready: Promise<Exports> = WebAssembly.instantiateStreaming(fetch(toolsUrl), {}).then((r) => r.instance.exports as unknown as Exports);

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

function handle(ex: Exports, req: ToolsReq): { result: unknown; transfer: Transferable[] } {
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
      return withBuf(ex, FL * 4, (src) => {
        f32(ex, src, FL).set(req.frame);
        return withBuf(ex, req.points * 4, (dst) => {
          ex.tl_preview(src, req.w1[0], req.w1[1], req.w2[0], req.w2[1], req.points, dst);
          const out = f32(ex, dst, req.points).slice();
          return { result: out, transfer: [out.buffer] };
        });
      });
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

self.onmessage = async (e: MessageEvent<ToolsReq & { id: number }>) => {
  const { id } = e.data;
  try {
    const ex = await ready;
    const { result, transfer } = handle(ex, e.data);
    (self as unknown as Worker).postMessage({ id, ok: true, result } satisfies ToolsRes, transfer);
  } catch (err) {
    (self as unknown as Worker).postMessage({ id, ok: false, error: String((err as Error)?.message ?? err) } satisfies ToolsRes);
  }
};
