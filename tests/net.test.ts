// Linking over the network, through the link service: the path messages
// take until a direct connection opens, or when one can't (Node has no
// WebRTC, so here that's all of it). Two instances with clocks five seconds
// apart find each other, measure the gap and play one timeline.

import { describe, expect, it } from 'vitest';
import { startRelay } from '../server/relay';
import { LinkChannel } from '../web/src/sync/channel';
import { SyncGroup } from '../web/src/sync/group';
import { NetLink } from '../web/src/sync/net';
import { beatAt } from '../web/src/sync/timeline';

const clock = (skew: number) => () => performance.timeOrigin + performance.now() + skew;

async function until(ok: () => boolean, what: string, ms = 6000): Promise<void> {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 10));
  }
}

describe('network link through the service', () => {
  it('finds, measures and plays in step with an instance whose clock is elsewhere', { timeout: 15_000 }, async () => {
    const relay = await startRelay(0);
    const url = `ws://127.0.0.1:${relay.port}/sync`;
    const members = [
      { id: 'aaaa', skew: 0 },
      { id: 'bbbb', skew: 5000.3 },
    ].map(({ id, skew }) => {
      const now = clock(skew);
      const ch = new LinkChannel(now);
      const net = new NetLink({ id, name: id, url, now, onMessage: (d) => ch.deliver(d, 'net'), RTCPeerConnection: null });
      ch.net = net;
      const g = new SyncGroup(ch, { id, name: id, now, bpm: 120, onTimeline: () => {}, offset: (o) => net.offset(o) ?? 0, ready: () => net.ready });
      return { id, skew, net, g };
    });
    const tick = setInterval(() => {
      for (const m of members) {
        m.net.tick();
        m.g.tick();
      }
    }, 20);
    try {
      const [a, b] = members;
      await until(() => a.net.offset('bbbb') !== null && b.net.offset('aaaa') !== null, 'the clocks');
      expect(Math.abs(a.net.offset('bbbb')! - 5000.3)).toBeLessThan(2);
      expect(Math.abs(b.net.offset('aaaa')! + 5000.3)).toBeLessThan(2);
      expect(a.net.peerInfo()).toMatchObject([{ id: 'bbbb', name: 'bbbb', via: 'relay' }]);
      await until(() => a.g.members().length === 2 && b.g.members().length === 2, 'the members');
      await until(() => a.g.keeping !== b.g.keeping, 'a timekeeper');

      b.g.request({ kind: 'play' });
      await until(() => a.g.timeline.playing && b.g.timeline.playing, 'play');
      expect(a.g.timeline).toEqual(b.g.timeline);
      // at one moment, both put the same beat
      const t = clock(0)();
      const [ba, bb] = members.map((m) => beatAt(m.g.local(m.g.timeline), t + m.skew));
      expect(Math.abs(ba - bb) * 500).toBeLessThan(2);

      // the other leaves: the one left is alone, and keeps time
      b.g.leave();
      b.net.close();
      await until(() => a.g.members().length === 1 && a.g.keeping, 'the other to go');
    } finally {
      clearInterval(tick);
      for (const m of members) m.net.close();
      await relay.close();
    }
  });
});
