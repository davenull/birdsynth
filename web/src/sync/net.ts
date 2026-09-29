// Linking over the network. An instance with it on connects to the link
// service (server/relay.ts, at /sync on the site), which introduces the
// instances on the same network, or with the same group code, to each other.
// Each pair then connects directly with WebRTC, the service carrying the
// setup. There are no outside servers, so only local candidates: enough on
// one network. Each pair gets two data channels: a reliable one for the
// group's messages, and one that neither retries nor waits, for pings.
// Until a direct connection opens, or if it never does, messages go through
// the service instead.
//
// Every instance pings every other one (four times a second directly; through
// the service, once a second after the first few) to learn how far its clock
// is from theirs (clock.ts).

import type { FromRelay, ToRelay } from '../../../server/protocol';
import { ClockEstimate } from './clock';

type Envelope = { k: 'm'; d: unknown } | { k: 'ping'; t0: number } | { k: 'pong'; t0: number; t1: number };

export type Via = 'direct' | 'relay';
export type NetState = 'connecting' | 'online' | 'offline';

export interface PeerInfo {
  id: string;
  name: string;
  via: Via;
  /** The quickest recent ping round trip (ms). */
  rtt: number | null;
  /** How far the peer's clock is ahead of this one's (ms). */
  offset: number | null;
}

export interface NetOptions {
  id: string;
  name: string;
  /** A group code ('' or none: the instances on this network). */
  code?: string;
  /** The service's WebSocket URL. */
  url: string;
  now(): number;
  onMessage(data: unknown, from: string): void;
  onChange?(): void;
  /** For tests: the WebSocket class, and RTCPeerConnection (null: everything through the service). */
  WebSocket?: typeof WebSocket;
  RTCPeerConnection?: typeof RTCPeerConnection | null;
}

const PING_DIRECT_MS = 250;
const PING_RELAY_MS = 1000;
const KEEPALIVE_MS = 25_000;
const RETRY_MIN_MS = 1000;
const RETRY_MAX_MS = 15_000;
/** How long a direct connection gets to open before the offering side tries again (doubling each time, up to a minute). */
const DIRECT_WAIT_MS = 8000;
/** After this long, the network's members are taken as known even if some haven't been heard from (or the service can't be reached). */
const INTRO_MS = 2000;
const GIVE_UP_MS = 4000;

interface Peer {
  id: string;
  name: string;
  /** In the service's latest list. */
  listed: boolean;
  /** Heard from (anything) since it was listed. */
  heard: boolean;
  since: number;
  pc: RTCPeerConnection | null;
  msg: RTCDataChannel | null;
  ping: RTCDataChannel | null;
  made: number;
  wait: number;
  queued: RTCIceCandidateInit[];
  clock: ClockEstimate;
  lastPing: number;
}

const open = (ch: RTCDataChannel | null): ch is RTCDataChannel => ch?.readyState === 'open';

export class NetLink {
  state: NetState = 'connecting';
  /** The group code the service has this instance under ('' for the network's own group). */
  code = '';
  private ws: WebSocket | null = null;
  private closed = false;
  private retryAt = 0;
  private backoff = RETRY_MIN_MS;
  private lastKeepalive = 0;
  private readonly started: number;
  private listedAt: number | null = null;
  private wantCode: string;
  private name: string;
  private readonly peers = new Map<string, Peer>();

  constructor(private readonly o: NetOptions) {
    this.wantCode = o.code ?? '';
    this.name = o.name;
    this.started = o.now();
    this.connect();
  }

  private get rtc(): typeof RTCPeerConnection | null {
    if (this.o.RTCPeerConnection !== undefined) return this.o.RTCPeerConnection;
    return typeof RTCPeerConnection === 'undefined' ? null : RTCPeerConnection;
  }

  /**
   * Whether this instance knows who's on the network: the service has listed
   * them and each has been heard from, or they've had time to be (or the
   * service can't be reached).
   */
  get ready(): boolean {
    const now = this.o.now();
    if (now - this.started > GIVE_UP_MS) return true;
    if (this.listedAt === null) return false;
    return now - this.listedAt > INTRO_MS || [...this.peers.values()].every((p) => !p.listed || p.heard);
  }

  /** Send to every peer: directly where connected, else through the service (or, `both`, each way: for a goodbye while the page closes). */
  broadcast(d: unknown, both = false): void {
    const env: Envelope = { k: 'm', d };
    const text = JSON.stringify(env);
    const rest: string[] = [];
    for (const p of this.peers.values()) {
      const direct = open(p.msg);
      if (direct) p.msg!.send(text);
      if ((!direct || both) && p.listed) rest.push(p.id);
    }
    if (rest.length) this.send({ t: 'relay', to: rest, data: env });
  }

  /** How far a peer's clock is ahead of this one's (ms), once enough pings have come back. */
  offset(id: string): number | null {
    const c = this.peers.get(id)?.clock;
    return c?.settled ? c.offset : null;
  }

  /** How long a peer has been known (ms; null if it isn't). */
  knownFor(id: string): number | null {
    const p = this.peers.get(id);
    return p ? this.o.now() - p.since : null;
  }

  /** How long this has been running (ms). */
  age(): number {
    return this.o.now() - this.started;
  }

  peerInfo(): PeerInfo[] {
    return [...this.peers.values()].map((p) => ({ id: p.id, name: p.name, via: open(p.msg) ? 'direct' : 'relay', rtt: p.clock.rtt, offset: p.clock.offset }));
  }

  setName(name: string): void {
    this.name = name;
    this.hello();
  }

  /** Join another group (or, with '', the network's own): the peers so far are left. */
  setCode(code: string): void {
    this.wantCode = code;
    for (const p of [...this.peers.values()]) this.drop(p);
    this.listedAt = null;
    this.hello();
    this.o.onChange?.();
  }

  /** Call often (each audio block): pings, reconnecting, retrying direct connections. */
  tick(): void {
    if (this.closed) return;
    const now = this.o.now();
    if (!this.ws && now >= this.retryAt) this.connect();
    const online = this.state === 'online';
    if (online && now - this.lastKeepalive > KEEPALIVE_MS) {
      this.lastKeepalive = now;
      this.send({ t: 'ping', n: 0 });
    }
    for (const p of this.peers.values()) {
      const direct = open(p.ping);
      if (!direct && !(online && p.listed)) continue;
      // quick pings until the clock estimate settles, then (through the service) slower
      if (now - p.lastPing >= (direct || !p.clock.settled ? PING_DIRECT_MS : PING_RELAY_MS)) {
        p.lastPing = now;
        this.toPeer(p, { k: 'ping', t0: now }, direct, true);
      }
      // the offering side tries again when a direct connection hasn't opened in time
      if (this.o.id < p.id && p.listed && online && this.rtc && !open(p.msg) && now - p.made > p.wait) {
        p.wait = Math.min(60_000, p.wait * 2);
        this.offer(p);
      }
    }
  }

  close(): void {
    this.closed = true;
    const ws = this.ws;
    this.ws = null;
    ws?.close();
    for (const p of [...this.peers.values()]) this.drop(p);
  }

  private connect(): void {
    const WS = this.o.WebSocket ?? WebSocket;
    let ws: WebSocket;
    try {
      ws = new WS(this.o.url);
    } catch {
      this.state = 'offline';
      this.retry();
      return;
    }
    this.ws = ws;
    this.state = 'connecting';
    ws.onopen = () => {
      if (this.ws !== ws) return;
      this.backoff = RETRY_MIN_MS;
      this.state = 'online';
      this.lastKeepalive = this.o.now();
      this.hello();
      this.o.onChange?.();
    };
    ws.onmessage = (e: MessageEvent) => {
      if (this.ws !== ws) return;
      let m: FromRelay;
      try {
        m = JSON.parse(String(e.data));
      } catch {
        return;
      }
      this.fromRelay(m);
    };
    ws.onclose = () => {
      if (this.ws !== ws) return;
      this.ws = null;
      this.state = 'offline';
      this.retry();
      // peers connected directly carry on; the rest are out of reach until the service is back
      for (const p of this.peers.values()) p.listed = open(p.msg) && p.listed;
      this.o.onChange?.();
    };
  }

  private retry(): void {
    this.retryAt = this.o.now() + this.backoff;
    this.backoff = Math.min(RETRY_MAX_MS, this.backoff * 2);
  }

  private hello(): void {
    this.send({ t: 'hello', id: this.o.id, name: this.name, code: this.wantCode });
  }

  private send(m: ToRelay): void {
    if (this.ws?.readyState === 1) this.ws.send(JSON.stringify(m));
  }

  private fromRelay(m: FromRelay): void {
    switch (m.t) {
      case 'peers': {
        this.code = m.code;
        this.listedAt ??= this.o.now();
        const ids = new Set<string>();
        for (const x of m.peers) {
          ids.add(x.id);
          this.peer(x.id, x.name);
        }
        for (const p of [...this.peers.values()])
          if (!ids.has(p.id)) {
            p.listed = false;
            if (!open(p.msg)) this.drop(p);
          }
        this.o.onChange?.();
        break;
      }
      case 'signal':
        void this.signalled(m.from, m.data);
        break;
      case 'relay': {
        const p = this.peers.get(m.from);
        if (p) this.receive(p, m.data as Envelope, false);
        break;
      }
      case 'error':
        console.warn(`link service: ${m.message}`);
        break;
    }
  }

  private peer(id: string, name: string): Peer {
    let p = this.peers.get(id);
    if (!p) {
      p = {
        id,
        name,
        listed: true,
        heard: false,
        since: this.o.now(),
        pc: null,
        msg: null,
        ping: null,
        made: this.o.now(),
        wait: DIRECT_WAIT_MS,
        queued: [],
        clock: new ClockEstimate(),
        lastPing: -Infinity,
      };
      this.peers.set(id, p);
      // one side offers, so the two never offer at once: the lower id
      if (this.o.id < id) this.offer(p);
    }
    p.listed = true;
    if (name) p.name = name;
    return p;
  }

  private drop(p: Peer): void {
    this.closePc(p);
    this.peers.delete(p.id);
  }

  private closePc(p: Peer): void {
    const pc = p.pc;
    p.pc = p.msg = p.ping = null;
    pc?.close();
  }

  private makePc(p: Peer): RTCPeerConnection | null {
    const RTC = this.rtc;
    if (!RTC) return null;
    this.closePc(p);
    const pc = new RTC({ iceServers: [] });
    const msg = pc.createDataChannel('msg', { negotiated: true, id: 0 });
    const ping = pc.createDataChannel('ping', { negotiated: true, id: 1, ordered: false, maxRetransmits: 0 });
    Object.assign(p, { pc, msg, ping, made: this.o.now(), queued: [] });
    const mine = () => p.pc === pc;
    pc.onicecandidate = (e) => {
      if (e.candidate && mine()) this.signal(p.id, { c: e.candidate.toJSON() });
    };
    pc.onconnectionstatechange = () => {
      if (!mine() || (pc.connectionState !== 'failed' && pc.connectionState !== 'closed')) return;
      this.closePc(p);
      if (!p.listed) this.drop(p);
      this.o.onChange?.();
    };
    msg.onopen = () => {
      if (!mine()) return;
      p.wait = DIRECT_WAIT_MS;
      // the direct path's round trips are quicker and more even: start its estimate afresh
      p.clock.restart();
      this.o.onChange?.();
    };
    msg.onclose = () => {
      if (!mine()) return;
      this.closePc(p);
      if (!p.listed) this.drop(p);
      this.o.onChange?.();
    };
    msg.onmessage = (e) => this.onData(p, e.data);
    ping.onmessage = (e) => this.onData(p, e.data);
    return pc;
  }

  private offer(p: Peer): void {
    const pc = this.makePc(p);
    if (!pc) return;
    pc.setLocalDescription().then(
      () => {
        if (p.pc === pc) this.signal(p.id, { sdp: pc.localDescription?.toJSON() });
      },
      (e: unknown) => console.warn('link: could not start a direct connection', e),
    );
  }

  private async signalled(from: string, data: unknown): Promise<void> {
    const p = this.peers.get(from);
    if (!p || !data || typeof data !== 'object') return;
    const d = data as { sdp?: RTCSessionDescriptionInit; c?: RTCIceCandidateInit };
    try {
      if (d.sdp?.type === 'offer') {
        // the other side offers only when its id is the lower
        if (this.o.id < from) return;
        const pc = this.makePc(p);
        if (!pc) return;
        await pc.setRemoteDescription(d.sdp);
        await this.flush(p, pc);
        await pc.setLocalDescription();
        if (p.pc === pc) this.signal(from, { sdp: pc.localDescription?.toJSON() });
      } else if (d.sdp?.type === 'answer') {
        const pc = p.pc;
        if (!pc || pc.signalingState !== 'have-local-offer') return;
        await pc.setRemoteDescription(d.sdp);
        await this.flush(p, pc);
      } else if (d.c) {
        const pc = p.pc;
        if (pc?.remoteDescription) await pc.addIceCandidate(d.c).catch(() => {});
        else p.queued.push(d.c);
      }
    } catch (e) {
      console.warn('link: direct connection setup failed', e);
    }
  }

  private async flush(p: Peer, pc: RTCPeerConnection): Promise<void> {
    const q = p.queued;
    p.queued = [];
    for (const c of q) await pc.addIceCandidate(c).catch(() => {});
  }

  private signal(to: string, data: unknown): void {
    this.send({ t: 'signal', to, data });
  }

  private onData(p: Peer, data: unknown): void {
    if (typeof data !== 'string') return;
    let env: Envelope;
    try {
      env = JSON.parse(data);
    } catch {
      return;
    }
    this.receive(p, env, true);
  }

  private receive(p: Peer, env: Envelope, direct: boolean): void {
    p.heard = true;
    switch (env?.k) {
      case 'm':
        this.o.onMessage(env.d, p.id);
        break;
      case 'ping':
        // answered the way it came, so the round trip is one path's
        this.toPeer(p, { k: 'pong', t0: env.t0, t1: this.o.now() }, direct, true);
        break;
      case 'pong':
        p.clock.add(env.t0, env.t1, this.o.now());
        break;
    }
  }

  private toPeer(p: Peer, env: Envelope, direct: boolean, quick = false): void {
    const ch = quick ? p.ping : p.msg;
    if (direct && open(ch)) ch.send(JSON.stringify(env));
    else this.send({ t: 'relay', to: p.id, data: env });
  }
}
