# birdsynth

A wavetable synthesizer for the browser, modelled on Serum 2. The DSP engine is
written in Rust and compiled to WebAssembly (simd128). It runs inside an
AudioWorklet, and the UI is Svelte 5 + TypeScript. An optional explainer layer
shows how each stage works, with live signals tapped from the engine.

**Status:** Phase 0 (tracer bullet) is complete. It plays a band-limited saw
from 8 voices with an amp envelope, driven from the QWERTY or on-screen
keyboard, and has the full engine ABI, tests, and a staging deploy at
https://birdsynth.abusing.technology.
[docs/plan.md](docs/plan.md) has the roadmap to Serum 2 parity.

## Requirements

- Node 24 or newer (26 is tested)
- Rust through **rustup**, which reads `rust-toolchain.toml` and installs the
  pinned toolchain with the `wasm32-unknown-unknown` target. Homebrew's `rust`
  can't build for wasm.
- `wasm-opt` from binaryen (`brew install binaryen`). It's optional, but release
  builds use it.
- Docker with buildx, for deploying.

## Commands

| Command | What it does |
|---|---|
| `npm run dev` | Vite dev server on :5173. It rebuilds `engine.wasm` whenever Rust or the spec changes, then reloads. |
| `npm test` | Builds the wasm, then runs `cargo test` and vitest (engine in Node, worklet in `node:vm`, protocol and params). |
| `npm run build` | Production build into `dist/`. |
| `npm run preview` | Serves `dist/` on :4173. |
| `npm run gen` | Regenerates the codecs from `params/` and `schema/`. |
| `npm run check` | svelte-check type checking. |
| `deploy/deploy.sh` | Builds the site and runs it on the VM (birdie@192.168.2.165) on port **8001**. |

## Playing

Keys `A`–`'` are white notes and `W E T Y U O P` are black notes. `Z`/`X` shift
the octave and `C`/`V` change the velocity. You can also click or drag on the
on-screen keyboard. `window.__synth` in the console drives everything too, for
example `__synth.noteOn(60)`, `__synth.midiIn([0x90, 64, 100])` or
`__synth.telemetry()`.

## Layout

```
params/*.toml          parameters: the single source of truth (ranges, curves, explainer text)
schema/protocol.toml   binary command layout, telemetry slots and taps
tools/gen-schema.mjs   generates crates/engine/src/spec/*.rs and web/src/gen/*.ts
crates/dsp             shared DSP: mip-mapped table layout, phase, math, RNG, the built-in saw
crates/engine          voices, envelopes, command queue, render loop, taps and telemetry
crates/engine-wasm     the C ABI (engine.wasm, no imports, allocation-counting allocator)
web/src/audio          worklet processor, host, block pool, tap history
web/src/state          ParamBank and parameter math
web/src/ui             knob, keyboard, scope, meter
tests/                 vitest suites
deploy/                Containerfile, nginx.conf, compose.yaml, deploy.sh
```

## How the pieces talk

The main thread writes binary command batches (see `schema/protocol.toml`). It
coalesces them per microtask and posts them to the worklet, which copies them
into the engine's command buffer in wasm memory. Every command carries an
absolute frame. Note-ons start on that exact frame; everything else lands on
the next 16-frame control boundary. Once per 1024 frames the worklet sends back
a pooled, transferable block holding the telemetry and the selected taps. Each
block is stamped with its frame, so scopes can draw what you're hearing right
now. If the engine traps, the worklet re-instantiates it within a quantum and
the host resends the parameters and the held notes.
