#!/usr/bin/env node
// Builds web/wasm/engine.wasm:
//   1. regenerate the codecs from the spec,
//   2. cargo build --release for wasm32-unknown-unknown,
//   3. wasm-opt -O3 (when binaryen is installed),
//   4. check the module imports nothing and matches the spec's ABI hash.
import fs from 'node:fs';
import path from 'node:path';
import { ROOT, has, run } from './env.mjs';

const OUT_DIR = path.join(ROOT, 'web', 'wasm');
const SRC = path.join(ROOT, 'target', 'wasm32-unknown-unknown', 'release', 'wt_engine_wasm.wasm');
const OUT = path.join(OUT_DIR, 'engine.wasm');
const quiet = process.argv.includes('--quiet');

function step(name, status) {
  if (status !== 0) {
    console.error(`build-wasm: ${name} failed`);
    process.exit(status || 1);
  }
}

step('gen-schema', run('node', ['tools/gen-schema.mjs'], { stdio: quiet ? 'ignore' : 'inherit' }));
step('cargo build', run('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'wt-engine-wasm', ...(quiet ? ['--quiet'] : [])]));

fs.mkdirSync(OUT_DIR, { recursive: true });
if (has('wasm-opt')) {
  step('wasm-opt', run('wasm-opt', ['-O3', SRC, '-o', OUT]));
} else {
  console.warn('build-wasm: wasm-opt not found (brew install binaryen); copying the unoptimized module');
  fs.copyFileSync(SRC, OUT);
}

const bytes = fs.readFileSync(OUT);
const mod = new WebAssembly.Module(bytes);
const imports = WebAssembly.Module.imports(mod);
if (imports.length) {
  console.error('build-wasm: engine.wasm must import nothing, but imports:', imports);
  process.exit(1);
}
const { instance } = await WebAssembly.instantiate(bytes, {});
const hash = instance.exports.wt_abi_hash() >>> 0;
const { ABI_HASH } = await import('../web/src/gen/protocol.ts');
if (hash !== ABI_HASH) {
  console.error(`build-wasm: ABI hash 0x${hash.toString(16)} does not match the spec's 0x${ABI_HASH.toString(16)}`);
  process.exit(1);
}
if (!quiet) console.log(`build-wasm: web/wasm/engine.wasm ${(bytes.length / 1024).toFixed(1)} KiB, no imports, ABI 0x${hash.toString(16).padStart(8, '0')}`);
