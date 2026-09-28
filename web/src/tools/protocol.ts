// Requests to the tools worker, and what each returns.

export type ToolsReq =
  | { op: 'mips'; frames: Float32Array; count: number }
  | { op: 'factoryList' }
  | { op: 'factory'; index: number }
  | { op: 'resample'; cycle: Float32Array }
  | { op: 'preview'; frame: Float32Array; w1: [number, number]; w2: [number, number]; points: number; remap?: Float32Array }
  | { op: 'noise'; index: number }
  | { op: 'filterResponse'; kind: number; cutoff: number; res: number; var: number; sr: number; f0: number; f1: number; points: number }
  | { op: 'irFactory'; index: number; sr: number }
  | { op: 'irPrepare'; l: Float32Array; r: Float32Array };

export interface ToolsResults {
  mips: Float32Array;
  factoryList: { name: string; frames: number }[];
  factory: { frames: Float32Array; count: number };
  resample: Float32Array;
  preview: Float32Array;
  noise: { data: Float32Array; rate: number };
  /** dB at log-spaced frequencies. */
  filterResponse: Float32Array;
  irFactory: { l: Float32Array; r: Float32Array };
  /** Packed for the engine; taps is the response length. */
  irPrepare: { data: Float32Array; taps: number };
}

export type ToolsRes = { id: number; ok: true; result: unknown } | { id: number; ok: false; error: string };
