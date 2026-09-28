# birdsynth: working notes

A Serum 2-style wavetable synth. The Rust engine is compiled to wasm and runs in
an AudioWorklet; the UI is Svelte 5 + TS. The roadmap and each phase's gates
are in `docs/plan.md`. P0–P5 are done and the site is public; P6 (the
sample-based oscillators) comes next.

## Commands
- The shell may lack `~/.cargo/bin`; use `. "$HOME/.cargo/env"` or `node tools/cargo.mjs …`.
- `npm test` runs everything: it builds the wasm, then `cargo test --workspace`, then vitest.
- Dev server: `preview_start {name: "birdsynth"}` (from `.claude/launch.json`) on :5173. The Vite plugin rebuilds `engine.wasm` when `.rs`/`.toml` files change.
- After editing `params/` or `schema/`, run `npm run gen`. A test fails if the generated files are stale. Never hand-edit `crates/engine/src/spec/*` or `web/src/gen/*`.

## Layout notes
- `crates/dsp`: mip layout (`mip.rs`), warps (`warp.rs`, shared by the engine and previews), the filters (`filter.rs`, 63 types on shared cores, with `response` for graphs), halfbands (`oversample.rs`), `hilbert.rs`, a small power-of-two FFT (`fft.rs`, the engine avoids rustfft's size), the convolver's IR layout (`conv.rs`, shared with the tools), phase/math/rng, the built-in saw.
- `crates/engine/src/fx/`: the racks (`mod.rs`: chains, click-free bypass and reorder, splitter band chains) and one file per effect family. Four instances per type; chain entries are type × 256 + instance. Big buffers live on the heap (`Fx::new` must never build modules on the stack: the wasm stack is 1 MB).
- `crates/engine`: `engine.rs` (commands, allocation, oversampling, the global-modulation policy), `voice.rs` (sources → routing → filters → buses, the fused cross-mod loop), `osc/` (kernel with scalar + SIMD + the per-sample `step`, unison, sub), `filter/` (block and per-sample `tick`), `lfo.rs`, `env.rs`, `modmatrix.rs`, `samples.rs` (noise slot), `tables.rs` (host-filled assets), `params.rs` (smoothing), `tests/` (`mod.rs` P1 gates, `p2.rs` P2 gates).
- Voices share one `Scratch` (engine-owned) and clear only what they use; per-sample routing exists only for filter-sourced cross-mod, otherwise routing is per source block (`route_block`), same arithmetic.
- Web state: `state/matrix.ts` (slots + `evaluate`, the reference model the engine is tested against), `state/lfo.ts` (shapes, mirroring `lfo.rs`), `state/noise.ts` (noise sample via the tools worker → worklet 'sample' upload → LoadSample).
- Taps: at most 8 per block. Displays call `synth.useTap(name)` while shown; `__synth.tap(name)` keeps what it reads.
- Drag-to-modulate uses pointer events (`ui/mod/drag.ts`); knobs with `data-mod="1"` are drop targets. Alt-drag on a modulated knob changes the first routing's amount.
- `crates/tools` + `tools-wasm`: mip builder, factory tables, resampling, previews (the kernels' bit-exact reference), noise, factory IRs (`ir.rs`), IR preparation, filter responses; runs in `web/src/tools/worker.ts`.
- Web FX state: `state/fx.ts` (racks, instances, module presets), `state/ir.ts` (convolver responses: factory at the engine's rate, or dropped files).
- Commands carry wasm32 pointers (u32): native Rust tests must not pass real pointers through commands (load tables with `tables_mut()` instead).
- Enum option lists in params/*.toml only grow at the end (patches will store option names).
- Patches (`state/patch.ts`): params by key, only where they differ from the default; matrix, LFO shapes, remap curves and FX chains by name; wavetables and IRs from files by SHA-256 hash. `migrate()` brings old versions forward. Bump `PATCH_VERSION` and add a migration (plus a fixture in `tests/fixtures/patches/`) for any format change.
- Factory presets (`presets/factory.ts`) are written in plain units. Each sets its own `master.volume`, levelled by `tests/patch.test.ts` (it prints the value to use when a preset is off level).
- The library (`state/library.ts`) keeps user presets, factory ratings and assets in IndexedDB (memory fallback); undo (`state/history.ts`) snapshots the five stores once a gesture settles (400 ms) and resets when a preset loads. MIDI learn (`input/learn.ts`) and tuning (`state/tuning.ts`, parsers in `tuning/tuning.ts`) belong to the setup, not the patch: both live in localStorage.
- Explainer: `explain/content.ts` (notes per data-explain key; parameters use their spec text), `explain/Overlay.svelte` (explain mode, callouts, the tour card), `explain/tours.ts` (steps can load a teaching patch, switch band-limiting off, turn pages). `tests/explain.test.ts` fails if a key in the markup has no notes; add new `data-explain={...}` templates to its EXPANSIONS.
- Wavetable editor: the maths is in `crates/tools/src/wt/` (spectrum, process, morph, formula, import), exposed as `tl_wt_*`, `tl_formula*`, `tl_pitch`, `tl_import`. `web/src/tools/handle.ts` is the worker's request handler (tests drive it on tools.wasm in Node; `useTools()` swaps the worker for it). `web/src/editor/model.ts` holds the table being edited, its own undo, and sends changed frames with `TableStore.setFrame` (reshapes replace the table). Editing a factory table re-labels its source `edit:…`, so patches carry its frames.
- Canvases inside grid or flex cells sit `position: absolute` in a relative wrapper: a canvas sized from its own pixels otherwise grows the row it's measured from. Long content (the frame strip) needs `min-width: 0` up the chain; the stage has one `minmax(0, 1fr)` column.
- The faceplate's `.viewport` uses `overflow: clip`, not hidden: a hidden box still scrolls on focus() or scrollIntoView and slides the panel sideways.

## Invariants (all tested)
- `engine.wasm` imports nothing. Keep `getrandom`/wasm-bindgen out of the dependency tree.
- `wt_render` and `wt_apply` never allocate once running (counted by the allocator in `crates/engine-wasm`).
- `process()` in `web/src/audio/worklet/processor.ts` creates no typed arrays. Views are made at instantiate time or when a block returns.
- Frames per render must be a multiple of 16 (`SUB_BLOCK`). Params land on the 16-frame grid; note-ons are frame-exact.
- The TS param mapping (`web/src/state/param-math.ts`) must match `ParamInfo::to_plain` in Rust.
- AudioWorkletGlobalScope has no TextDecoder, performance, fetch or setTimeout. The panic message is decoded as ASCII.

## Verifying in the Browser pane
- In the pane the AudioContext runs without a gesture, and MIDI is denied: use `__synth.midiIn([...])`.
- `__synth.telemetry()`, `__synth.telemetryAll()`, `__synth.tap('master.l', n)`, `__synth.matrix()` and `__synth.stats()` (with `underrunEvents`) are the proof points. Presets: `presets()`, `loadPreset(name)`, `savePatch()`/`loadPatch(text)`, `undo()`, `commit()`. MIDI: `learn(key)` then `midiIn([0xB0, cc, v])`, `learned()`. Explainer: `explain.missing()` per page (`page('fx')`), `explain.open(key)`, `explain.inharmonic()`; tours: `tour.start('aliasing')`, `tour.next()`, `tour.settled()`, `tour.state()` (each step's `expect`). Editor: `editor.open(0)`, `editor.formula(src)`, `editor.draw(points)`, `editor.morph(mode, n)`, `editor.importTone(hz, secs, mode)`, `editor.latency()` (pen → tap ms). `__synth.setPlain(key, value)` sets a parameter in its own unit. Find knobs by their ARIA name, e.g. `find("Osc A Level")`.
- Pane coordinates: the screenshot frame is smaller than the CSS viewport (e.g. 800 × 758 for 1101 × 1044); convert with the ratio before `left_click_drag`. Viewport emulation (`resize_window`) skews drag coordinates further, so test drags at the pane's own size.
- Smoothed parameters glide for about 0.2 s (12 ms time constant) before they snap to the target; tests that compare against exact values render ~0.5 s first.
- Measure pitch with a least-squares fit through all rising zero crossings of `master.l` (as `pitch()` in `tests/engine/harness.ts` does). First/last crossings alone scatter ±0.01 Hz on 8192 frames.

## Deploy
- `deploy/deploy.sh` streams the image to birdie@192.168.2.165 over SSH. It runs as compose project `birdsynth` in `~/birdsynth`, container `birdsynth`, on port **8001**. It only works from the 192.168.2.x network.
- Public URL: **https://birdsynth.abusing.technology** (route added by the user 2026-09-28). It goes through the Cloudflare tunnel, which is managed in the dashboard (token-based cloudflared on the VM), so route changes are the user's step.
- Through Cloudflare, hashed JS keeps our 1-year immutable cache-control (edge HIT), the wasm passes through uncached (`DYNAMIC`), and HTTP gets a 301 to HTTPS.
- nginx serves `/assets/*` as immutable and `index.html` as no-cache. The site is public since P4 (no noindex; `robots.txt` allows all).
- Checking a deploy: `node tools/smoke.mjs` (page, headers, wasm types and imports, http→https, robots), then load it in a fresh pane tab, play a preset and read the console.

## Conventions
- Our own name, visuals and content: no Serum assets and no Vital (GPL) code.
- Commit locally at the end of each phase. Don't push unless asked.
