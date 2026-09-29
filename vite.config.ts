import { spawn } from 'node:child_process';
import type { Server as HttpServer } from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Connect, type Plugin } from 'vite';
import { svelte, vitePreprocess } from '@sveltejs/vite-plugin-svelte';
import { createRelay } from './server/relay.ts';

const ROOT = path.dirname(fileURLToPath(import.meta.url));

function buildWasm(): Promise<boolean> {
  return new Promise((resolve) => {
    const p = spawn(process.execPath, ['tools/build-wasm.mjs', '--quiet'], { cwd: ROOT, stdio: 'inherit' });
    p.on('exit', (code) => resolve(code === 0));
    p.on('error', () => resolve(false));
  });
}

// Builds engine.wasm before Vite starts, and in dev rebuilds it whenever the
// Rust code or the spec changes, then reloads the page.
function engineWasm(): Plugin {
  return {
    name: 'birdsynth-engine-wasm',
    async buildStart() {
      if (!(await buildWasm())) this.error('engine.wasm failed to build; see the cargo output above');
    },
    configureServer(server) {
      const dirs = ['crates', 'params', 'schema'].map((d) => path.join(ROOT, d) + path.sep);
      server.watcher.add(dirs);
      let timer: ReturnType<typeof setTimeout> | undefined;
      let running = false;
      let again = false;
      const rebuild = async () => {
        if (running) {
          again = true;
          return;
        }
        running = true;
        const ok = await buildWasm();
        running = false;
        if (again) {
          again = false;
          return rebuild();
        }
        if (ok) server.ws.send({ type: 'full-reload' });
        else server.config.logger.error('engine.wasm build failed; keeping the previous build');
      };
      const onChange = (file: string) => {
        if (!/\.(rs|toml)$/.test(file) || !dirs.some((d) => file.startsWith(d))) return;
        clearTimeout(timer);
        timer = setTimeout(rebuild, 150);
      };
      server.watcher.on('change', onChange);
      server.watcher.on('add', onChange);
    },
  };
}

// The link service (server/relay.ts) at /sync on the dev and preview
// servers, as nginx has it in production.
function linkService(): Plugin {
  const attach = (http: HttpServer | null | undefined, middlewares: Connect.Server) => {
    if (!http) return;
    const relay = createRelay();
    http.on('upgrade', (req, socket, head) => relay.upgrade(req, socket, head));
    http.on('close', () => relay.close());
    middlewares.use((req, res, next) => {
      if (!relay.http(req, res)) next();
    });
  };
  return {
    name: 'birdsynth-link-service',
    configureServer(server) {
      attach(server.httpServer as HttpServer | null, server.middlewares);
    },
    configurePreviewServer(server) {
      attach(server.httpServer as HttpServer, server.middlewares);
    },
  };
}

export default defineConfig({
  root: 'web',
  publicDir: 'public',
  plugins: [engineWasm(), linkService(), svelte({ configFile: false, preprocess: vitePreprocess() })],
  worker: { format: 'es' },
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    target: 'es2023',
    assetsInlineLimit: 0,
  },
  server: { port: 5173, strictPort: true },
  preview: { port: 4173, strictPort: true },
});
