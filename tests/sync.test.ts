// Linked instances: a group settles on one timekeeper, takes play, stop and
// tempo from any member, agrees on one timeline whatever the timing, and
// carries on when the timekeeper leaves or goes silent.

import { describe, expect, it } from 'vitest';
import { EXPIRE_MS, LEAD_MS, LISTEN_MS, SyncGroup, type Channel, type Message } from '../web/src/sync/group';
import { beatAt, change, type Timeline } from '../web/src/sync/timeline';

/** A shared clock and a bus that delivers each message to the other members of the same partition. */
class World {
  t = 1_000_000;
  private readonly subs = new Map<string, { fn: (m: Message) => void; part: string }>();
  private queue: [string, Message][] = [];
  readonly groups = new Map<string, SyncGroup>();
  readonly seen = new Map<string, Timeline[]>();

  channel(id: string, part: string): Channel {
    return {
      post: (m) => {
        const from = this.subs.get(id);
        for (const [to, s] of this.subs) if (to !== id && from && s.part === from.part) this.queue.push([to, structuredClone(m)]);
      },
      listen: (fn) => {
        this.subs.set(id, { fn, part });
        return () => this.subs.delete(id);
      },
      close: () => {},
    };
  }

  join(id: string, part = 'lan', bpm = 120): SyncGroup {
    this.seen.set(id, []);
    const g = new SyncGroup(this.channel(id, part), { id, name: id, now: () => this.t, bpm, onTimeline: (tl) => this.seen.get(id)!.push(tl) });
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
