// One channel for a linked group over two transports: the tabs of this
// browser (a BroadcastChannel) and the network (net.ts). Every message goes
// out on both, numbered by its sender. A member takes only the first copy of
// each message, and nothing older than what it has already taken from that
// sender, so a message that arrives both ways, or is overtaken by a later
// one, counts once and in order. (What's dropped as late is sent again:
// heartbeats carry the timeline and requests are retried.)

import type { Channel, Message } from './group';

export type Transport = 'tab' | 'net';

interface Numbered {
  s: number;
  m: Message;
}

const numbered = (x: unknown): x is Numbered =>
  !!x && typeof x === 'object' && typeof (x as Numbered).s === 'number' && typeof (x as Numbered).m?.from === 'string';

export class LinkChannel implements Channel {
  private seq = 0;
  private readonly last = new Map<string, number>();
  private readonly tabSeen = new Map<string, number>();
  private fn: ((m: Message) => void) | null = null;
  /** The tabs of this browser (a BroadcastChannel, where there is one). */
  tabs: { post(x: unknown): void; close(): void } | null = null;
  /** The network side, while linking over the network. */
  net: { broadcast(x: unknown, both?: boolean): void } | null = null;

  constructor(private readonly now: () => number) {}

  post(m: Message): void {
    const x: Numbered = { s: ++this.seq, m };
    this.tabs?.post(x);
    this.net?.broadcast(x, m.t === 'bye');
  }

  listen(fn: (m: Message) => void): () => void {
    this.fn = fn;
    return () => {
      if (this.fn === fn) this.fn = null;
    };
  }

  close(): void {
    this.tabs?.close();
    this.fn = null;
  }

  /** A message from one of the transports. */
  deliver(x: unknown, via: Transport): void {
    if (!numbered(x)) return;
    const from = x.m.from;
    if (via === 'tab') this.tabSeen.set(from, this.now());
    if (x.s <= (this.last.get(from) ?? 0)) return;
    this.last.set(from, x.s);
    if (x.m.t === 'bye') this.forget(from);
    this.fn?.(x.m);
  }

  /** Whether a member has been heard lately from a tab of this browser (so it shares this machine's clock). */
  inTabs(id: string, within = 3000): boolean {
    const t = this.tabSeen.get(id);
    return t !== undefined && this.now() - t < within;
  }

  private forget(id: string): void {
    this.tabSeen.delete(id);
  }
}

/** The tabs of this browser showing birdsynth, over one BroadcastChannel. */
export function tabPort(name: string, onMessage: (x: unknown) => void): { post(x: unknown): void; close(): void } {
  const bc = new BroadcastChannel(name);
  bc.onmessage = (e: MessageEvent) => onMessage(e.data);
  return { post: (x) => bc.postMessage(x), close: () => bc.close() };
}
