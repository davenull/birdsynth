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
const TARGET = path.join(ROOT, 'target', 'wasm32-unknown-unknown', 'release');
const MODULES = [
  { crate: 'wt-engine-wasm', src: 'wt_engine_wasm.wasm', out: 'engine.wasm' },
  { crate: 'wt-tools-wasm', src: 'wt_tools_wasm.wasm', out: 'tools.wasm' },
];
const quiet = process.argv.includes('--quiet');

function step(name, status) {
  if (status !== 0) {
    console.error(`build-wasm: ${name} failed`);
    process.exit(status || 1);
  }
}

step('gen-schema', run('node', ['tools/gen-schema.mjs'], { stdio: quiet ? 'ignore' : 'inherit' }));
step('cargo build', run('cargo', ['build', '--release', '--target', 'wasm32-unknown-unknown', ...MODULES.flatMap((m) => ['-p', m.crate]), ...(quiet ? ['--quiet'] : [])]));

fs.mkdirSync(OUT_DIR, { recursive: true });
const opt = has('wasm-opt');
if (!opt) console.warn('build-wasm: wasm-opt not found (brew install binaryen); copying unoptimized modules');
const sizes = [];
for (const m of MODULES) {
  const src = path.join(TARGET, m.src);
  const out = path.join(OUT_DIR, m.out);
  if (opt) step(`wasm-opt ${m.out}`, run('wasm-opt', ['-O3', src, '-o', out]));
  else fs.copyFileSync(src, out);
  const bytes = fs.readFileSync(out);
  const imports = WebAssembly.Module.imports(new WebAssembly.Module(bytes));
  if (imports.length) {
    console.error(`build-wasm: ${m.out} must import nothing, but imports:`, imports);
    process.exit(1);
  }
  sizes.push(`${m.out} ${(bytes.length / 1024).toFixed(1)} KiB`);
}

const { instance } = await WebAssembly.instantiate(fs.readFileSync(path.join(OUT_DIR, 'engine.wasm')), {});
const hash = instance.exports.wt_abi_hash() >>> 0;
const { ABI_HASH } = await import('../web/src/gen/protocol.ts');
if (hash !== ABI_HASH) {
  console.error(`build-wasm: ABI hash 0x${hash.toString(16)} does not match the spec's 0x${ABI_HASH.toString(16)}`);
  process.exit(1);
}
if (!quiet) console.log(`build-wasm: ${sizes.join(', ')}; no imports; ABI 0x${hash.toString(16).padStart(8, '0')}`);
