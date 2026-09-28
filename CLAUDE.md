# birdsynth: working notes

A Serum 2-style wavetable synth. The Rust engine is compiled to wasm and runs in
an AudioWorklet; the UI is Svelte 5 + TS. The roadmap and each phase's gates
are in `docs/plan.md`. Phase 0 is done; P1 (the wavetable oscillator core)
comes next.

## Commands
- The shell may lack `~/.cargo/bin`; use `. "$HOME/.cargo/env"` or `node tools/cargo.mjs …`.
- `npm test` runs everything: it builds the wasm, then `cargo test --workspace`, then vitest.
- Dev server: `preview_start {name: "birdsynth"}` (from `.claude/launch.json`) on :5173. The Vite plugin rebuilds `engine.wasm` when `.rs`/`.toml` files change.
- After editing `params/` or `schema/`, run `npm run gen`. A test fails if the generated files are stale. Never hand-edit `crates/engine/src/spec/*` or `web/src/gen/*`.

## Invariants (all tested)
- `engine.wasm` imports nothing. Keep `getrandom`/wasm-bindgen out of the dependency tree.
- `wt_render` and `wt_apply` never allocate once running (counted by the allocator in `crates/engine-wasm`).
- `process()` in `web/src/audio/worklet/processor.ts` creates no typed arrays. Views are made at instantiate time or when a block returns.
- Frames per render must be a multiple of 16 (`SUB_BLOCK`). Params land on the 16-frame grid; note-ons are frame-exact.
- The TS param mapping (`web/src/state/param-math.ts`) must match `ParamInfo::to_plain` in Rust.
- AudioWorkletGlobalScope has no TextDecoder, performance, fetch or setTimeout. The panic message is decoded as ASCII.

## Verifying in the Browser pane
- In the pane the AudioContext runs without a gesture, and MIDI is denied: use `__synth.midiIn([...])`.
- `__synth.telemetry()`, `__synth.tap('master.l', n)` and `__synth.stats()` (with `underrunEvents`) are the proof points. Find knobs by their ARIA name, e.g. `find("Osc A Level")`.
- Measure pitch with a least-squares fit through all rising zero crossings of `master.l` (as `pitch()` in `tests/engine/harness.ts` does). First/last crossings alone scatter ±0.01 Hz on 8192 frames.

## Deploy
- `deploy/deploy.sh` streams the image to birdie@192.168.2.165 over SSH. It runs as compose project `birdsynth` in `~/birdsynth`, container `birdsynth`, on port **8001**. It only works from the 192.168.2.x network.
- Public URL: **https://birdsynth.abusing.technology** (route added by the user 2026-09-28). It goes through the Cloudflare tunnel, which is managed in the dashboard (token-based cloudflared on the VM), so route changes are the user's step.
- Through Cloudflare, hashed JS keeps our 1-year immutable cache-control (edge HIT), the wasm passes through uncached (`DYNAMIC`), and HTTP gets a 301 to HTTPS.
- nginx serves `/assets/*` as immutable and `index.html` as no-cache, with `X-Robots-Tag: noindex` until the P4 public launch.

## Conventions
- Our own name, visuals and content: no Serum assets and no Vital (GPL) code.
- Commit locally at the end of each phase. Don't push unless asked.
