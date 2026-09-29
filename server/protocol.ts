// What instances and the link service say to each other (server/relay.ts;
// the instances' side is web/src/sync/net.ts). Plain JSON over a WebSocket.

/** What an instance sends. */
export type ToRelay =
  | { t: 'hello'; id: string; name: string; code?: string }
  | { t: 'signal'; to: string; data: unknown }
  | { t: 'relay'; to: string | string[]; data: unknown }
  | { t: 'ping'; n: number };

/** What the service sends. `code` is the group code in use ('' for the network's own group). */
export type FromRelay =
  | { t: 'peers'; you: string; code: string; peers: { id: string; name: string }[] }
  | { t: 'signal'; from: string; data: unknown }
  | { t: 'relay'; from: string; data: unknown }
  | { t: 'pong'; n: number }
  | { t: 'error'; message: string };

export const RELAY_PATH = '/sync';

/** A group code as typed: case, spaces and punctuation don't matter. */
export const normalCode = (code: string): string => code.toUpperCase().replace(/[^A-Z0-9]/g, '').slice(0, 16);
