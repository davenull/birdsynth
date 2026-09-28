// The tools Web Worker: runs tools.wasm (mip building, factory tables,
// resampling, previews, the wavetable editor's operations) off the main thread.

import toolsUrl from '../../wasm/tools.wasm?url';
import { handle, type Exports } from './handle';
import type { ToolsReq, ToolsRes } from './protocol';

const ready: Promise<Exports> = WebAssembly.instantiateStreaming(fetch(toolsUrl), {}).then((r) => r.instance.exports as unknown as Exports);

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
