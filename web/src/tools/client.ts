// Promise wrapper around the tools worker.

import type { ToolsReq, ToolsRes, ToolsResults } from './protocol';

type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void };

export class Tools {
  private readonly worker: Worker;
  private next = 1;
  private readonly pending = new Map<number, Pending>();

  constructor() {
    this.worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
    this.worker.onmessage = (e: MessageEvent<ToolsRes>) => {
      const p = this.pending.get(e.data.id);
      if (!p) return;
      this.pending.delete(e.data.id);
      if (e.data.ok) p.resolve(e.data.result);
      else p.reject(new Error(e.data.error));
    };
    this.worker.onerror = (e) => {
      for (const p of this.pending.values()) p.reject(new Error(`tools worker failed: ${e.message}`));
      this.pending.clear();
    };
  }

  call<K extends ToolsReq['op']>(req: Extract<ToolsReq, { op: K }>, transfer: Transferable[] = []): Promise<ToolsResults[K]> {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.worker.postMessage({ ...req, id }, transfer);
    });
  }
}

let shared: Tools | null = null;

/** The one tools worker, started on first use. */
export function tools(): Tools {
  return (shared ??= new Tools());
}
