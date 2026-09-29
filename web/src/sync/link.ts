// Linking this synth with other instances: the tabs of this browser, and,
// when switched on, birdsynth on other computers on the network (or anywhere,
// with the same group code). The group's shared timeline drives the
// transport and the tempo, and this synth's own play, stop and tempo knob
// become requests to the group. A timeline change lands in the engine on the
// exact frame that's heard at its moment, so linked instances start and
// change together.

import { normalCode } from '../../../server/protocol';
import { PARAM_ID, PARAMS } from '../gen/params';
import { toPlain } from '../state/param-math';
import type { Synth } from '../synth';
import { LinkChannel, tabPort } from './channel';
import { SyncGroup, type Member } from './group';
import { NetLink, type NetState } from './net';
import { beatAt, type Request, type Timeline } from './timeline';

/** This instance's clock: the machine's, in ms (the same in every tab of a browser). */
export const sessionNow = (): number => performance.timeOrigin + performance.now();

export const TAB_CHANNEL = 'birdsynth-link';

export interface LinkMember extends Member {
  /** How this instance hears it: a tab of this browser, or over the network (directly, or through the link service). */
  via: 'self' | 'tab' | 'direct' | 'relay';
  /** Ping round trip (ms), over the network. */
  rtt: number | null;
  /** How far its clock is ahead of this one's (ms); null until measured. */
  offset: number | null;
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

const newId = (): string => {
  const b = new Uint8Array(8);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(36).padStart(2, '0'))
    .join('')
    .slice(0, 12);
};

/** A starting name that says which machine this is ("Mac Chrome 4f2a"). */
function defaultName(id: string): string {
  const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent;
  const os = /iPhone/.test(ua) ? 'iPhone' : /iPad/.test(ua) ? 'iPad' : /Android/.test(ua) ? 'Android' : /Mac/.test(ua) ? 'Mac' : /Win/.test(ua) ? 'Windows' : /Linux|CrOS/.test(ua) ? 'Linux' : 'birdsynth';
  const browser = /Edg\//.test(ua) ? ' Edge' : /Firefox\//.test(ua) ? ' Firefox' : /Chrome\//.test(ua) ? ' Chrome' : /Safari\//.test(ua) ? ' Safari' : '';
  return `${os}${browser} ${id.slice(0, 4)}`;
}

const BPM = PARAM_ID['global.bpm'];
/** How often a linked instance checks it's still where the timeline says (ms), and how far off it may drift first (ms). */
const CHECK_MS = 1000;
const DRIFT_MS = 0.5;
/** How long to wait for a keeper's clock over the network before going by this one's (ms). */
const CLOCK_WAIT_MS = 3000;
const NUDGE_KEY = 'birdsynth.link-nudge';
const NET_KEY = 'birdsynth.link-net';
const CODE_KEY = 'birdsynth.link-code';
export const MAX_NUDGE_MS = 300;

export class Link {
  /** Linking is on (a setting of this browser); it's `linked` once the engine is up and the group joined. */
  on = read('birdsynth.link') === '1';
  /** Over the network too (a setting of this browser). */
  net = read(NET_KEY) === '1';
  /** A group code: links with instances giving the same one, on any network ('' : this network's). */
  code = normalCode(read(CODE_KEY) ?? '');
  name: string;
  readonly id = newId();
  members: LinkMember[] = [];
  /** Where the last timeline was anchored: its time here, the engine frame, and the keeper's clock offset used. */
  lastAnchor: { at: number; frame: number; beat: number; offset: number } | null = null;
  /** A timeline waiting for its keeper's clock to be measured. */
  deferred: Timeline | null = null;
  /**
   * Plays this tab this much later (ms; negative: earlier), for a path to the
   * speakers the browser doesn't report (Bluetooth, some embedded browsers).
   * Each tab has its own (sessionStorage).
   */
  nudge = 0;
  /** Re-anchors made because the audio clock drifted from the shared one (or its reported latency, or a clock estimate, changed), and the last one's size (ms). */
  corrections = 0;
  lastCorrectionMs = 0;
  /** The last few re-anchors: when (this clock) and how far off it was (ms), and why ('drift', or 'clock' when the keeper's clock estimate moved). */
  readonly correctionLog: { at: number; ms: number; why: 'drift' | 'clock' }[] = [];
  private lastCheck = 0;
  /** How far off the last check found the engine (ms), for a drift to be seen twice before it's acted on. */
  private lastDrift = 0;
  private group: SyncGroup | null = null;
  private channel: LinkChannel | null = null;
  private netLink: NetLink | null = null;
  private offTick: (() => void) | null = null;
  /** Setting the tempo from the timeline, so it isn't taken for the knob moving. */
  private applying = false;
  private tempoTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly subs = new Set<() => void>();

  constructor(private readonly synth: Synth) {
    this.name = readName() || defaultName(this.id);
    try {
      this.nudge = Math.max(-MAX_NUDGE_MS, Math.min(MAX_NUDGE_MS, Number(sessionStorage.getItem(NUDGE_KEY)) || 0));
    } catch {
      this.nudge = 0;
    }
    // closing (or reloading) the page says goodbye, so the others don't wait for it to go quiet;
    // back from the back/forward cache, it joins again
    if (typeof window !== 'undefined') {
      window.addEventListener('pagehide', () => {
        if (this.group) this.leave();
      });
      window.addEventListener('pageshow', (e) => {
        if (e.persisted) this.join();
      });
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

  /** The network side: 'off', or the link service connection's state. */
  get netState(): NetState | 'off' {
    return this.netLink?.state ?? 'off';
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

  setNet(on: boolean): void {
    this.net = on;
    write(NET_KEY, on ? '1' : null);
    if (this.group) {
      if (on) this.startNet();
      else this.stopNet();
    }
    this.refresh();
  }

  setCode(code: string): void {
    this.code = normalCode(code);
    write(CODE_KEY, this.code || null);
    this.netLink?.setCode(this.code);
    this.refresh();
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
    this.name = name.trim().slice(0, 40) || defaultName(this.id);
    writeName(this.name);
    this.group?.rename(this.name);
    this.netLink?.setName(this.name);
    this.emit();
  }

  /** Join the group (once the engine runs, when linking is on). */
  join(): void {
    const host = this.synth.host;
    if (this.group || !this.on || !host) return;
    const channel = new LinkChannel(sessionNow);
    if (typeof BroadcastChannel !== 'undefined') channel.tabs = tabPort(TAB_CHANNEL, (x) => channel.deliver(x, 'tab'));
    this.channel = channel;
    if (this.net) this.startNet();
    this.group = new SyncGroup(channel, {
      id: this.id,
      name: this.name,
      now: sessionNow,
      bpm: this.bpm(),
      onTimeline: (t) => this.follow(t),
      onMembers: () => this.refresh(),
      offset: (id) => this.offsetOf(id) ?? 0,
      ready: () => this.netLink?.ready ?? true,
    });
    // ticks ride the audio blocks: timers stall in background tabs, the worklet doesn't
    this.offTick = host.onUpdate(() => {
      this.netLink?.tick();
      this.group?.tick();
      const d = this.deferred;
      if (d && this.offsetOf(d.keeper) !== null) this.follow(d);
      this.check();
    });
    this.refresh();
  }

  leave(): void {
    this.offTick?.();
    this.offTick = null;
    // the goodbye goes out on both sides before the network side closes
    this.group?.leave();
    this.group = null;
    this.stopNet(100);
    this.channel = null;
    this.deferred = null;
    this.members = [];
    this.emit();
  }

  /** Play, stop or tempo, for the whole group. */
  request(r: Request): void {
    this.group?.request(r);
  }

  private startNet(): void {
    const channel = this.channel;
    if (this.netLink || !channel || typeof location === 'undefined') return;
    this.netLink = new NetLink({
      id: this.id,
      name: this.name,
      code: this.code,
      url: `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/sync`,
      now: sessionNow,
      onMessage: (d) => channel.deliver(d, 'net'),
      onChange: () => this.refresh(),
    });
    channel.net = this.netLink;
  }

  private stopNet(delay = 0): void {
    const net = this.netLink;
    this.netLink = null;
    if (this.channel) this.channel.net = null;
    if (net) {
      if (delay) setTimeout(() => net.close(), delay);
      else net.close();
    }
  }

  /**
   * How far a member's clock is ahead of this one's (ms): none for a tab of
   * this browser (one machine, one clock); measured over the network;
   * null while it's still being measured.
   */
  private offsetOf(id: string): number | null {
    if (id === this.id || this.channel?.inTabs(id)) return 0;
    const net = this.netLink;
    if (!net) return 0;
    const m = net.offset(id);
    if (m !== null) return m;
    return (net.knownFor(id) ?? net.age()) > CLOCK_WAIT_MS ? 0 : null;
  }

  private refresh(): void {
    const g = this.group;
    const info = new Map((this.netLink?.peerInfo() ?? []).map((p) => [p.id, p]));
    this.members = (g?.members() ?? []).map((m) => {
      const p = info.get(m.id);
      const via = m.self ? 'self' : this.channel?.inTabs(m.id) ? 'tab' : (p?.via ?? 'relay');
      return { ...m, via, rtt: p?.rtt ?? null, offset: m.self ? 0 : this.offsetOf(m.id) };
    });
    this.emit();
  }

  /**
   * Once a second: is the engine still where the timeline says? The audio
   * clock runs a little fast or slow against this machine's (a few ms a
   * minute), machines' clocks drift apart, and the browser may report a new
   * output latency (a stream restarted): re-anchor when it's off by more than
   * half a millisecond two checks running (the browser's output time
   * sometimes jumps for a moment and comes back), or at once when the
   * keeper's clock estimate has moved.
   */
  private check(): void {
    const now = sessionNow();
    if (now - this.lastCheck < CHECK_MS) return;
    this.lastCheck = now;
    if (this.netLink) this.refresh();
    const a = this.lastAnchor;
    const t = this.group?.timeline;
    const h = this.synth.host;
    const off = t ? this.offsetOf(t.keeper) : null;
    if (!a || !t || !h || off === null) return;
    const tl = off ? { ...t, at: t.at - off } : t;
    const sr = h.ctx.sampleRate;
    const at = now + 200;
    // where the anchor puts the beat due at `at`, and the frame that will be heard then
    const anchored = a.frame + (beatAt(tl, at) - a.beat) * ((60 * sr) / tl.bpm);
    const heard = h.heardFrame() + ((at - now + this.nudge) * sr) / 1000;
    const ms = ((anchored - heard) / sr) * 1000;
    const why = Math.abs(off - a.offset) > DRIFT_MS / 2 ? 'clock' : 'drift';
    const seen = Math.abs(ms) > DRIFT_MS && (why === 'clock' || (Math.abs(this.lastDrift) > DRIFT_MS && Math.sign(this.lastDrift) === Math.sign(ms)));
    this.lastDrift = ms;
    if (seen) {
      this.corrections++;
      this.lastCorrectionMs = ms;
      this.lastDrift = 0;
      this.correctionLog.push({ at: now, ms, why });
      if (this.correctionLog.length > 20) this.correctionLog.shift();
      this.follow(t);
    }
  }

  private bpm(): number {
    return toPlain(PARAMS[BPM], this.synth.bank.get(BPM));
  }

  /**
   * Put this engine on the timeline: its tempo, and the beat at the frame
   * that's heard at the timeline's moment (or now, if that has passed: the
   * engine anchors a late one where it was meant to be). Over the network, a
   * timeline waits until its keeper's clock has been measured.
   */
  private follow(t: Timeline): void {
    const off = this.offsetOf(t.keeper);
    if (off === null) {
      this.deferred = t;
      this.emit();
      return;
    }
    this.deferred = null;
    const h = this.synth.host;
    if (h) {
      const tl = off ? { ...t, at: t.at - off } : t;
      const now = sessionNow();
      const at = Math.max(tl.at, now);
      const frame = h.heardFrame() + Math.round(((at - now + this.nudge) * h.ctx.sampleRate) / 1000);
      this.applying = true;
      try {
        this.synth.setParamAt(frame, 'global.bpm', tl.bpm);
      } finally {
        this.applying = false;
      }
      const beat = beatAt(tl, at);
      this.lastAnchor = { at, frame, beat, offset: off };
      this.lastDrift = 0;
      h.send((w) => w.setTimeline(frame, beat, 1, tl.playing ? 1 : 0));
    }
    this.emit();
  }
}
