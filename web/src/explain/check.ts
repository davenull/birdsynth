// What an automated tour run checks while a step shows: each tour step can
// name things a listener should hear or see, and these measure them on the
// running synth (the output taps and the engine's telemetry).

import { TAP, TEL, type TapName } from '../gen/protocol';
import type { Synth } from '../synth';
import { spectrumDb } from './fft';
import { inharmonicDbc } from './measure';

export interface Expect {
  /** The strongest partial off the note's harmonics, in dB below the strongest harmonic. */
  inharmonicBelow?: number;
  inharmonicAbove?: number;
  /** Output level (RMS dBFS of the last 8192 frames) above this. */
  levelAbove?: number;
  /** The output's pitch is this MIDI note, within 10 cents. */
  note?: number;
  /** Energy above `hz`, in dB relative to all of it, below (a low-pass at work) or above (new harmonics) `db`. */
  highBelow?: [hz: number, db: number];
  highAbove?: [hz: number, db: number];
  /** This telemetry value moves by at least `min` over a second (`index` for arrays). */
  moves?: [key: keyof typeof TEL, min: number, index?: number];
  /** Stereo width: the side signal's level relative to the mid's, in dB, above this. */
  widthAbove?: number;
  /** The arpeggiator steps; a clip plays. */
  arp?: boolean;
  clip?: boolean;
  /** At least this many modulation routings. */
  routings?: number;
  /** Every FLOW node that has something routed through it shows a signal. */
  flowLive?: boolean;
}

export interface Check {
  name: string;
  ok: boolean;
  value: number | string;
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The last `n` frames the listener heard of a tap (recording it for a moment if nothing is). */
async function heard(s: Synth, tap: TapName, n = 8192): Promise<Float32Array | null> {
  const h = s.host;
  if (!h) return null;
  const release = s.useTap(tap);
  try {
    const idx = TAP[tap];
    const out = new Float32Array(n);
    for (let tries = 0; tries < 20; tries++) {
      if (h.taps.read(idx, Math.min(h.heardFrame(), h.taps.latest(idx)), out)) return out;
      await wait(50);
    }
    return null;
  } finally {
    // keep it a little longer: the next check may want it too
    setTimeout(release, 2000);
  }
}

const rmsDb = (x: Float32Array) => {
  let e = 0;
  for (let i = 0; i < x.length; i++) e += x[i] * x[i];
  return 10 * Math.log10(e / x.length + 1e-20);
};

/** Least-squares slope through the rising zero crossings: the period, in frames. */
function pitchHz(x: Float32Array, sr: number): number {
  const z: number[] = [];
  for (let i = 1; i < x.length; i++) if (x[i - 1] < 0 && x[i] >= 0) z.push(i - 1 + -x[i - 1] / (x[i] - x[i - 1]));
  const n = z.length;
  if (n < 3) return 0;
  const mi = (n - 1) / 2;
  const mz = z.reduce((a, b) => a + b, 0) / n;
  let num = 0;
  let den = 0;
  z.forEach((t, i) => {
    num += (i - mi) * (t - mz);
    den += (i - mi) ** 2;
  });
  return sr / (num / den);
}

/** Energy above `hz` relative to the whole spectrum, in dB. */
function highDb(x: Float32Array, sr: number, hz: number): number {
  const db = new Float32Array(x.length / 2 + 1);
  spectrumDb(x, db);
  let hi = 0;
  let all = 0;
  for (let k = 1; k < db.length; k++) {
    const e = 10 ** (db[k] / 10);
    all += e;
    if ((k * sr) / x.length >= hz) hi += e;
  }
  return 10 * Math.log10(hi / all + 1e-20);
}

/** Measure what a step expects. */
export async function checkStep(s: Synth, ex: Expect): Promise<{ ok: boolean; checks: Check[] }> {
  const checks: Check[] = [];
  const add = (name: string, ok: boolean, value: number | string) => checks.push({ name, ok, value: typeof value === 'number' ? Math.round(value * 100) / 100 : value });
  const h = s.host;
  if (!h) return { ok: false, checks: [{ name: 'engine', ok: false, value: 'not running' }] };
  const sr = h.ctx.sampleRate;
  const needsOut =
    ex.levelAbove !== undefined || ex.note !== undefined || ex.highBelow || ex.highAbove || ex.inharmonicAbove !== undefined || ex.inharmonicBelow !== undefined || ex.widthAbove !== undefined;
  const l = needsOut ? await heard(s, 'master.l') : null;
  if (needsOut && !l) add('output', false, 'no signal recorded');
  if (l) {
    if (ex.levelAbove !== undefined) {
      // over a quarter second (most of the history), so a gap between notes doesn't read as silence
      const long = (await heard(s, 'master.l', 12_288)) ?? l;
      add(`level > ${ex.levelAbove} dB`, rmsDb(long) > ex.levelAbove, rmsDb(long));
    }
    if (ex.note !== undefined) {
      const want = 440 * 2 ** ((ex.note - 69) / 12);
      const cents = 1200 * Math.log2(pitchHz(l, sr) / want);
      add(`pitch ${ex.note}`, Math.abs(cents) < 10, Number.isFinite(cents) ? cents : 'none');
    }
    if (ex.highBelow) {
      const v = highDb(l, sr, ex.highBelow[0]);
      add(`energy above ${ex.highBelow[0]} Hz < ${ex.highBelow[1]} dB`, v < ex.highBelow[1], v);
    }
    if (ex.highAbove) {
      const v = highDb(l, sr, ex.highAbove[0]);
      add(`energy above ${ex.highAbove[0]} Hz > ${ex.highAbove[1]} dB`, v > ex.highAbove[1], v);
    }
    if (ex.inharmonicBelow !== undefined || ex.inharmonicAbove !== undefined) {
      const v = inharmonicDbc(l, sr, 440 * 2 ** ((h.tel[TEL.focusPitch] - 69) / 12));
      if (ex.inharmonicBelow !== undefined) add(`inharmonic < ${ex.inharmonicBelow} dBc`, v < ex.inharmonicBelow, v);
      if (ex.inharmonicAbove !== undefined) add(`inharmonic > ${ex.inharmonicAbove} dBc`, v > ex.inharmonicAbove, v);
    }
    if (ex.widthAbove !== undefined) {
      const r = await heard(s, 'master.r');
      if (!r) add('width', false, 'no right channel');
      else {
        const mid = new Float32Array(l.length);
        const side = new Float32Array(l.length);
        for (let i = 0; i < l.length; i++) {
          mid[i] = 0.5 * (l[i] + r[i]);
          side[i] = 0.5 * (l[i] - r[i]);
        }
        const v = rmsDb(side) - rmsDb(mid);
        add(`width > ${ex.widthAbove} dB`, v > ex.widthAbove, v);
      }
    }
  }
  if (ex.moves || ex.arp || ex.clip) {
    // watch the telemetry for a second
    const [key, min, index = 0] = ex.moves ?? ['beat', 0, 0];
    let lo = Infinity;
    let hi = -Infinity;
    const steps = new Set<number>();
    let pos0 = -1;
    let clipMoved = false;
    let clipOn = false;
    for (let i = 0; i < 20; i++) {
      const t = h.tel;
      const v = t[(TEL[key] as number) + index];
      lo = Math.min(lo, v);
      hi = Math.max(hi, v);
      if (t[TEL.arpStep] >= 0) steps.add(t[TEL.arpStep]);
      if (t[TEL.clipPlaying] >= 0) {
        clipOn = true;
        if (pos0 < 0) pos0 = t[TEL.clipPos];
        else if (t[TEL.clipPos] !== pos0) clipMoved = true;
      }
      await wait(50);
    }
    if (ex.moves) add(`${key} moves ≥ ${min}`, hi - lo >= min, hi - lo);
    if (ex.arp) add('arp steps', steps.size >= 2, steps.size);
    if (ex.clip) add('clip plays', clipOn && clipMoved, clipOn ? (clipMoved ? 'playing' : 'stuck') : 'not playing');
  }
  if (ex.routings !== undefined) add(`routings ≥ ${ex.routings}`, s.matrix.used >= ex.routings, s.matrix.used);
  if (ex.flowLive) {
    await wait(300);
    const nodes = [...document.querySelectorAll<HTMLElement>('.flow-page .node:not(.idle)')];
    const dead = nodes.filter((n) => !n.classList.contains('live')).map((n) => n.dataset.node);
    add('FLOW nodes live', nodes.length > 0 && dead.length === 0, dead.length ? `silent: ${dead.join(', ')}` : `${nodes.length} live`);
  }
  return { ok: checks.every((c) => c.ok), checks };
}
