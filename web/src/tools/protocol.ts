// Requests to the tools worker, and what each returns.

export type ToolsReq =
  | { op: 'mips'; frames: Float32Array; count: number }
  | { op: 'factoryList' }
  | { op: 'factory'; index: number }
  | { op: 'resample'; cycle: Float32Array }
  | { op: 'preview'; frame: Float32Array; w1: [number, number]; w2: [number, number]; points: number };

export interface ToolsResults {
  mips: Float32Array;
  factoryList: { name: string; frames: number }[];
  factory: { frames: Float32Array; count: number };
  resample: Float32Array;
  preview: Float32Array;
}

export type ToolsRes = { id: number; ok: true; result: unknown } | { id: number; ok: false; error: string };
