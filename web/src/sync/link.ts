// Linking this synth with other instances (tabs of this browser, for now):
// the group's shared timeline drives the transport and the tempo, and this
// synth's own play, stop and tempo knob become requests to the group. A
// timeline change lands in the engine on the exact frame that's heard at its
// moment, so linked tabs start and change together.

import { PARAM_ID, PARAMS } from '../gen/params';
import { toPlain } from '../state/param-math';
import type { Synth } from '../synth';
import { SyncGroup, type Channel, type Member, type Message } from './group';
import { beatAt, type Request, type Timeline } from './timeline';

/** The clock linked tabs share: the machine's, in ms. */
export const sessionNow = (): number => performance.timeOrigin + performance.now();

/** Every tab of this browser showing birdsynth, over one BroadcastChannel. */
export function tabChannel(name = 'birdsynth-link'): Channel {
  const bc = new BroadcastChannel(name);
  return {
    post: (m: Message) => bc.postMessage(m),
    listen: (fn) => {
      const h = (e: MessageEvent<Message>) => fn(e.data);
      bc.addEventListener('message', h);
      return () => bc.removeEventListener('message', h);
    },
    close: () => bc.close(),
  };
}

const read = (k: string): string | null => {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage.getItem(k);
  } catch {
    return null;
  }
};
const write = (k: string, v: string | null): void => {
  try {
    if (v === null) localStorage.removeItem(k);
    else localStorage.setItem(k, v);
  } catch {
    // private mode: for this session only
  }
};

/** A tab's name in the link is its own (sessionStorage: each tab has one, and it lasts through reloads). */
const readName = (): string | null => {
  try {
    return typeof sessionStorage === 'undefined' ? null : sessionStorage.getItem('birdsynth.link-name');
  } catch {
    return null;
  }
};
const writeName = (v: string): void => {
  try {
    sessionStorage.setItem('birdsynth.link-name', v);
  } catch {
    // private mode: for now only
  }
};

const BPM = PARAM_ID['global.bpm'];
/** How often a linked tab checks it's still where the timeline says (ms), and how far off it may drift first (ms). */
const CHECK_MS = 1000;
const DRIFT_MS = 0.5;
const NUDGE_KEY = 'birdsynth.link-nudge';
export const MAX_NUDGE_MS = 300;

export class Link {
  /** Linking is on (a setting of this browser); it's `linked` once the engine is up and the group joined. */
  on = read('birdsynth.link') === '1';
  name: string;
  readonly id = Math.random().toString(36).slice(2, 10);
  members: Member[] = [];
  /** Where the last timeline was anchored: its session time and the engine frame (for checking the alignment). */
  lastAnchor: { at: number; frame: number; beat: number } | null = null;
  /**
   * Plays this tab this much later (ms; negative: earlier), for a path to the
   * speakers the browser doesn't report (Bluetooth, some embedded browsers).
   * Each tab has its own (sessionStorage).
   */
  nudge = 0;
  /** Re-anchors made because the audio clock drifted from the shared one (or its reported latency changed), and the last one's size (ms). */
  corrections = 0;
  lastCorrectionMs = 0;
  private lastCheck = 0;
  private group: SyncGroup | null = null;
  private offTick: (() => void) | null = null;
  /** Setting the tempo from the timeline, so it isn't taken for the knob moving. */
  private applying = false;
  private tempoTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly subs = new Set<() => void>();

  constructor(private readonly synth: Synth) {
    this.name = readName() || `birdsynth ${this.id.slice(0, 4)}`;
    try {
      this.nudge = Math.max(-MAX_NUDGE_MS, Math.min(MAX_NUDGE_MS, Number(sessionStorage.getItem(NUDGE_KEY)) || 0));
    } catch {
      this.nudge = 0;
    }
    // the tempo knob, while linked, asks the group (once per 80 ms while it moves)
    synth.bank.subscribe(BPM, () => {
      if (!this.group || this.applying || this.tempoTimer) return;
      this.tempoTimer = setTimeout(() => {
        this.tempoTimer = null;
        this.request({ kind: 'tempo', bpm: this.bpm() });
      }, 80);
    });
  }

  get linked(): boolean {
    return this.group !== null;
  }

  get timeline(): Timeline | null {
    return this.group?.timeline ?? null;
  }

  get keeping(): boolean {
    return this.group?.keeping ?? false;
  }

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  private emit(): void {
    for (const fn of this.subs) fn();
  }

  setOn(on: boolean): void {
    this.on = on;
    write('birdsynth.link', on ? '1' : null);
    if (on) this.join();
    else this.leave();
    this.emit();
  }

  setNudge(ms: number): void {
    this.nudge = Math.max(-MAX_NUDGE_MS, Math.min(MAX_NUDGE_MS, Math.round(ms)));
    try {
      if (this.nudge) sessionStorage.setItem(NUDGE_KEY, String(this.nudge));
      else sessionStorage.removeItem(NUDGE_KEY);
    } catch {
      // private mode: for now only
    }
    const t = this.group?.timeline;
    if (t) this.follow(t);
    this.emit();
  }

  setName(name: string): void {
    this.name = name.trim() || `birdsynth ${this.id.slice(0, 4)}`;
    writeName(this.name);
    // members learn the new name when this one rejoins
    if (this.group) {
      this.leave();
      this.join();
    }
    this.emit();
  }

  /** Join the group (once the engine runs, when linking is on). */
  join(): void {
    const host = this.synth.host;
    if (this.group || !this.on || !host || typeof BroadcastChannel === 'undefined') return;
    this.group = new SyncGroup(tabChannel(), {
      id: this.id,
      name: this.name,
      now: sessionNow,
      bpm: this.bpm(),
      onTimeline: (t) => this.follow(t),
      onMembers: (m) => {
        this.members = m;
        this.emit();
      },
    });
    this.members = this.group.members();
    // ticks ride the audio blocks: timers stall in background tabs, the worklet doesn't
    this.offTick = host.onUpdate(() => {
      this.group?.tick();
      this.check();
    });
    this.emit();
  }

  leave(): void {
    this.offTick?.();
    this.offTick = null;
    this.group?.leave();
    this.group = null;
    this.members = [];
    this.emit();
  }

  /** Play, stop or tempo, for the whole group. */
  request(r: Request): void {
    this.group?.request(r);
  }

  /**
   * Once a second: is the engine still where the timeline says? The audio
   * clock runs a little fast or slow against the shared one (a few ms a
   * minute here; more between two machines), and the browser may report a
   * new output latency (a stream restarted): re-anchor when it's off by more
   * than half a millisecond.
   */
  private check(): void {
    const now = sessionNow();
    if (now - this.lastCheck < CHECK_MS) return;
    this.lastCheck = now;
    const a = this.lastAnchor;
    const t = this.group?.timeline;
    const h = this.synth.host;
    if (!a || !t || !h) return;
    const sr = h.ctx.sampleRate;
    const at = now + 200;
    // where the anchor puts the beat due at `at`, and the frame that will be heard then
    const anchored = a.frame + (beatAt(t, at) - a.beat) * ((60 * sr) / t.bpm);
    const heard = h.heardFrame() + ((at - now + this.nudge) * sr) / 1000;
    if (Math.abs(anchored - heard) > (DRIFT_MS * sr) / 1000) {
      this.corrections++;
      this.lastCorrectionMs = ((anchored - heard) / sr) * 1000;
      this.follow(t);
    }
  }

  private bpm(): number {
    return toPlain(PARAMS[BPM], this.synth.bank.get(BPM));
  }

  /**
   * Put this engine on the timeline: its tempo and the beat at the frame
   * that's heard at the timeline's moment (or now, if that has passed:
   * the engine anchors a late one where it was meant to be).
   */
  private follow(t: Timeline): void {
    const h = this.synth.host;
    if (h) {
      const now = sessionNow();
      const at = Math.max(t.at, now);
      const frame = h.heardFrame() + Math.round(((at - now + this.nudge) * h.ctx.sampleRate) / 1000);
      this.applying = true;
      try {
        this.synth.setParamAt(frame, 'global.bpm', t.bpm);
      } finally {
        this.applying = false;
      }
      const beat = beatAt(t, at);
      this.lastAnchor = { at, frame, beat };
      h.send((w) => w.setTimeline(frame, beat, 1, t.playing ? 1 : 0));
    }
    this.emit();
  }
}
