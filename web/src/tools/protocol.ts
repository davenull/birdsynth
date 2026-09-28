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
  | { op: 'irPrepare'; l: Float32Array; r: Float32Array }
  // the wavetable editor (frames are count × 2048)
  | { op: 'wtProcess'; kind: number; a: number; b: number; frames: Float32Array; count: number }
  | { op: 'wtPwm'; frame: Float32Array; count: number }
  | { op: 'wtSort'; frames: Float32Array; count: number }
  | { op: 'wtMorph'; keys: Float32Array; count: number; target: number; mode: number }
  | { op: 'wtAnalyze'; frame: Float32Array }
  | { op: 'wtSynthesize'; mag: Float32Array; phase: Float32Array }
  | { op: 'formula'; src: string; frames: Float32Array; count: number; apply: Uint8Array; selected: Uint8Array; seed: number }
  | { op: 'formulaCheck'; src: string }
  | { op: 'pitch'; audio: Float32Array; sr: number }
  | { op: 'import'; mode: number; audio: Float32Array; sr: number; arg: number; max: number };

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
  wtProcess: Float32Array;
  wtPwm: Float32Array;
  /** Frame indices, darkest first. */
  wtSort: Uint32Array;
  wtMorph: Float32Array;
  /** 1025 each: index 0 is DC, then harmonics 1..1024. */
  wtAnalyze: { mag: Float32Array; phase: Float32Array };
  wtSynthesize: Float32Array;
  formula: FormulaResult & { frames: Float32Array };
  formulaCheck: FormulaResult;
  /** Hz, or 0 when the recording has no clear pitch. */
  pitch: number;
  import: { frames: Float32Array; count: number };
}

/** A formula's outcome: ran (error null), or where and why it didn't. */
export interface FormulaResult {
  error: string | null;
  pos: number;
}

export type ToolsRes = { id: number; ok: true; result: unknown } | { id: number; ok: false; error: string };
