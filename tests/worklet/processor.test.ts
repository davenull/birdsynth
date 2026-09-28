// Runs the real AudioWorklet processor (bundled from processor.ts) inside
// node:vm with fake worklet globals. Typed-array constructors are replaced
// with counting subclasses, to prove process() never allocates one.

import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';
import { beforeAll, describe, expect, it } from 'vitest';
import { BLOCK_BYTES, BLOCK_FRAMES, HDR, POOL_SIZE, TAPS_AT, TEL_AT } from '../../web/src/audio/block';
import { CmdWriter, DEBUG, TAP, TEL } from '../../web/src/gen/protocol';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const SR = 48_000;

let bundle = '';
beforeAll(() => {
  const out = buildSync({
    entryPoints: [path.join(ROOT, 'web/src/audio/worklet/processor.ts')],
    bundle: true,
    format: 'iife',
    target: 'es2023',
    write: false,
  });
  bundle = out.outputFiles[0].text;
});

type Msg = unknown;

function makeWorklet() {
  const counts = { typed: 0 };
  const counted = <T extends new (...a: never[]) => object>(Base: T): T =>
    class extends (Base as new (...a: unknown[]) => object) {
      constructor(...a: unknown[]) {
        super(...a);
        counts.typed++;
      }
    } as unknown as T;

  const posted: Msg[] = [];
  let frame = 0;
  let ctor: (new (o: unknown) => { port: FakePort; process(i: unknown, o: Float32Array[][]): boolean }) | null = null;

  class FakePort {
    onmessage: ((e: { data: unknown }) => void) | null = null;
    onmessageerror: (() => void) | null = null;
    postMessage(msg: Msg) {
      posted.push(msg);
    }
  }
  class AudioWorkletProcessor {
    port = new FakePort();
  }
  const sandbox: Record<string, unknown> = {
    AudioWorkletProcessor,
    registerProcessor: (_name: string, c: typeof ctor) => (ctor = c),
    sampleRate: SR,
    WebAssembly,
    ArrayBuffer,
    Float32Array: counted(Float32Array),
    Float64Array: counted(Float64Array),
    Uint8Array: counted(Uint8Array),
    Uint32Array: counted(Uint32Array),
    Int32Array: counted(Int32Array),
    DataView: counted(DataView),
    Date,
    console,
  };
  Object.defineProperty(sandbox, 'currentFrame', { get: () => frame });
  vm.createContext(sandbox);
  vm.runInContext(bundle, sandbox, { filename: 'processor.js' });
  if (!ctor) throw new Error('processor did not register');

  const wasm = fs.readFileSync(path.join(ROOT, 'web/wasm/engine.wasm'));
  const proc = new (ctor as NonNullable<typeof ctor>)({ processorOptions: { wasm: wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength) } });
  const send = (data: unknown) => proc.port.onmessage!({ data });
  const L = new Float32Array(128);
  const R = new Float32Array(128);
  const outputs = [[L, R]];
  let inProcess = 0;
  const step = () => {
    const before = counts.typed;
    proc.process([], outputs);
    inProcess += counts.typed - before;
    frame += 128;
  };
  const cmd = (fill: (w: CmdWriter) => void) => {
    const w = new CmdWriter();
    fill(w);
    for (const b of w.take()) send({ t: 'cmd', b });
  };
  return { proc, posted, send, step, cmd, L, R, counts, allocsInProcess: () => inProcess, resetAllocs: () => (inProcess = 0) };
}

describe('worklet processor', () => {
  it('compiles the engine, warms up and reports ready', () => {
    const w = makeWorklet();
    const ready = w.posted.find((m) => (m as { t?: string }).t === 'ready') as { abi: number; sampleRate: number };
    expect(ready).toBeTruthy();
    expect(ready.sampleRate).toBe(SR);
    // the counter sees the processor's own views, so a zero in the next test means something
    expect(w.counts.typed).toBeGreaterThan(5);
  });

  it('renders without creating a typed array in process()', () => {
    const w = makeWorklet();
    w.send({ t: 'pool', bufs: Array.from({ length: POOL_SIZE }, () => new ArrayBuffer(BLOCK_BYTES)) });
    w.send({ t: 'taps', mask: (1 << TAP['master.l']) | (1 << TAP['focus.osc']) });
    w.cmd((c) => {
      for (let i = 0; i < 6; i++) c.noteOn(0, 48 + i * 4, 0, 0.8, i + 1);
    });
    const recycle = () => {
      // return blocks to the pool as the main thread does
      for (let i = w.posted.length - 1; i >= 0; i--) {
        const m = w.posted[i];
        if (m instanceof ArrayBuffer) {
          w.posted.splice(i, 1);
          w.send(m);
        }
      }
    };
    for (let i = 0; i < 400; i++) {
      w.step();
      recycle();
    }
    w.resetAllocs();
    for (let i = 0; i < 4000; i++) {
      w.step();
      if (i % 8 === 7) recycle();
      if (i === 2000) w.cmd((c) => c.noteOff(0, 48, 0, 0, 1));
    }
    expect(w.allocsInProcess()).toBe(0);
    expect(w.L.some((v) => v !== 0)).toBe(true);
  });

  it('sends one block per 1024 frames with telemetry and the chosen taps', () => {
    const w = makeWorklet();
    w.send({ t: 'pool', bufs: Array.from({ length: POOL_SIZE }, () => new ArrayBuffer(BLOCK_BYTES)) });
    w.send({ t: 'taps', mask: (1 << TAP['master.l']) | (1 << TAP['focus.out']) });
    w.cmd((c) => c.noteOn(0, 69, 0, 1, 1));
    for (let i = 0; i < 64; i++) w.step();
    const blocks = w.posted.filter((m): m is ArrayBuffer => m instanceof ArrayBuffer);
    expect(blocks.length).toBe(POOL_SIZE); // 64 quanta = 8 blocks, and the pool had 8
    let expectStart = 0;
    for (const b of blocks) {
      const f64 = new Float64Array(b, 0, 1);
      const u32 = new Uint32Array(b);
      const f32 = new Float32Array(b);
      expect(f64[0]).toBe(expectStart);
      expect(u32[HDR.frames]).toBe(BLOCK_FRAMES);
      expect(u32[HDR.tapMask]).toBe((1 << TAP['master.l']) | (1 << TAP['focus.out']));
      expect(f32[TEL_AT + TEL.voicesActive]).toBe(1);
      expectStart += BLOCK_FRAMES;
    }
    // the second tap in the mask (focus.out) equals the master left channel divided by the master gain
    const last = new Float32Array(blocks.at(-1)!);
    const master = last.subarray(TAPS_AT, TAPS_AT + BLOCK_FRAMES);
    const voice = last.subarray(TAPS_AT + BLOCK_FRAMES, TAPS_AT + 2 * BLOCK_FRAMES);
    const ratio = master[500] / voice[500];
    expect(Math.abs(ratio - 10 ** (-6 / 20))).toBeLessThan(1e-3);
    // with the pool empty, further quanta are counted as dropped, not allocated
    w.step();
    expect(w.posted.filter((m) => m instanceof ArrayBuffer).length).toBe(POOL_SIZE);
  });

  it('restarts after a trap and plays again as soon as the host resends', () => {
    const w = makeWorklet();
    w.send({ t: 'pool', bufs: Array.from({ length: POOL_SIZE }, () => new ArrayBuffer(BLOCK_BYTES)) });
    w.cmd((c) => c.noteOn(0, 60, 0, 1, 1));
    for (let i = 0; i < 10; i++) w.step();
    w.cmd((c) => c.debug(0, DEBUG.Trap, 0));
    w.step();
    const trap = w.posted.find((m) => (m as { t?: string }).t === 'trap') as { msg: string; traps: number };
    expect(trap?.msg).toMatch(/debug trap requested by the host/);
    expect(trap.traps).toBe(1);
    // the host's resync: the held note again
    w.cmd((c) => c.noteOn(0, 60, 0, 1, 1));
    let quanta = 0;
    do {
      w.step();
      quanta++;
    } while (!w.L.some((v) => v !== 0) && quanta < 100);
    expect(quanta).toBeLessThanOrEqual(20);
  });
});
