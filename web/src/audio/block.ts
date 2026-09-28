// Layout of the blocks the worklet sends back to the main thread: one per
// BLOCK_FRAMES of audio, carrying the latest telemetry and the recorded taps.
// Blocks are pooled ArrayBuffers, transferred back and forth, never allocated
// while rendering.
//
//   byte 0   f64  frame of the block's first sample
//        8   u32  frames in the block
//       12   u32  tap mask (which taps follow, in index order)
//       16   f32  engine CPU load, percent of real time
//       20   u32  blocks dropped because the pool was empty
//       24   u32  engine traps recovered so far
//       28   u32  reserved
//       32   f32  telemetry[TEL.LEN] (peaks are the maximum over the block)
//       ...  f32  taps: BLOCK_FRAMES floats per tap in the mask

import { TEL } from '../gen/protocol';

export const BLOCK_FRAMES = 1024;
export const BLOCK_MAX_TAPS = 8;
export const POOL_SIZE = 8;

export const HDR = {
  frame: 0, // f64 index
  frames: 2, // u32 index
  tapMask: 3,
  cpu: 4, // f32 index
  dropped: 5,
  traps: 6,
} as const;

export const TEL_AT = 8; // f32 index
export const TAPS_AT = TEL_AT + Math.ceil(TEL.LEN / 4) * 4; // f32 index, 16-byte aligned
export const BLOCK_BYTES = (TAPS_AT + BLOCK_MAX_TAPS * BLOCK_FRAMES) * 4;

export interface BlockViews {
  buf: ArrayBuffer;
  f32: Float32Array;
  u32: Uint32Array;
  f64: Float64Array;
}

export function views(buf: ArrayBuffer): BlockViews {
  return { buf, f32: new Float32Array(buf), u32: new Uint32Array(buf), f64: new Float64Array(buf, 0, 1) };
}

// Messages between the host and the processor.
export type ToWorklet =
  | { t: 'cmd'; b: ArrayBuffer }
  | { t: 'taps'; mask: number }
  | { t: 'pool'; bufs: ArrayBuffer[] }
  /** A whole mip-mapped table for an oscillator (frames × frameStride floats). */
  | { t: 'table'; osc: number; frames: number; data: ArrayBuffer }
  /** One mip-mapped frame to overwrite in an oscillator's table. */
  | { t: 'frame'; osc: number; index: number; data: ArrayBuffer }
  /** Mono f32 audio for a sample slot (0: the noise oscillator). */
  | { t: 'sample'; slot: number; frames: number; rate: number; data: ArrayBuffer };

/** Largest slice of an upload copied into wasm memory per render quantum. */
export const UPLOAD_CHUNK = 512 * 1024;

export type FromWorklet =
  | { t: 'ready'; abi: number; sampleRate: number }
  | { t: 'trap'; msg: string; traps: number }
  | { t: 'fatal'; msg: string }
  | { t: 'log'; msg: string };
