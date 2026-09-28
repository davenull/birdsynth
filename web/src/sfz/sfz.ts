// SFZ: the text format that maps recordings across the keyboard. This reads
// the part a multisample needs: <control>, <global>, <master>, <group> and
// <region> headers (each level's opcodes inherited by the regions under it),
// #define substitutions, comments, note names (c4 = 60), and these opcodes:
//
//   sample, default_path, key, lokey, hikey, pitch_keycenter, lovel, hivel,
//   loop_mode, loop_start / loopstart, loop_end / loopend, offset, end,
//   tune, transpose, volume, pan, note_offset, octave_offset
//
// Anything else is kept out and listed in `ignored`.

export interface SfzRegion {
  /** The sample's path, with default_path applied and slashes made forward. */
  sample: string;
  lokey: number;
  hikey: number;
  lovel: number;
  hivel: number;
  /** The note that plays the sample at its own pitch, fractional after tune and transpose. */
  root: number;
  /** As the engine's loop modes: 0 off, 1 forward, 4 loop while held (loop_sustain). */
  loopMode: 0 | 1 | 4;
  /** Plays to the end whatever the note does (one_shot). */
  oneShot: boolean;
  loopStart: number | null;
  loopEnd: number | null;
  offset: number;
  end: number | null;
  /** Linear gain from volume (dB). */
  gain: number;
  /** -1..1 */
  pan: number;
}

export interface Sfz {
  regions: SfzRegion[];
  /** Opcodes this reader doesn't use, with how often they appeared. */
  ignored: Record<string, number>;
  warnings: string[];
}

const NOTE: Record<string, number> = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 };

/** A key number or name (c4 = 60, c#4 = 61, db4 = 61). */
export function noteNumber(v: string): number | null {
  const t = v.trim().toLowerCase();
  if (/^-?\d+$/.test(t)) return Number(t);
  const m = /^([a-g])([#b♯♭]?)(-?\d+)$/.exec(t);
  if (!m) return null;
  const acc = m[2] === '#' || m[2] === '♯' ? 1 : m[2] === 'b' || m[2] === '♭' ? -1 : 0;
  return NOTE[m[1]] + acc + 12 * (Number(m[3]) + 1);
}

const USED = new Set([
  'sample',
  'default_path',
  'key',
  'lokey',
  'hikey',
  'pitch_keycenter',
  'lovel',
  'hivel',
  'loop_mode',
  'loopmode',
  'loop_start',
  'loopstart',
  'loop_end',
  'loopend',
  'offset',
  'end',
  'tune',
  'transpose',
  'volume',
  'pan',
  'note_offset',
  'octave_offset',
]);

/** The opcodes of a header's body: `name=value`, where a value runs until the next `name=`. */
function opcodes(body: string): [string, string][] {
  const out: [string, string][] = [];
  const re = /([A-Za-z0-9_]+)=/g;
  const marks: { name: string; at: number; val: number }[] = [];
  let m: RegExpExecArray | null;
  while ((m = re.exec(body))) marks.push({ name: m[1], at: m.index, val: m.index + m[0].length });
  for (let i = 0; i < marks.length; i++) {
    const end = i + 1 < marks.length ? marks[i + 1].at : body.length;
    let v = body.slice(marks[i].val, end);
    // sample paths may contain spaces; other values are one word
    v = marks[i].name === 'sample' || marks[i].name === 'default_path' ? v.trim() : v.trim().split(/\s+/)[0] ?? '';
    out.push([marks[i].name, v]);
  }
  return out;
}

export function parseSfz(text: string): Sfz {
  const warnings: string[] = [];
  const ignored: Record<string, number> = {};
  // comments, then #defines
  let src = text.replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/\/\/.*$/gm, '');
  const defines = new Map<string, string>();
  src = src.replace(/^\s*#define\s+(\$[A-Za-z0-9_]+)\s+(\S+)\s*$/gm, (_, k: string, v: string) => {
    defines.set(k, v);
    return '';
  });
  src = src.replace(/^\s*#include\s+"([^"]*)".*$/gm, (_, f: string) => {
    warnings.push(`#include "${f}" is not supported: its regions are missing`);
    return '';
  });
  for (const [k, v] of [...defines].sort((a, b) => b[0].length - a[0].length)) src = src.split(k).join(v);

  type Ops = Map<string, string>;
  let control: Ops = new Map();
  let global: Ops = new Map();
  let master: Ops = new Map();
  let group: Ops = new Map();
  const regions: SfzRegion[] = [];
  const parts = src.split(/<([A-Za-z_]+)>/);
  // parts: [before, header, body, header, body, ...]
  for (let i = 1; i < parts.length; i += 2) {
    const header = parts[i].toLowerCase();
    const ops: Ops = new Map(opcodes(parts[i + 1] ?? ''));
    switch (header) {
      case 'control':
        control = ops;
        break;
      case 'global':
        global = ops;
        master = new Map();
        group = new Map();
        break;
      case 'master':
        master = ops;
        group = new Map();
        break;
      case 'group':
        group = ops;
        break;
      case 'region': {
        const all: Ops = new Map([...global, ...master, ...group, ...ops]);
        for (const k of all.keys()) if (!USED.has(k)) ignored[k] = (ignored[k] ?? 0) + 1;
        const r = region(all, control, warnings);
        if (r) regions.push(r);
        break;
      }
      default:
        ignored[`<${header}>`] = (ignored[`<${header}>`] ?? 0) + 1;
    }
  }
  return { regions, ignored, warnings };
}

function region(o: Map<string, string>, control: Map<string, string>, warnings: string[]): SfzRegion | null {
  const sample = o.get('sample');
  if (!sample) {
    warnings.push('a region without sample= was skipped');
    return null;
  }
  const num = (k: string, d: number) => {
    const v = o.get(k);
    if (v === undefined) return d;
    const n = Number(v);
    return Number.isFinite(n) ? n : d;
  };
  const key = (k: string, d: number) => {
    const v = o.get(k);
    if (v === undefined) return d;
    const n = noteNumber(v);
    if (n === null) warnings.push(`${k}=${v} isn't a key`);
    return n ?? d;
  };
  const shift = Number(control.get('note_offset') ?? 0) + 12 * Number(control.get('octave_offset') ?? 0);
  const k = o.has('key') ? key('key', 60) : null;
  const lokey = (k ?? key('lokey', 0)) + shift;
  const hikey = (k ?? key('hikey', 127)) + shift;
  const centre = (o.has('pitch_keycenter') ? key('pitch_keycenter', 60) : (k ?? 60)) + shift;
  const mode = (o.get('loop_mode') ?? o.get('loopmode') ?? 'no_loop').toLowerCase();
  const ls = o.get('loop_start') ?? o.get('loopstart');
  const le = o.get('loop_end') ?? o.get('loopend');
  const path = (control.get('default_path') ?? '') + sample;
  return {
    sample: path.replace(/\\/g, '/').replace(/^\.\//, ''),
    lokey: clampKey(lokey),
    hikey: clampKey(hikey),
    lovel: Math.max(0, Math.min(127, num('lovel', 0))),
    hivel: Math.max(0, Math.min(127, num('hivel', 127))),
    root: centre - num('transpose', 0) - num('tune', 0) / 100,
    loopMode: mode === 'loop_continuous' ? 1 : mode === 'loop_sustain' ? 4 : 0,
    oneShot: mode === 'one_shot',
    loopStart: ls === undefined ? null : Number(ls),
    // SFZ's loop end is the last sample of the loop; ours is one past it
    loopEnd: le === undefined ? null : Number(le) + 1,
    offset: num('offset', 0),
    end: o.has('end') ? num('end', 0) + 1 : null,
    gain: 10 ** (num('volume', 0) / 20),
    pan: Math.max(-1, Math.min(1, num('pan', 0) / 100)),
  };
}

function clampKey(k: number): number {
  return Math.max(0, Math.min(127, Math.round(k)));
}

/** Where a region's sample is among the files given (by path, then by file name, ignoring case). */
export function findSample(path: string, files: string[]): string | null {
  const norm = (s: string) => s.replace(/\\/g, '/').toLowerCase();
  const p = norm(path);
  const exact = files.find((f) => norm(f).endsWith(p));
  if (exact) return exact;
  const base = p.split('/').pop()!;
  return files.find((f) => norm(f).split('/').pop() === base) ?? null;
}
