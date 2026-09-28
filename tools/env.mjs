// Shared helpers for the build scripts.
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

// rustup installs into ~/.cargo/bin, which shells started before the install
// (or GUI-launched dev servers) may not have on PATH.
export function toolEnv() {
  const bin = path.join(os.homedir(), '.cargo', 'bin');
  const PATH = [bin, process.env.PATH ?? ''].join(path.delimiter);
  return { ...process.env, PATH };
}

export function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { cwd: ROOT, env: toolEnv(), stdio: 'inherit', ...opts });
  if (r.error) throw r.error;
  return r.status ?? 1;
}

export function has(cmd) {
  const r = spawnSync(cmd, ['--version'], { env: toolEnv(), stdio: 'ignore' });
  return !r.error && r.status === 0;
}
