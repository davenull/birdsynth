// A group of linked instances. One of them, the timekeeper, owns the shared
// timeline: every member's play, stop and tempo go to it as requests, it
// applies them one at a time and sends out each new version, scheduled a
// little ahead so every member switches on the same moment. The timekeeper
// is the member that joined first; when it goes quiet or leaves, the next
// oldest takes over and carries the timeline on as it stands, so nothing is
// heard to change. Conflicts settle the same way everywhere: the highest
// version wins, ties to the lower keeper id.
//
// The group runs over any channel that delivers each message to every other
// member (BroadcastChannel between tabs; a network link later), and needs
// only a shared clock, `now()`.

import { change, newer, type Request, type Timeline } from './timeline';

export interface Channel {
  post(m: Message): void;
  listen(fn: (m: Message) => void): () => void;
  close(): void;
}

export type Message =
  | { t: 'here'; from: string; name: string; joined: number }
  | { t: 'bye'; from: string }
  | { t: 'timeline'; from: string; timeline: Timeline }
  | { t: 'request'; from: string; req: Request };

export interface Member {
  id: string;
  name: string;
  joined: number;
  self: boolean;
  keeper: boolean;
}

export interface GroupOptions {
  id: string;
  name: string;
  /** The shared clock, ms. */
  now(): number;
  /** How far ahead (ms) a change is scheduled, so every member hears of it in time. */
  lead?: number;
  /** The tempo to start from, alone. */
  bpm: number;
  onTimeline(t: Timeline): void;
  onMembers?(members: Member[]): void;
}

export const HEARTBEAT_MS = 500;
/** A member not heard from for this long has gone (a closed tab, a sleeping laptop). */
export const EXPIRE_MS = 3000;
/** A newcomer listens this long for an older member before it may keep time itself. */
export const LISTEN_MS = 600;
/** A request not answered by a new timeline in this long is sent again (to whoever keeps time by then). */
export const RETRY_MS = 400;
export const LEAD_MS = 150;

interface Peer {
  name: string;
  joined: number;
  seen: number;
}

export class SyncGroup {
  timeline: Timeline;
  private readonly peers = new Map<string, Peer>();
  private readonly joined: number;
  private lastHere = -Infinity;
  private keptTime = false;
  private pending: { req: Request; version: number; sent: number } | null = null;
  private readonly off: () => void;

  constructor(
    private readonly ch: Channel,
    private readonly o: GroupOptions,
  ) {
    const now = o.now();
    this.joined = now;
    this.timeline = { playing: false, bpm: o.bpm, at: now, beat: 0, version: 0, keeper: o.id };
    this.off = ch.listen((m) => this.receive(m));
    this.tick();
  }

  get id(): string {
    return this.o.id;
  }

  /** The member keeping time: the first to join (ties to the lower id), of those heard from lately. */
  keeperId(): string {
    let id = this.o.id;
    let joined = this.joined;
    for (const [pid, p] of this.peers)
      if (p.joined < joined || (p.joined === joined && pid < id)) {
        id = pid;
        joined = p.joined;
      }
    return id;
  }

  /** Whether this member keeps time now (not while it's still listening for an older one). */
  get keeping(): boolean {
    return this.keeperId() === this.o.id && this.o.now() - this.joined >= LISTEN_MS;
  }

  members(): Member[] {
    const keeper = this.keeperId();
    const out: Member[] = [{ id: this.o.id, name: this.o.name, joined: this.joined, self: true, keeper: keeper === this.o.id }];
    for (const [id, p] of this.peers) out.push({ id, name: p.name, joined: p.joined, self: false, keeper: keeper === id });
    return out.sort((a, b) => a.joined - b.joined || (a.id < b.id ? -1 : 1));
  }

  /** Play, stop or a tempo: applied here if this member keeps time, else sent to the one that does. */
  request(req: Request): void {
    if (this.keeping) return this.decide(req);
    this.pending = { req, version: this.timeline.version, sent: this.o.now() };
    this.ch.post({ t: 'request', from: this.o.id, req });
  }

  /** Call often (each audio block): heartbeats, forgetting the silent, taking over, re-sending a request. */
  tick(): void {
    const now = this.o.now();
    if (now - this.lastHere >= HEARTBEAT_MS) this.here(now);
    let changed = false;
    for (const [id, p] of this.peers)
      if (now - p.seen > EXPIRE_MS) {
        this.peers.delete(id);
        changed = true;
      }
    const keeping = this.keeping;
    if (keeping && !this.keptTime) {
      // this member keeps time now: carry the timeline on as it stands, under its own name
      this.publish({ ...this.timeline, version: this.timeline.version + 1, keeper: this.o.id });
      changed = true;
    }
    this.keptTime = keeping;
    if (this.pending) {
      if (keeping) {
        const { req } = this.pending;
        this.pending = null;
        this.decide(req);
      } else if (now - this.pending.sent > RETRY_MS) {
        this.pending.sent = now;
        this.ch.post({ t: 'request', from: this.o.id, req: this.pending.req });
      }
    }
    if (changed) this.o.onMembers?.(this.members());
  }

  leave(): void {
    this.ch.post({ t: 'bye', from: this.o.id });
    this.off();
    this.ch.close();
  }

  private here(now: number): void {
    this.lastHere = now;
    this.ch.post({ t: 'here', from: this.o.id, name: this.o.name, joined: this.joined });
  }

  private decide(req: Request): void {
    const t = change(this.timeline, req, this.o.now() + (this.o.lead ?? LEAD_MS), this.o.id);
    if (t) this.publish(t);
  }

  private publish(t: Timeline): void {
    this.timeline = t;
    this.ch.post({ t: 'timeline', from: this.o.id, timeline: t });
    this.o.onTimeline(t);
  }

  private receive(m: Message): void {
    if (m.from === this.o.id) return;
    const now = this.o.now();
    const peer = this.peers.get(m.from);
    if (peer) peer.seen = now;
    switch (m.t) {
      case 'here':
        if (!peer) {
          this.peers.set(m.from, { name: m.name, joined: m.joined, seen: now });
          // someone new: answer at once, with the timeline if this member keeps it
          this.here(now);
          if (this.keeping) this.ch.post({ t: 'timeline', from: this.o.id, timeline: this.timeline });
          this.o.onMembers?.(this.members());
        }
        break;
      case 'bye':
        if (this.peers.delete(m.from)) {
          this.o.onMembers?.(this.members());
          this.tick();
        }
        break;
      case 'timeline':
        if (newer(m.timeline, this.timeline)) {
          this.timeline = m.timeline;
          if (this.pending && m.timeline.version > this.pending.version) this.pending = null;
          this.o.onTimeline(m.timeline);
        }
        break;
      case 'request':
        if (this.keeping) this.decide(m.req);
        break;
    }
  }
}
