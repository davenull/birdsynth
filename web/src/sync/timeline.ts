// The shared timeline linked instances play to: whether the transport runs,
// the tempo, and which beat falls when. Times are session milliseconds, the
// clock every linked instance agrees on (for tabs of one browser, the
// machine's own clock).

export interface Timeline {
  playing: boolean;
  bpm: number;
  /** A session time (ms) and the beat at that moment; the beat runs on from there at `bpm`. */
  at: number;
  beat: number;
  /** Counts changes, so an instance keeps only the newest; ties go to the lower keeper id. */
  version: number;
  /** The instance that made this version. */
  keeper: string;
}

export type Request = { kind: 'play' } | { kind: 'stop' } | { kind: 'tempo'; bpm: number };

export const MIN_BPM = 20;
export const MAX_BPM = 999;

/** Whether something from another instance is a timeline (anything can arrive over a network). */
export function isTimeline(t: unknown): t is Timeline {
  const x = t as Timeline;
  return (
    !!x &&
    typeof x.playing === 'boolean' &&
    Number.isFinite(x.bpm) &&
    x.bpm >= MIN_BPM &&
    x.bpm <= MAX_BPM &&
    Number.isFinite(x.at) &&
    Math.abs(x.beat) < 1e9 &&
    Number.isSafeInteger(x.version) &&
    x.version >= 0 &&
    typeof x.keeper === 'string'
  );
}

export function isRequest(r: unknown): r is Request {
  const x = r as Request;
  return !!x && (x.kind === 'play' || x.kind === 'stop' || (x.kind === 'tempo' && Number.isFinite(x.bpm)));
}

export function beatAt(t: Timeline, time: number): number {
  return t.beat + ((time - t.at) * t.bpm) / 60_000;
}

/** Whether `a` supersedes `b`. */
export function newer(a: Timeline, b: Timeline): boolean {
  return a.version > b.version || (a.version === b.version && a.keeper < b.keeper);
}

/**
 * A request applied at session time `when` (a little ahead of now, so every
 * instance hears of it in time to act on the same moment). Returns null when
 * it changes nothing (play while playing, stop while stopped, the same tempo).
 */
export function change(t: Timeline, r: Request, when: number, keeper: string): Timeline | null {
  const next = { version: t.version + 1, keeper };
  switch (r.kind) {
    case 'play':
      // from the top: beat 0 lands on `when` everywhere
      return t.playing ? null : { ...t, ...next, playing: true, at: when, beat: 0 };
    case 'stop':
      return t.playing ? { ...t, ...next, playing: false, at: when, beat: beatAt(t, when) } : null;
    case 'tempo': {
      const bpm = Math.min(MAX_BPM, Math.max(MIN_BPM, r.bpm));
      // the beat carries on from where it is at `when`, at the new tempo
      return Math.abs(bpm - t.bpm) < 1e-6 ? null : { ...t, ...next, bpm, at: when, beat: beatAt(t, when) };
    }
  }
}
