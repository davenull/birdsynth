// The link service: who sees whom (one network, or one group code), what it
// passes on and to whom, and the limits that keep it small.

import { afterEach, describe, expect, it } from 'vitest';
import WebSocket from 'ws';
import type { FromRelay } from '../server/protocol';
import { networkKey, startRelay, type RelayOptions } from '../server/relay';

type Of<T extends FromRelay['t']> = Extract<FromRelay, { t: T }>;

class Client {
  readonly ws: WebSocket;
  inbox: FromRelay[] = [];
  private wake: (() => void)[] = [];

  constructor(url: string, ip?: string, origin?: string, extra: Record<string, string> = {}) {
    const headers: Record<string, string> = { ...extra };
    if (ip) headers['x-forwarded-for'] = ip;
    if (origin) headers.origin = origin;
    this.ws = new WebSocket(url, { headers });
    this.ws.on('message', (d) => {
      this.inbox.push(JSON.parse(String(d)));
      this.wake.splice(0).forEach((f) => f());
    });
  }

  opened(): Promise<void> {
    return new Promise((ok, fail) => {
      this.ws.once('open', () => ok());
      this.ws.once('unexpected-response', (_req, res) => fail(new Error(`HTTP ${res.statusCode}`)));
      this.ws.once('error', fail);
    });
  }

  send(m: unknown): void {
    this.ws.send(JSON.stringify(m));
  }

  /** The next message of a kind (and matching), waiting up to `ms` for it. */
  async next<T extends FromRelay['t']>(t: T, match: (m: Of<T>) => boolean = () => true, ms = 2000): Promise<Of<T>> {
    const until = Date.now() + ms;
    for (;;) {
      const i = this.inbox.findIndex((m) => m.t === t && match(m as Of<T>));
      if (i >= 0) return this.inbox.splice(i, 1)[0] as Of<T>;
      if (Date.now() > until) throw new Error(`no ${t} message`);
      await new Promise<void>((ok) => {
        this.wake.push(ok);
        setTimeout(ok, 50);
      });
    }
  }

  /** The newest peer list, once it names exactly these. */
  peers(ids: string[]): Promise<Of<'peers'>> {
    const want = [...ids].sort().join();
    return this.next('peers', (m) => m.peers.map((p) => p.id).sort().join() === want);
  }

  async quiet(ms = 150): Promise<FromRelay[]> {
    await new Promise((ok) => setTimeout(ok, ms));
    return this.inbox.splice(0);
  }
}

let relays: { close(): Promise<void> }[] = [];
let clients: Client[] = [];

async function relay(o: RelayOptions = {}): Promise<{ url: string; port: number }> {
  const r = await startRelay(0, '127.0.0.1', o);
  relays.push(r);
  return { url: `ws://127.0.0.1:${r.port}/sync`, port: r.port };
}

async function client(url: string, id: string, ip: string, code?: string): Promise<Client> {
  const c = new Client(url, ip);
  clients.push(c);
  await c.opened();
  c.send({ t: 'hello', id, name: `n-${id}`, code });
  return c;
}

afterEach(async () => {
  for (const c of clients) c.ws.terminate();
  clients = [];
  await Promise.all(relays.map((r) => r.close()));
  relays = [];
});

describe('link service', () => {
  it('takes an IPv4 address as a network, IPv6 by its /64, and private addresses as one local network', () => {
    expect(networkKey('203.0.113.7')).toBe('203.0.113.7');
    expect(networkKey('::ffff:203.0.113.7')).toBe('203.0.113.7');
    for (const local of ['192.168.2.40', '10.1.2.3', '172.20.0.1', '127.0.0.1', '::1', 'fe80::1%en0', 'fd12:3456::9', '::ffff:192.168.1.9']) expect(networkKey(local), local).toBe('local');
    expect(networkKey('172.32.0.1')).toBe('172.32.0.1');
    expect(networkKey('2001:db8:aa:bb:1:2:3:4')).toBe(networkKey('2001:0db8:00aa:00bb::99'));
    expect(networkKey('2001:db8:aa:bb::1')).not.toBe(networkKey('2001:db8:aa:bc::1'));
  });

  it('introduces the instances on one network to each other, and no one else', async () => {
    const { url } = await relay();
    const a = await client(url, 'aaaa1', '203.0.113.7');
    await a.peers([]);
    const b = await client(url, 'bbbb2', '203.0.113.7');
    const c = await client(url, 'cccc3', '198.51.100.2');
    const la = await a.peers(['bbbb2']);
    expect(la).toMatchObject({ you: 'aaaa1', code: '', peers: [{ id: 'bbbb2', name: 'n-bbbb2' }] });
    await b.peers(['aaaa1']);
    await c.peers([]);

    // a visitor can't pick another network by claiming Cloudflare's header: it isn't read
    const spoof = new Client(url, undefined, undefined, { 'cf-connecting-ip': '203.0.113.7' });
    clients.push(spoof);
    await spoof.opened();
    spoof.send({ t: 'hello', id: 'eeee5', name: 'n-eeee5' });
    await spoof.peers([]);
    expect((await a.quiet()).filter((m) => m.t === 'peers' && m.peers.some((p) => p.id === 'eeee5'))).toEqual([]);

    // a closes: b hears it's gone
    a.ws.close();
    await b.peers([]);
  });

  it('passes setup and messages only within a group', async () => {
    const { url } = await relay();
    const a = await client(url, 'aaaa1', '203.0.113.7');
    const b = await client(url, 'bbbb2', '203.0.113.7');
    const c = await client(url, 'cccc3', '198.51.100.2');
    await a.peers(['bbbb2']);
    await b.peers(['aaaa1']);
    await c.peers([]);

    a.send({ t: 'signal', to: 'bbbb2', data: { sdp: 'x' } });
    expect(await b.next('signal')).toEqual({ t: 'signal', from: 'aaaa1', data: { sdp: 'x' } });
    a.send({ t: 'relay', to: ['bbbb2', 'cccc3'], data: 42 });
    expect(await b.next('relay')).toEqual({ t: 'relay', from: 'aaaa1', data: 42 });
    c.send({ t: 'signal', to: 'aaaa1', data: 'hi' });
    c.send({ t: 'relay', to: 'aaaa1', data: 'hi' });
    expect((await c.quiet()).filter((m) => m.t !== 'peers')).toEqual([]);
    expect((await a.quiet()).filter((m) => m.t === 'signal' || m.t === 'relay')).toEqual([]);
  });

  it('links any networks that give the same group code', async () => {
    const { url } = await relay();
    const a = await client(url, 'aaaa1', '203.0.113.7');
    const b = await client(url, 'bbbb2', '203.0.113.7');
    await a.peers(['bbbb2']);
    const c = await client(url, 'cccc3', '198.51.100.2', 'jam-1');
    await c.peers([]);
    // a switches to the same code, typed differently: it leaves b for c
    a.send({ t: 'hello', id: 'aaaa1', name: 'n-aaaa1', code: 'Jam 1' });
    expect(await a.peers(['cccc3'])).toMatchObject({ code: 'JAM1' });
    await c.peers(['aaaa1']);
    await b.peers([]);
    a.send({ t: 'relay', to: 'bbbb2', data: 'old friend' });
    expect((await b.quiet()).filter((m) => m.t === 'relay')).toEqual([]);
  });

  it('keeps one connection per instance, a full group closed, and floods cut down', async () => {
    const { url } = await relay({ maxPerGroup: 3, rate: 20 });
    const a = await client(url, 'aaaa1', '203.0.113.7');
    const b = await client(url, 'bbbb2', '203.0.113.7');
    await b.peers(['aaaa1']);

    // a again, on a new connection (a reload that kept its id): the old one goes, b still sees one a
    const a2 = await client(url, 'aaaa1', '203.0.113.7');
    await a2.peers(['bbbb2']);
    await new Promise((ok) => setTimeout(ok, 100));
    expect(a.ws.readyState).toBe(WebSocket.CLOSED);
    expect((await b.quiet(50)).filter((m) => m.t === 'peers').at(-1)).toMatchObject({ peers: [{ id: 'aaaa1' }] });

    const c = await client(url, 'cccc3', '203.0.113.7');
    await c.peers(['aaaa1', 'bbbb2']);
    const d = await client(url, 'dddd4', '203.0.113.7');
    expect(await d.next('error')).toEqual({ t: 'error', message: 'the group is full' });

    // 200 messages at once: about a burst's worth (twice the rate) get through
    for (let i = 0; i < 200; i++) a2.send({ t: 'relay', to: 'bbbb2', data: i });
    const got = (await b.quiet(300)).filter((m) => m.t === 'relay').length;
    expect(got).toBeGreaterThanOrEqual(35);
    expect(got).toBeLessThanOrEqual(45);
  });

  it('refuses pages of other sites, and answers its health check', async () => {
    const { url, port } = await relay();
    const evil = new Client(url, '203.0.113.7', 'https://evil.example');
    clients.push(evil);
    await expect(evil.opened()).rejects.toThrow('HTTP 403');
    const own = new Client(url, '203.0.113.7', `http://127.0.0.1:${port}`);
    clients.push(own);
    await own.opened();
    const res = await fetch(`http://127.0.0.1:${port}/sync/health`);
    expect(res.status).toBe(200);
    expect(await res.text()).toMatch(/^ok 1 0\n$/);
  });
});
