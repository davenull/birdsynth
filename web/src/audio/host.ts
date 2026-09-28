// Main-thread side of the engine: creates the AudioContext and the worklet,
// sends command batches, and unpacks the telemetry and tap blocks that come
// back.

import processorUrl from './worklet/processor.ts?worker&url';
import engineUrl from '../../wasm/engine.wasm?url';
import { ABI_HASH, CmdWriter, TAP_NAMES, TEL, type TapName } from '../gen/protocol';
import { BLOCK_BYTES, BLOCK_FRAMES, HDR, POOL_SIZE, TAPS_AT, TEL_AT, type FromWorklet, type ToWorklet } from './block';
import { TapStore } from './taps';

export interface HostEvents {
  /** The engine trapped and restarted empty; resend the patch. */
  onTrap?(msg: string, traps: number): void;
  onFatal?(msg: string): void;
}

export class EngineHost {
  readonly taps = new TapStore();
  /** Latest telemetry (see TEL). */
  readonly tel = new Float32Array(TEL.LEN);
  cpuPct = 0;
  dropped = 0;
  traps = 0;
  blocks = 0;
  /** Frame just past the newest block. */
  frame = 0;

  private readonly w = new CmdWriter(1 << 16);
  private flushQueued = false;
  private readonly ready: Promise<void>;
  private readonly listeners = new Set<() => void>();

  private constructor(
    readonly ctx: AudioContext,
    readonly node: AudioWorkletNode,
    private readonly events: HostEvents,
  ) {
    let resolve!: () => void;
    let reject!: (e: Error) => void;
    this.ready = new Promise<void>((res, rej) => ((resolve = res), (reject = rej)));
    node.port.onmessage = (e: MessageEvent<ArrayBuffer | FromWorklet>) => {
      const d = e.data;
      if (d instanceof ArrayBuffer) return this.onBlock(d);
      switch (d.t) {
        case 'ready':
          if (d.abi !== ABI_HASH) reject(new Error('engine.wasm is out of date'));
          else resolve();
          break;
        case 'trap':
          this.traps = d.traps;
          console.warn(`engine trapped and restarted: ${d.msg}`);
          this.events.onTrap?.(d.msg, d.traps);
          break;
        case 'fatal':
          console.error(`engine failed: ${d.msg}`);
          reject(new Error(d.msg));
          this.events.onFatal?.(d.msg);
          break;
        case 'log':
          console.warn(d.msg);
          break;
      }
    };
    node.port.onmessageerror = () => console.error('a message from the audio worklet could not be deserialized');
    const bufs = Array.from({ length: POOL_SIZE }, () => new ArrayBuffer(BLOCK_BYTES));
    this.post({ t: 'pool', bufs }, bufs);
  }

  static async create(events: HostEvents = {}): Promise<EngineHost> {
    if (!window.isSecureContext) throw new Error('audio needs a secure page (https or localhost)');
    const ctx = new AudioContext({ latencyHint: 'interactive' });
    if (!ctx.audioWorklet) throw new Error('this browser has no AudioWorklet support');
    const [wasm] = await Promise.all([
      fetch(engineUrl).then((r) => {
        if (!r.ok) throw new Error(`engine.wasm: HTTP ${r.status}`);
        return r.arrayBuffer();
      }),
      ctx.audioWorklet.addModule(processorUrl),
    ]);
    const node = new AudioWorkletNode(ctx, 'wt-engine', {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { wasm },
    });
    const host = new EngineHost(ctx, node, events);
    await host.ready;
    node.connect(ctx.destination);
    return host;
  }

  private post(msg: ToWorklet, transfer: Transferable[] = []): void {
    this.node.port.postMessage(msg, transfer);
  }

  /** Append commands; they're sent together at the end of the current task. */
  send(fill: (w: CmdWriter) => void): void {
    fill(this.w);
    if (!this.flushQueued) {
      this.flushQueued = true;
      queueMicrotask(() => this.flush());
    }
  }

  flush(): void {
    this.flushQueued = false;
    for (const b of this.w.take()) this.post({ t: 'cmd', b }, [b]);
  }

  setTaps(names: readonly TapName[]): void {
    let mask = 0;
    for (const n of names) mask |= 1 << TAP_NAMES.indexOf(n);
    this.post({ t: 'taps', mask: mask >>> 0 });
  }

  /** Called after every block, for displays that follow telemetry. */
  onUpdate(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private onBlock(buf: ArrayBuffer): void {
    const f64 = new Float64Array(buf, 0, 1);
    const u32 = new Uint32Array(buf);
    const f32 = new Float32Array(buf);
    const start = f64[HDR.frame];
    const frames = u32[HDR.frames];
    const mask = u32[HDR.tapMask];
    this.cpuPct = f32[HDR.cpu];
    this.dropped = u32[HDR.dropped];
    this.traps = u32[HDR.traps];
    this.tel.set(f32.subarray(TEL_AT, TEL_AT + TEL.LEN));
    let k = 0;
    for (let i = 0; i < TAP_NAMES.length; i++) {
      if (mask & (1 << i)) {
        const at = TAPS_AT + k * BLOCK_FRAMES;
        this.taps.push(i, start, f32.subarray(at, at + frames));
        k++;
      }
    }
    this.frame = start + frames;
    this.blocks++;
    this.node.port.postMessage(buf, [buf]); // back to the pool
    for (const fn of this.listeners) fn();
  }

  /** The audio frame the listener is hearing now (the superhet site's method). */
  heardFrame(): number {
    const ctx = this.ctx;
    let t: number | null = null;
    try {
      const ts = ctx.getOutputTimestamp();
      if (ts.contextTime && ts.performanceTime) t = ts.contextTime + (performance.now() - ts.performanceTime) / 1000;
    } catch {
      // not supported here
    }
    if (t == null) t = ctx.currentTime - (ctx.outputLatency || 0) - (ctx.baseLatency || 0);
    return Math.max(0, Math.floor(t * ctx.sampleRate));
  }
}
