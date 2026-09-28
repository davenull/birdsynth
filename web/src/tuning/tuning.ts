// Microtuning files turned into the engine's tuning table: the pitch of
// every MIDI note as a fractional note number (12-TET at A440 is n → n).
//
//  .scl  Scala scale: a description, a count, then that many pitches in
//        cents (with a ".") or ratios ("3/2", "2"); the last is the period.
//  .kbm  Scala keyboard mapping: which key plays which scale degree, and
//        which key sounds at what frequency.
//  .tun  AnaMark tuning: "note N=cents" above 8.1758 Hz (MIDI note 0), in
//        [Tuning] or, more precisely, [Exact Tuning].
//
// Without a .kbm, the scale starts at middle C (note 60 = 261.6256 Hz), so
// a 12-TET scale gives back standard tuning.

export interface Scale {
  description: string;
  /** Degrees 1..n in cents; the last is the period (usually 1200). */
  cents: number[];
}

export interface KeyMap {
  /** Keys in the repeating pattern; 0 maps keys straight to degrees. */
  size: number;
  first: number;
  last: number;
  /** The key that plays degree 0. */
  zero: number;
  /** The key that sounds at `freq`. */
  ref: number;
  freq: number;
  /** The degree that the pattern advances by each time it repeats. */
  period: number;
  /** Per key of the pattern: a degree, or null for a silent key (played untuned here). */
  map: (number | null)[];
}

export class TuningError extends Error {}

/** The lines that carry data (comments start with "!"). */
function lines(text: string): { text: string; line: number }[] {
  return text
    .split(/\r?\n/)
    .map((t, i) => ({ text: t.trim(), line: i + 1 }))
    .filter((l) => !l.text.startsWith('!'));
}

function pitchCents(tok: string, line: number): number {
  if (tok.includes('.')) {
    const c = Number(tok);
    if (!Number.isFinite(c)) throw new TuningError(`line ${line}: "${tok}" isn't a number of cents`);
    return c;
  }
  const m = /^(\d+)(?:\/(\d+))?$/.exec(tok);
  if (!m) throw new TuningError(`line ${line}: "${tok}" isn't a ratio or a number of cents`);
  const num = Number(m[1]);
  const den = m[2] === undefined ? 1 : Number(m[2]);
  if (num <= 0 || den <= 0) throw new TuningError(`line ${line}: the ratio ${tok} must be positive`);
  return 1200 * Math.log2(num / den);
}

export function parseScl(text: string): Scale {
  const ls = lines(text);
  if (ls.length < 2) throw new TuningError('a .scl file needs a description line and a note count');
  const description = ls[0].text;
  const count = Number(ls[1].text.split(/\s+/)[0]);
  if (!Number.isInteger(count) || count < 1) throw new TuningError(`line ${ls[1].line}: the note count "${ls[1].text}" isn't a positive whole number`);
  const cents: number[] = [];
  for (const l of ls.slice(2)) {
    if (!l.text) continue;
    if (cents.length === count) break;
    cents.push(pitchCents(l.text.split(/\s+/)[0], l.line));
  }
  if (cents.length < count) throw new TuningError(`the scale says it has ${count} notes but lists ${cents.length}`);
  return { description, cents };
}

export function parseKbm(text: string): KeyMap {
  const ls = lines(text).filter((l) => l.text);
  if (ls.length < 7) throw new TuningError('a .kbm file needs at least 7 lines (size, first, last, zero key, reference key, frequency, period degree)');
  const num = (i: number, what: string, int = true) => {
    const v = Number(ls[i].text.split(/\s+/)[0]);
    if (!Number.isFinite(v) || (int && !Number.isInteger(v))) throw new TuningError(`line ${ls[i].line}: "${ls[i].text}" isn't a valid ${what}`);
    return v;
  };
  const size = num(0, 'map size');
  const km: KeyMap = { size, first: num(1, 'first key'), last: num(2, 'last key'), zero: num(3, 'zero key'), ref: num(4, 'reference key'), freq: num(5, 'frequency', false), period: num(6, 'period degree'), map: [] };
  if (km.freq <= 0) throw new TuningError(`line ${ls[5].line}: the reference frequency must be above 0 Hz`);
  for (let i = 0; i < size; i++) {
    const l = ls[7 + i];
    if (!l) {
      km.map.push(null); // missing entries are unmapped keys
      continue;
    }
    const tok = l.text.split(/\s+/)[0].toLowerCase();
    if (tok === 'x') km.map.push(null);
    else {
      const d = Number(tok);
      if (!Number.isInteger(d)) throw new TuningError(`line ${l.line}: "${l.text}" isn't a scale degree or "x"`);
      km.map.push(d);
    }
  }
  return km;
}

/** The default mapping: every key the next degree, degree 0 on middle C at 261.63 Hz. */
export function defaultKeyMap(scale: Scale): KeyMap {
  return { size: 0, first: 0, last: 127, zero: 60, ref: 60, freq: 440 * 2 ** (-9 / 12), period: scale.cents.length, map: [] };
}

/** A degree's pitch in cents above degree 0 (degrees past the period wrap into the next one). */
function degreeCents(scale: Scale, d: number): number {
  const n = scale.cents.length;
  const period = scale.cents[n - 1];
  const oct = Math.floor(d / n);
  const i = d - oct * n;
  return oct * period + (i === 0 ? 0 : scale.cents[i - 1]);
}

/** A key's pitch in cents above degree 0, or null for an unmapped key. */
function keyCents(scale: Scale, km: KeyMap, key: number): number | null {
  const offset = key - km.zero;
  if (km.size === 0) return degreeCents(scale, offset);
  const reps = Math.floor(offset / km.size);
  const d = km.map[offset - reps * km.size];
  if (d === null || d === undefined) return null;
  return reps * degreeCents(scale, km.period) + degreeCents(scale, d);
}

const toPitch = (hz: number) => 69 + 12 * Math.log2(hz / 440);

/** The engine's table for a scale and mapping: 128 fractional note numbers. */
export function tableFromScale(scale: Scale, km: KeyMap = defaultKeyMap(scale)): Float32Array {
  const out = new Float32Array(128);
  let refCents = keyCents(scale, km, km.ref);
  if (refCents === null) refCents = keyCents(scale, { ...km, size: 0 }, km.ref) ?? 0;
  for (let k = 0; k < 128; k++) {
    const c = k < km.first || k > km.last ? null : keyCents(scale, km, k);
    out[k] = c === null ? k : toPitch(km.freq * 2 ** ((c - refCents) / 1200));
  }
  return out;
}

/** Note 0 of a .tun file, unless it says otherwise. */
const TUN_BASE = 440 * 2 ** (-69 / 12);

export function parseTun(text: string): { name: string; table: Float32Array } {
  const out = Float32Array.from({ length: 128 }, (_, i) => i);
  let section = '';
  let base = TUN_BASE;
  const plain = new Map<number, number>();
  const exact = new Map<number, number>();
  let name = '';
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/;.*$/, '').trim();
    if (!line) continue;
    const sec = /^\[(.+)\]$/.exec(line);
    if (sec) {
      section = sec[1].trim().toLowerCase();
      continue;
    }
    const kv = /^([^=]+)=(.*)$/.exec(line);
    if (!kv) continue;
    const key = kv[1].trim().toLowerCase();
    const val = kv[2].trim().replace(/^"|"$/g, '');
    const note = /^note\s+(\d+)$/.exec(key);
    if (section === 'tuning' && note) plain.set(Number(note[1]), Number(val));
    else if (section === 'exact tuning' && note) exact.set(Number(note[1]), Number(val));
    else if (section === 'exact tuning' && key === 'basefreq') base = Number(val);
    else if ((section === 'info' || section === 'scale begin') && key === 'name') name = val;
  }
  const cents = exact.size ? exact : plain;
  if (!cents.size) throw new TuningError('no [Tuning] or [Exact Tuning] section with "note N=cents" lines');
  if (!(base > 0)) throw new TuningError('BaseFreq must be above 0 Hz');
  for (const [n, c] of cents) {
    if (n < 0 || n > 127) continue;
    if (!Number.isFinite(c)) throw new TuningError(`note ${n}: "${c}" isn't a number of cents`);
    out[n] = toPitch(base * 2 ** (c / 1200));
  }
  return { name, table: out };
}

/** 12-TET: every note its own number. */
export function standardTable(): Float32Array {
  return Float32Array.from({ length: 128 }, (_, i) => i);
}
