// Runs the link service on its own (in the site's container, beside nginx):
//
//   node server/main.ts                 (Node 23.6+ runs the TypeScript as is)
//   node dist-server/relay.cjs          (the bundle `npm run build` makes)
//
// RELAY_PORT (default 8002) and RELAY_HOST (default 127.0.0.1: only nginx,
// on the same machine, reaches it).

import { startRelay } from './relay.ts';

const port = Number(process.env.RELAY_PORT) || 8002;
const host = process.env.RELAY_HOST || '127.0.0.1';

startRelay(port, host).then(
  (r) => {
    console.log(`birdsynth link service on ${host}:${r.port}`);
    const stop = () => void r.close().then(() => process.exit(0));
    process.on('SIGTERM', stop);
    process.on('SIGINT', stop);
  },
  (e: unknown) => {
    console.error('birdsynth link service failed to start:', e);
    process.exit(1);
  },
);
