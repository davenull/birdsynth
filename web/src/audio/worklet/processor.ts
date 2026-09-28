// The AudioWorklet processor: runs engine.wasm on the audio thread.
//
// The main thread passes the .wasm bytes in processorOptions; they're
// compiled here once and kept, so recovering from a trap only needs a new
// Instance. Commands arrive as binary batches and are copied straight into
// the engine's command buffer. Nothing in process() allocates typed arrays:
// every view is made when the engine is instantiated or when a pooled
// block comes back from the main thread.

import { ABI_HASH, CmdWriter, TEL } from '../../gen/protocol';
import { BLOCK_FRAMES, BLOCK_MAX_TAPS, HDR, TAPS_AT, TEL_AT, views, type BlockViews, type FromWorklet, type ToWorklet } from '../block';

// AudioWorkletGlobalScope globals, declared here so the DOM typings stay
// untouched (they would clash with a global worklet lib).
declare const sampleRate: number;
declare const currentFrame: number;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor(options?: AudioWorkletNodeOptions);
}
declare function registerProcessor(name: string, ctor: new (options: AudioWorkletNodeOptions) => AudioWorkletProcessor): void;

interface EngineExports {
  memory: WebAssembly.Memory;
  wt_abi_hash(): number;
  wt_init(sampleRate: number): number;
  wt_cmd_ptr(): number;
  wt_cmd_cap(): number;
  wt_apply(len: number): number;
  wt_render(frames: number, start: number): number;
  wt_out_ptr(ch: number): number;
  wt_tel_ptr(): number;
  wt_tel_len(): number;
  wt_tap_ptr(i: number): number;
  wt_tap_count(): number;
  wt_panic_ptr(): number;
  wt_panic_len(): number;
}

type Resizable = WebAssembly.Memory & { toResizableBuffer?: () => ArrayBuffer };

/** About 0.5 s rendered at startup with notes held, so V8 optimizes the render path before anything is heard. */
const WARMUP_FRAMES = 24_576;

/** Keep only the first BLOCK_MAX_TAPS taps of a mask: that's all a block carries. */
function limitMask(mask: number): number {
  let out = 0;
  let n = 0;
  for (let i = 0; i < 32 && n < BLOCK_MAX_TAPS; i++) {
    if (mask & (1 << i)) {
      out |= 1 << i;
      n++;
    }
  }
  return out >>> 0;
}

function ascii(b: Uint8Array): string {
  let s = '';
  for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]);
  return s;
}

class WtProcessor extends AudioWorkletProcessor {
  private readonly module: WebAssembly.Module;
  private ex!: EngineExports;
  private mem!: WebAssembly.Memory;
  private buf!: ArrayBuffer;
  private resizable = false;
  private dead = false;

  // views into wasm memory, sized for one render quantum
  private quantum = 128;
  private cmd!: Uint8Array;
  private cmdLen = 0;
  private outL!: Float32Array;
  private outR!: Float32Array;
  private tel!: Float32Array;
  private tapViews: Float32Array[] = [];
  private tapMask = 0;
  private readonly local = new CmdWriter(4096);

  // blocks going back to the main thread
  private readonly pool: BlockViews[] = [];
  private cur: BlockViews | null = null;
  private blockPos = 0;
  private blockStart = 0;
  private peakL = 0;
  private peakR = 0;
  private dropped = 0;
  private traps = 0;

  // CPU load: summed Date.now() deltas (millisecond ticks) over about a second
  private cpuMs = 0;
  private cpuFrames = 0;
  private cpuPct = 0;

  constructor(options: AudioWorkletNodeOptions) {
    super(options);
    const opts = options.processorOptions as { wasm: ArrayBuffer };
    this.port.onmessage = (e: MessageEvent) => this.onMessage(e.data);
    this.port.onmessageerror = () => this.post({ t: 'log', msg: 'worklet: a message from the main thread could not be deserialized' });
    this.module = new WebAssembly.Module(opts.wasm);
    try {
      this.instantiate();
      this.warmUp();
      this.post({ t: 'ready', abi: this.ex.wt_abi_hash() >>> 0, sampleRate });
    } catch (e) {
      this.dead = true;
      this.post({ t: 'fatal', msg: String((e as Error)?.message ?? e) });
    }
  }

  private post(msg: FromWorklet): void {
    this.port.postMessage(msg);
  }

  private instantiate(): void {
    const ex = new WebAssembly.Instance(this.module, {}).exports as unknown as EngineExports;
    if (ex.wt_abi_hash() >>> 0 !== ABI_HASH) {
      throw new Error(`engine.wasm ABI 0x${(ex.wt_abi_hash() >>> 0).toString(16)} does not match the host's 0x${ABI_HASH.toString(16)}; rebuild with npm run build:wasm`);
    }
    if (ex.wt_init(sampleRate) !== 0) throw new Error(`the engine rejected sample rate ${sampleRate}`);
    this.ex = ex;
    this.mem = ex.memory;
    const r = this.mem as Resizable;
    this.resizable = typeof r.toResizableBuffer === 'function';
    this.buf = this.resizable ? r.toResizableBuffer!() : this.mem.buffer;
    this.makeViews();
    this.cmdLen = 0;
    if (this.tapMask) this.localCmd((w) => w.setTaps(0, this.tapMask));
  }

  private makeViews(): void {
    const ex = this.ex;
    const q = this.quantum;
    this.cmd = new Uint8Array(this.buf, ex.wt_cmd_ptr(), ex.wt_cmd_cap());
    this.outL = new Float32Array(this.buf, ex.wt_out_ptr(0), q);
    this.outR = new Float32Array(this.buf, ex.wt_out_ptr(1), q);
    this.tel = new Float32Array(this.buf, ex.wt_tel_ptr(), ex.wt_tel_len());
    this.tapViews = [];
    for (let i = 0; i < ex.wt_tap_count(); i++) this.tapViews.push(new Float32Array(this.buf, ex.wt_tap_ptr(i), q));
  }

  /** Queue commands the processor itself issues (tap mask, warm-up). */
  private localCmd(fill: (w: CmdWriter) => void): void {
    this.local.clear();
    fill(this.local);
    this.pushCmd(this.local.bytes());
  }

  private pushCmd(src: Uint8Array): void {
    if (this.cmdLen + src.length > this.cmd.length) this.applyPending();
    if (src.length > this.cmd.length) {
      this.post({ t: 'log', msg: `worklet: dropped a ${src.length}-byte command batch (the host must split batches)` });
      return;
    }
    this.cmd.set(src, this.cmdLen);
    this.cmdLen += src.length;
  }

  private applyPending(): void {
    if (this.cmdLen === 0 || this.dead) return;
    const len = this.cmdLen;
    this.cmdLen = 0;
    try {
      this.ex.wt_apply(len);
    } catch (e) {
      this.recover(e);
    }
  }

  private warmUp(): void {
    this.localCmd((w) => {
      for (let i = 0; i < 8; i++) w.noteOn(0, 36 + i * 7, 0, 1, 0x7fff_0000 + i);
    });
    this.applyPending();
    for (let f = 0; f < WARMUP_FRAMES; f += this.quantum) this.ex.wt_render(this.quantum, -1);
    this.localCmd((w) => w.reset(0));
    this.applyPending();
  }

  private onMessage(d: ArrayBuffer | ToWorklet): void {
    if (d instanceof ArrayBuffer) {
      this.pool.push(views(d)); // a block coming back
      return;
    }
    switch (d.t) {
      case 'cmd':
        if (!this.dead) this.pushCmd(new Uint8Array(d.b));
        break;
      case 'taps':
        this.tapMask = limitMask(d.mask);
        if (!this.dead) this.localCmd((w) => w.setTaps(0, this.tapMask));
        break;
      case 'pool':
        for (const b of d.bufs) this.pool.push(views(b));
        break;
    }
  }

  /** The engine trapped: report it and start a fresh instance. The main thread resends the patch. */
  private recover(err: unknown): void {
    let msg = String((err as Error)?.message ?? err);
    try {
      const n = this.ex.wt_panic_len();
      if (n > 0) msg = ascii(new Uint8Array(this.mem.buffer, this.ex.wt_panic_ptr(), n));
    } catch {
      // the old instance is unusable; keep the JS error text
    }
    this.traps++;
    try {
      this.instantiate();
      this.post({ t: 'trap', msg, traps: this.traps });
    } catch (e) {
      this.dead = true;
      this.post({ t: 'fatal', msg: `${msg}; restart failed: ${String((e as Error)?.message ?? e)}` });
    }
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const out = outputs[0];
    const L = out[0];
    const R = out[1] ?? out[0];
    if (this.dead) {
      L.fill(0);
      R.fill(0);
      return true;
    }
    const n = L.length;
    const grew = !this.resizable && this.mem.buffer !== this.buf;
    if (grew) this.buf = this.mem.buffer; // memory grew and detached the old views
    if (grew || n !== this.quantum) {
      this.quantum = n; // quanta are 128 frames today, but stay correct if that changes
      this.makeViews();
    }
    this.applyPending();
    try {
      const t0 = Date.now();
      this.ex.wt_render(n, currentFrame);
      this.cpuMs += Date.now() - t0;
      L.set(this.outL);
      if (R !== L) R.set(this.outR);
      this.collect(n);
    } catch (e) {
      L.fill(0);
      R.fill(0);
      this.recover(e);
    }
    this.cpuFrames += n;
    if (this.cpuFrames >= sampleRate) {
      this.cpuPct = (this.cpuMs / ((this.cpuFrames / sampleRate) * 1000)) * 100;
      this.cpuMs = 0;
      this.cpuFrames = 0;
    }
    return true;
  }

  /** Copy this quantum's taps and telemetry into the current block; send it when full. */
  private collect(n: number): void {
    if (!this.cur) {
      const next = this.pool.pop();
      if (!next) {
        this.dropped++;
        return;
      }
      this.cur = next;
      this.blockPos = 0;
    }
    const blk = this.cur;
    if (this.blockPos === 0) {
      this.blockStart = currentFrame;
      this.peakL = 0;
      this.peakR = 0;
    }
    let k = 0;
    for (let i = 0; i < this.tapViews.length && k < BLOCK_MAX_TAPS; i++) {
      if (this.tapMask & (1 << i)) {
        blk.f32.set(this.tapViews[i], TAPS_AT + k * BLOCK_FRAMES + this.blockPos);
        k++;
      }
    }
    const tel = this.tel;
    if (tel[TEL.peakL] > this.peakL) this.peakL = tel[TEL.peakL];
    if (tel[TEL.peakR] > this.peakR) this.peakR = tel[TEL.peakR];
    this.blockPos += n;
    if (this.blockPos + n > BLOCK_FRAMES) this.send();
  }

  private send(): void {
    const blk = this.cur!;
    blk.f64[HDR.frame] = this.blockStart;
    blk.u32[HDR.frames] = this.blockPos;
    blk.u32[HDR.tapMask] = this.tapMask;
    blk.f32[HDR.cpu] = this.cpuPct;
    blk.u32[HDR.dropped] = this.dropped;
    blk.u32[HDR.traps] = this.traps;
    blk.f32.set(this.tel, TEL_AT);
    blk.f32[TEL_AT + TEL.peakL] = this.peakL;
    blk.f32[TEL_AT + TEL.peakR] = this.peakR;
    this.port.postMessage(blk.buf, [blk.buf]);
    this.cur = null;
    this.blockPos = 0;
  }
}

registerProcessor('wt-engine', WtProcessor);
