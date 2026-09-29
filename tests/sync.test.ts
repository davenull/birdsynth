// Linked instances: a group settles on one timekeeper, takes play, stop and
// tempo from any member, agrees on one timeline whatever the timing, and
// carries on when the timekeeper leaves or goes silent; across machines
// whose clocks disagree, it keeps one beat.

import { describe, expect, it } from 'vitest';
import { LinkChannel } from '../web/src/sync/channel';
import { EXPIRE_MS, HEARTBEAT_MS, LEAD_MS, LISTEN_MS, SyncGroup, type Channel, type Message } from '../web/src/sync/group';
import { beatAt, change, type Timeline } from '../web/src/sync/timeline';

/**
 * True time and a bus that delivers each message to the other members of
 * the same partition. Each member may have its own clock (`skew` ms ahead of
 * true time), and knows the others' to within `err` ms.
 */
class World {
  t = 1_000_000;
  private readonly subs = new Map<string, { fn: (m: Message) => void; part: string }>();
  private queue: [string, Message][] = [];
  readonly groups = new Map<string, SyncGroup>();
  readonly seen = new Map<string, Timeline[]>();
  readonly skew = new Map<string, number>();
  /** Drops the messages it returns true for. */
  drop: ((to: string, m: Message) => boolean) | null = null;

  constructor(private readonly err = 0) {}

  channel(id: string, part: string): Channel {
    return {
      post: (m) => {
        const from = this.subs.get(id);
        for (const [to, s] of this.subs) if (to !== id && from && s.part === from.part && !this.drop?.(to, m)) this.queue.push([to, structuredClone(m)]);
      },
      listen: (fn) => {
        this.subs.set(id, { fn, part });
        return () => this.subs.delete(id);
      },
      close: () => {},
    };
  }

  join(id: string, part = 'lan', bpm = 120, skew = 0): SyncGroup {
    this.seen.set(id, []);
    this.skew.set(id, skew);
    // each member's idea of another's clock is off by up to ±err, the same way each time
    const miss = (other: string) => (((id + other).split('').reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % 1000) / 500 - 1) * this.err;
    const g = new SyncGroup(this.channel(id, part), {
      id,
      name: id,
      now: () => this.t + skew,
      bpm,
      onTimeline: (tl) => this.seen.get(id)!.push(tl),
      offset: (other) => (this.skew.get(other) ?? 0) - skew + miss(other),
    });
    this.groups.set(id, g);
    this.deliver();
    return g;
  }

  deliver(): void {
    while (this.queue.length) {
      const [to, m] = this.queue.shift()!;
      this.subs.get(to)?.fn(m);
    }
  }

  /** Let time pass in 20 ms steps (one audio block or so), every live member ticking. */
  run(ms: number, silent: string[] = []): void {
    for (let e = 0; e < ms; e += 20) {
      this.t += 20;
      for (const [id, g] of this.groups) if (!silent.includes(id)) g.tick();
      this.deliver();
    }
  }

  merge(part: string): void {
    for (const s of this.subs.values()) s.part = part;
  }

  timelines(): Timeline[] {
    return [...this.groups.values()].map((g) => g.timeline);
  }

  /** The beat each member puts at true time `t` (by its own clock and its idea of the keeper's). */
  beats(t = this.t): Map<string, number> {
    return new Map([...this.groups.entries()].map(([id, g]) => [id, beatAt(g.local(g.timeline), t + this.skew.get(id)!)]));
  }

  keepers(): string[] {
    return [...this.groups.values()].filter((g) => g.keeping).map((g) => g.id);
  }
}

describe('sync timeline', () => {
  it('starts from the top, stops where it is, changes tempo without a jump', () => {
    const t0: Timeline = { playing: false, bpm: 120, at: 0, beat: 0, version: 0, keeper: 'a' };
    const play = change(t0, { kind: 'play' }, 1000, 'a')!;
    expect(play).toMatchObject({ playing: true, at: 1000, beat: 0, version: 1 });
    expect(beatAt(play, 1500)).toBe(1);
    expect(change(play, { kind: 'play' }, 1200, 'a'), 'play while playing changes nothing').toBeNull();
    const faster = change(play, { kind: 'tempo', bpm: 180 }, 2000, 'a')!;
    expect(beatAt(faster, 2000)).toBeCloseTo(beatAt(play, 2000), 12);
    expect(beatAt(faster, 3000) - beatAt(faster, 2000)).toBeCloseTo(3, 12);
    const stop = change(faster, { kind: 'stop' }, 2500, 'a')!;
    expect(stop).toMatchObject({ playing: false, beat: beatAt(faster, 2500) });
    expect(change(t0, { kind: 'tempo', bpm: 5 }, 0, 'a')!.bpm, 'clamped').toBe(20);
  });
});

describe('sync group', () => {
  it('keeps time by whoever joined first, and anyone can start, change and stop', () => {
    const w = new World();
    w.join('c');
    w.run(50);
    w.join('a');
    w.run(50);
    w.join('b');
    w.run(LISTEN_MS + 100);
    // c joined first (ids don't matter): everyone agrees
    for (const g of w.groups.values()) expect(g.keeperId()).toBe('c');
    expect([...w.groups.values()].filter((g) => g.keeping).map((g) => g.id)).toEqual(['c']);

    // b presses play: c schedules it a little ahead, and everyone gets the same timeline
    const before = w.t;
    w.groups.get('b')!.request({ kind: 'play' });
    w.deliver();
    const [tl] = w.timelines();
    expect(w.timelines().every((x) => JSON.stringify(x) === JSON.stringify(tl))).toBe(true);
    expect(tl).toMatchObject({ playing: true, beat: 0, keeper: 'c' });
    expect(tl.at - before).toBe(LEAD_MS);

    // two members at once: applied in turn, one outcome everywhere
    w.groups.get('a')!.request({ kind: 'tempo', bpm: 100 });
    w.groups.get('b')!.request({ kind: 'tempo', bpm: 140 });
    w.deliver();
    w.run(100);
    const all = w.timelines();
    expect(new Set(all.map((x) => JSON.stringify(x))).size).toBe(1);
    expect(all[0].bpm).toBe(140);
    expect(all[0].playing).toBe(true);

    w.groups.get('a')!.request({ kind: 'stop' });
    w.run(100);
    expect(w.timelines().every((x) => !x.playing)).toBe(true);
  });

  it('brings a late joiner onto the timeline at once, without it taking over', () => {
    const w = new World();
    w.join('a');
    w.run(LISTEN_MS + 50);
    w.groups.get('a')!.request({ kind: 'play' });
    w.run(1000);
    const d = w.join('d');
    w.run(40);
    expect(d.timeline).toEqual(w.groups.get('a')!.timeline);
    expect(d.keeping).toBe(false);
    w.run(LISTEN_MS * 2);
    expect(d.keeping).toBe(false);
    expect(w.seen.get('d')!.at(-1)?.playing).toBe(true);
  });

  it('carries on when the timekeeper leaves, or goes silent', () => {
    const w = new World();
    w.join('a');
    w.run(100);
    w.join('b');
    w.run(100);
    w.join('c');
    w.run(LISTEN_MS);
    w.groups.get('c')!.request({ kind: 'play' });
    w.run(200);
    const playing = w.groups.get('b')!.timeline;

    // a closes its tab: b takes over straight away, and the music doesn't move
    w.groups.get('a')!.leave();
    w.groups.delete('a');
    w.run(40);
    expect(w.groups.get('b')!.keeping).toBe(true);
    const handed = w.groups.get('c')!.timeline;
    expect(handed.keeper).toBe('b');
    expect({ ...handed, version: 0, keeper: '' }).toEqual({ ...playing, version: 0, keeper: '' });

    // b goes silent (a sleeping laptop): c's stop, asked meanwhile, is kept and sent again,
    // and c applies it itself once b has expired
    w.groups.get('c')!.request({ kind: 'stop' });
    w.run(EXPIRE_MS + 200, ['b']);
    const c = w.groups.get('c')!;
    expect(c.keeping).toBe(true);
    expect(c.timeline.playing).toBe(false);
  });

  it('settles two groups that meet on one timekeeper and one timeline', () => {
    const w = new World();
    w.join('x', 'left');
    w.run(200);
    w.join('y', 'right');
    w.run(LISTEN_MS + 50);
    w.groups.get('x')!.request({ kind: 'tempo', bpm: 90 });
    w.groups.get('y')!.request({ kind: 'play' });
    w.run(100);
    expect(w.groups.get('x')!.keeping && w.groups.get('y')!.keeping).toBe(true);
    // the network heals
    w.merge('lan');
    w.run(HEARTBEAT_STEPS * 20);
    const [a, b] = w.timelines();
    expect(a).toEqual(b);
    expect([...w.groups.values()].filter((g) => g.keeping).map((g) => g.id)).toEqual(['x']);
  });
});

const HEARTBEAT_STEPS = 60;

describe('sync group across machines', () => {
  it('keeps one beat when every clock disagrees, through a change of keeper', () => {
    // clocks seconds apart, each known to the others within ±0.2 ms
    const w = new World(0.2);
    w.join('a', 'lan', 120, 0);
    w.run(100);
    w.join('b', 'lan', 120, 7000.3);
    w.run(100);
    w.join('c', 'lan', 120, -2500.7);
    w.run(LISTEN_MS + 20);
    // c keeps time: by its own clock it joined first (it's behind), and everyone goes by the same numbers
    expect(w.keepers()).toEqual(['c']);
    w.groups.get('a')!.request({ kind: 'play' });
    w.run(1000);
    const beatMs = 60_000 / 120;
    const spread = (m: Map<string, number>) => (Math.max(...m.values()) - Math.min(...m.values())) * beatMs;
    expect(spread(w.beats())).toBeLessThan(0.45);
    // a's request reaches c on the next step, and plays a lead later: 1 s less those, everywhere
    expect(w.beats().get('a')! * beatMs).toBeCloseTo(1000 - 20 - LEAD_MS, 0);

    // c closes: the next oldest takes over in its own clock, and the beat carries on where it was
    const before = w.beats(w.t + 40);
    w.groups.get('c')!.leave();
    w.groups.delete('c');
    w.run(40);
    const [next] = w.keepers();
    expect(next).toBe('a');
    expect(w.groups.get('b')!.timeline.keeper).toBe('a');
    expect(spread(w.beats())).toBeLessThan(0.45);
    for (const [id, beat] of w.beats()) expect(Math.abs(beat - before.get(id)!) * beatMs, `no jump for ${id}`).toBeLessThan(0.45);

    // and a tempo change lands on one moment everywhere
    w.groups.get('b')!.request({ kind: 'tempo', bpm: 150 });
    w.run(500);
    expect(w.groups.get('a')!.timeline.bpm).toBe(150);
    expect(spread(w.beats()) * (120 / 150)).toBeLessThan(0.45);
  });

  it('does not hand time to a newcomer just because its clock is behind', () => {
    const w = new World();
    w.join('m');
    w.run(LISTEN_MS + 100);
    w.groups.get('m')!.request({ kind: 'play' });
    w.run(500);
    // a machine whose clock is a minute behind: by its own clock it joined before everyone
    const late = w.join('d', 'lan', 120, -60_000);
    w.run(LISTEN_MS * 3);
    expect(w.groups.get('m')!.keeping).toBe(true);
    expect(late.keeping).toBe(false);
    expect(late.timeline).toEqual(w.groups.get('m')!.timeline);
  });

  it('catches up from the next heartbeat when it misses a change', () => {
    const w = new World();
    w.join('a');
    w.run(100);
    w.join('b');
    w.run(LISTEN_MS);
    // b doesn't get a's next timeline
    w.drop = (to, m) => to === 'b' && m.t === 'timeline';
    w.groups.get('a')!.request({ kind: 'play' });
    w.deliver();
    expect(w.groups.get('b')!.timeline.playing).toBe(false);
    w.drop = null;
    w.run(HEARTBEAT_MS + 40);
    expect(w.groups.get('b')!.timeline).toEqual(w.groups.get('a')!.timeline);
  });
});

describe('sync group, from a stranger', () => {
  it('ignores messages that are not what they claim to be', () => {
    let posted: ((m: Message) => void) | null = null;
    const ch: Channel = { post: () => {}, listen: (fn) => ((posted = fn), () => {}), close: () => {} };
    let t = 0;
    const g = new SyncGroup(ch, { id: 'me', name: 'me', now: () => t, bpm: 120, onTimeline: () => {} });
    t += LISTEN_MS + 20;
    g.tick();
    const before = { ...g.timeline };
    const send = (m: unknown) => posted!(m as Message);
    send({ t: 'timeline', from: 'x', timeline: { ...before, version: 9, bpm: Number.NaN } });
    send({ t: 'timeline', from: 'x', timeline: { ...before, version: 9, at: 'soon' } });
    send({ t: 'timeline', from: 'x' });
    send({ t: 'here', from: 'y', name: 'y', joined: Number.NaN, timeline: before });
    send({ t: 'request', from: 'z', req: { kind: 'tempo', bpm: Number.POSITIVE_INFINITY } });
    send({ t: 'request', from: 'z', req: { kind: 'explode' } });
    send({ from: 42 });
    send(null);
    expect(g.timeline).toEqual(before);
    expect(g.members().map((m) => m.id)).toEqual(['me']);
    send({ t: 'here', from: 'w', name: 'w'.repeat(500), joined: -5, timeline: before });
    expect(g.members().find((m) => m.id === 'w')!.name).toHaveLength(40);
  });
});

describe('link channel', () => {
  it('takes each message once, in order, whichever way it comes', () => {
    const got: Message[] = [];
    const ch = new LinkChannel(() => 0);
    ch.listen((m) => got.push(m));
    const play = { s: 1, m: { t: 'request', from: 'x', req: { kind: 'play' } } };
    const stop = { s: 2, m: { t: 'request', from: 'x', req: { kind: 'stop' } } };
    ch.deliver(play, 'tab');
    ch.deliver(stop, 'tab');
    // the network's copies come late: the play must not be applied again after the stop
    ch.deliver(play, 'net');
    ch.deliver(stop, 'net');
    ch.deliver({ s: 1, m: { t: 'bye', from: 'y' } }, 'net');
    ch.deliver({ nonsense: true }, 'net');
    expect(got.map((m) => (m.t === 'request' ? m.req.kind : m.t))).toEqual(['play', 'stop', 'bye']);
    expect(ch.inTabs('x')).toBe(true);
    expect(ch.inTabs('y')).toBe(false);

    // what this member posts goes out on both, numbered
    const tabs: unknown[] = [];
    const net: unknown[] = [];
    ch.tabs = { post: (x) => tabs.push(x), close: () => {} };
    ch.net = { broadcast: (x) => net.push(x) };
    ch.post({ t: 'bye', from: 'me' });
    ch.post({ t: 'bye', from: 'me' });
    expect(tabs).toEqual(net);
    expect(tabs.map((x) => (x as { s: number }).s)).toEqual([1, 2]);
  });
});
