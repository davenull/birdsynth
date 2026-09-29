// The link service: introduces birdsynth instances to each other so they can
// link over the local network. Instances coming from the same network (the
// same public address as Cloudflare reports it; for IPv6, the same /64) see
// each other, and so do instances that give the same group code, wherever
// they are. The service passes their connection setup (WebRTC offers,
// answers and candidates) between them, and carries their messages itself
// when a direct connection can't be made. It stores nothing.
//
// In production it runs beside nginx in the site's container
// (`server/main.ts`); in development the Vite dev and preview servers host
// it on their own port (vite.config.ts).

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { Duplex } from 'node:stream';
import { WebSocketServer, type WebSocket } from 'ws';
import { normalCode, RELAY_PATH, type FromRelay, type ToRelay } from './protocol.ts';

export { normalCode, RELAY_PATH };

export interface RelayOptions {
  /** Instances one group may hold. */
  maxPerGroup?: number;
  /** Messages a client may send per second (bursts of twice that are let through). */
  rate?: number;
  /** How often idle connections are checked (ms). */
  keepalive?: number;
}

export interface Relay {
  /** Takes a WebSocket upgrade to /sync; false (the socket untouched) for anything else. */
  upgrade(req: IncomingMessage, socket: Duplex, head: Buffer): boolean;
  /** Answers /sync/health; false for anything else. */
  http(req: IncomingMessage, res: ServerResponse): boolean;
  stats(): { clients: number; groups: number };
  close(): void;
}

interface Client {
  ws: WebSocket;
  id: string;
  name: string;
  group: string;
  tokens: number;
  last: number;
  alive: boolean;
}

const MAX_BYTES = 32 * 1024;
const MAX_CLIENTS = 2000;
const ID = /^[a-z0-9]{4,32}$/;

function expand6(a: string): string[] | null {
  const [head, tail, extra] = a.split('::');
  if (extra !== undefined) return null;
  const h = head ? head.split(':') : [];
  const t = tail !== undefined ? (tail ? tail.split(':') : []) : [];
  const fill = tail !== undefined ? 8 - h.length - t.length : 0;
  if (fill < 0) return null;
  const all = [...h, ...Array<string>(fill).fill('0'), ...t];
  return all.length === 8 ? all.map((x) => (parseInt(x, 16) || 0).toString(16)) : null;
}

/**
 * Which network an address belongs to: an IPv4 address is one network (a
 * home router shares it); an IPv6 network is its /64. Private and loopback
 * addresses (the dev server, a direct connection on the LAN) are all one
 * 'local' network.
 */
export function networkKey(ip: string): string {
  const a = ip.trim().toLowerCase().replace(/^::ffff:(?=\d+\.)/, '');
  const v4 = /^(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(a);
  if (v4) {
    const [p, q] = [Number(v4[1]), Number(v4[2])];
    const local = p === 10 || p === 127 || (p === 172 && q >= 16 && q < 32) || (p === 192 && q === 168) || (p === 169 && q === 254) || (p === 100 && q >= 64 && q < 128);
    return local ? 'local' : a;
  }
  const g = expand6(a.split('%')[0]);
  if (!g) return a || 'local';
  const first = parseInt(g[0], 16);
  if (g.slice(0, 7).every((x) => x === '0') || (first & 0xfe00) === 0xfc00 || (first & 0xffc0) === 0xfe80) return 'local';
  return g.slice(0, 4).join(':') + '::/64';
}

/** The address a connection comes from: Cloudflare's client address, else the proxy's, else the socket's. */
export function clientAddress(req: IncomingMessage): string {
  const cf = req.headers['cf-connecting-ip'];
  if (typeof cf === 'string' && cf) return cf;
  const xff = req.headers['x-forwarded-for'];
  if (typeof xff === 'string' && xff) return xff.split(',')[0];
  return req.socket.remoteAddress ?? '';
}

/** Only pages of the site itself (or tools that send no Origin) may connect. */
function sameSite(req: IncomingMessage): boolean {
  const origin = req.headers.origin;
  if (!origin) return true;
  try {
    return new URL(origin).host === req.headers.host;
  } catch {
    return false;
  }
}

export function createRelay(o: RelayOptions = {}): Relay {
  const maxPerGroup = o.maxPerGroup ?? 16;
  const rate = o.rate ?? 100;
  const clients = new Map<WebSocket, Client>();
  const byId = new Map<string, Client>();
  const wss = new WebSocketServer({ noServer: true, maxPayload: MAX_BYTES });

  const send = (ws: WebSocket, m: FromRelay) => {
    if (ws.readyState === ws.OPEN) ws.send(JSON.stringify(m));
  };
  const members = (group: string) => [...clients.values()].filter((c) => c.id && c.group === group);
  const announce = (group: string) => {
    const list = members(group);
    const code = group.startsWith('code:') ? group.slice(5) : '';
    for (const c of list) send(c.ws, { t: 'peers', you: c.id, code, peers: list.filter((p) => p !== c).map((p) => ({ id: p.id, name: p.name })) });
  };

  // connections that stop answering pings are dropped (a sleeping laptop, a lost network)
  const beat = setInterval(() => {
    for (const c of clients.values()) {
      if (!c.alive) {
        c.ws.terminate();
        continue;
      }
      c.alive = false;
      c.ws.ping();
    }
  }, o.keepalive ?? 20_000);
  beat.unref?.();

  wss.on('connection', (ws: WebSocket, req: IncomingMessage) => {
    const net = `net:${networkKey(clientAddress(req))}`;
    const c: Client = { ws, id: '', name: '', group: net, tokens: rate * 2, last: Date.now(), alive: true };
    clients.set(ws, c);
    ws.on('pong', () => (c.alive = true));

    ws.on('message', (raw) => {
      c.alive = true;
      // a token bucket: `rate` a second, bursts of twice that
      const now = Date.now();
      c.tokens = Math.min(rate * 2, c.tokens + ((now - c.last) / 1000) * rate);
      c.last = now;
      if (c.tokens < 1) return;
      c.tokens--;
      let m: ToRelay;
      try {
        m = JSON.parse(String(raw));
      } catch {
        return send(ws, { t: 'error', message: 'not JSON' });
      }
      if (!m || typeof m !== 'object') return;
      switch (m.t) {
        case 'hello': {
          if (typeof m.id !== 'string' || !ID.test(m.id)) return send(ws, { t: 'error', message: 'bad id' });
          const code = typeof m.code === 'string' ? normalCode(m.code) : '';
          const group = code ? `code:${code}` : net;
          if (members(group).filter((x) => x !== c).length >= maxPerGroup) return send(ws, { t: 'error', message: 'the group is full' });
          // the same instance again on a new connection: the old one goes
          const old = byId.get(m.id);
          if (old && old !== c) {
            clients.delete(old.ws);
            old.ws.terminate();
          }
          const before = c.id ? c.group : '';
          c.id = m.id;
          c.name = String(m.name ?? '').slice(0, 40);
          c.group = group;
          byId.set(c.id, c);
          if (before && before !== group) announce(before);
          if (old && old !== c && old.group !== group) announce(old.group);
          announce(group);
          break;
        }
        case 'signal':
        case 'relay': {
          if (!c.id) return;
          // only to members of the same group
          const to = Array.isArray(m.to) ? m.to.slice(0, maxPerGroup) : [m.to];
          for (const id of to) {
            const x = typeof id === 'string' ? byId.get(id) : undefined;
            if (!x || x === c || x.group !== c.group) continue;
            send(x.ws, m.t === 'signal' ? { t: 'signal', from: c.id, data: m.data } : { t: 'relay', from: c.id, data: m.data });
          }
          break;
        }
        case 'ping':
          send(ws, { t: 'pong', n: Number(m.n) || 0 });
          break;
      }
    });

    ws.on('close', () => {
      if (clients.get(ws) !== c) return;
      clients.delete(ws);
      if (c.id && byId.get(c.id) === c) byId.delete(c.id);
      if (c.id) announce(c.group);
    });
    ws.on('error', () => ws.terminate());
  });

  const stats = () => ({ clients: clients.size, groups: new Set([...clients.values()].filter((c) => c.id).map((c) => c.group)).size });

  return {
    upgrade(req, socket, head) {
      const path = (req.url ?? '').split('?')[0];
      if (path !== RELAY_PATH) return false;
      if (!sameSite(req) || clients.size >= MAX_CLIENTS) {
        socket.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n');
        return true;
      }
      wss.handleUpgrade(req, socket, head, (ws) => wss.emit('connection', ws, req));
      return true;
    },
    http(req, res) {
      if ((req.url ?? '').split('?')[0] !== `${RELAY_PATH}/health`) return false;
      const s = stats();
      res.writeHead(200, { 'content-type': 'text/plain', 'cache-control': 'no-store' });
      res.end(`ok ${s.clients} ${s.groups}\n`);
      return true;
    },
    stats,
    close() {
      clearInterval(beat);
      for (const ws of clients.keys()) ws.terminate();
      clients.clear();
      byId.clear();
      wss.close();
    },
  };
}

/** The relay on its own HTTP server (production: nginx forwards /sync to it). */
export function startRelay(port: number, host = '127.0.0.1', o: RelayOptions = {}): Promise<{ port: number; relay: Relay; close(): Promise<void> }> {
  const relay = createRelay(o);
  const server = createServer((req, res) => {
    if (!relay.http(req, res)) {
      res.writeHead(404);
      res.end();
    }
  });
  server.on('upgrade', (req, socket, head) => {
    if (!relay.upgrade(req, socket, head)) socket.destroy();
  });
  return new Promise((ok, fail) => {
    server.once('error', fail);
    server.listen(port, host, () => {
      const addr = server.address();
      ok({
        port: typeof addr === 'object' && addr ? addr.port : port,
        relay,
        close: () =>
          new Promise<void>((done) => {
            relay.close();
            server.close(() => done());
          }),
      });
    });
  });
}
