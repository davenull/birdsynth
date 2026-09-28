# birdsynth: a Serum 2–style wavetable synth (browser app, Rust→WASM engine, explainer layer)

**Status:** P0–P3 finished 2026-09-28; every gate so far passes. Staging runs on the VM at :8001, reachable at https://birdsynth.abusing.technology (noindex until P4). Next is P4.

P1 measurements: pitch within ±0.5 cent C0–C8 at 44.1/48/96 kHz; saw aliasing ≤ −60 dBc (20 Hz–12 kHz fundamentals, audible band); worst centroid step 0.88% over a 2-octave glide (audible band); SIMD = scalar bit for bit; filters within ±0.3 dB of analytic; 16 voices × 16 unison = 2.9% of real time in Node (scalar 4.5%).

P2 measurements: envelope segments end on the 16-frame grid (≤ 0.33 ms late) at 120 and 60 BPM and 2× rate; LFO periods exact to < 0.1% (Free and Retrig, 0.37–31 Hz over 60 s); BPM-synced LFOs drift < 1e-4 cycles (free) and < 1e-3 (retriggered) over 60 s; the matrix matches the TS model over 1,000 random routings (curves, aux, output, bipolar, bypass) to 2e-5; the fused FM loop (A←B and the A↔B cycle, 4 × 3 unison lanes) is bit-exact against an independent reference; hard-sync aliasing at C6 is −34.7 / −41.8 / −46.5 dB at 1× / 2× / 4×. Heavy patch without FX (16 voices, 3 osc × 16 unison, two warps each, 2 driven ladders, 16 matrix slots) = 16.8% of real time in Node, about 19% in the browser worklet with 0 underruns over 30 s. The fuller voice costs 16 × 16 unison 2.9% → 4.1%.

P3 measurements: delay echoes land on the sample (±1) in all three modes; all five reverbs decay within ±10% of the set RT60 at 1 s and 3 s; the EQ is within ±0.3 dB of its analytic response; the compressor sits within ±0.5 dB of its static curve from -40 to 0 dB; the splitter (2 and 3 bands), the multiband compressor at rest and mono bass all sum flat within ±0.1 dB; the frequency shifter is within ±0.1 Hz; zero-latency convolution matches direct convolution below -100 dB (40 to 20,000 taps); distortion aliasing is ≤ -70 dBc for the smooth curves at 4× (the hard-edged ones gain ≥ 10 dB over no oversampling); no subnormal state after an impulse and 60 s of silence (effects and all 63 filter types); bypass and reorder never jump more than 0.03 on a 50 Hz sine; all 63 filter types stay finite and bounded under audio-rate cutoff modulation at 1× and 4×; every warp (46 modes, 4 amounts, either slot) is bit-exact against its preview in both kernels. Heavy patch with full FX (12 effects in Main including a 2.6 s convolution, a bus reverb, 16 matrix slots) = 22.6% of real time in Node; engine.wasm 354 KiB. Filter types came to 63 (not ~90), each on a shared core; FX instances are four per type.

## Context
You want a full wavetable synthesizer in the style of Serum. The choices so far:
- **Platform:** a **web app with a Rust→WASM (simd128) DSP engine** running in an AudioWorklet.
- **Scope:** **Serum 2 feature parity** (checklist in the appendix).
- **Explainer:** an "**instrument + explainer**" layer, meaning annotated signal flow, live taps and tours, as on the superhet site.
- **Rust toolchain:** **replace Homebrew's `rust` with rustup**. Homebrew rust 1.98.1 was installed at 11:58 today; it only ships the aarch64-apple-darwin std and ignores `rust-toolchain.toml`.
- **Hosting:** give the synth **its own container and hostname on the VM** (port 8001, behind the Cloudflare tunnel). You add the hostname route in Cloudflare.

The project lives in the empty directory `/Users/david.fogle/wavetable`. The machine has an M5 Pro with 48 GB, Node 26, npm 11, Homebrew, CMake and clang 21, with Command Line Tools only.

The `modulation_demo` sites are plain JS with no build step, no FFT, no voices and no MIDI. Their patterns carry over, but little of their code does.

**Outcome:** a synth that plays and sounds like Serum 2, delivered in phases. Each phase ends as a playable build with numeric acceptance gates.

**IP:** we use our own name (**birdsynth**), our own visuals, and our own wavetables, noises, IRs and presets. We don't copy Serum assets or Vital's GPL code. Serum-format wavetable WAV import and export is fine.

**Browser facts,** measured in the Browser pane (Chromium 152) and in Node 26 during review:
- **Missing in AudioWorkletGlobalScope:** `TextDecoder`, `performance`, `fetch`, `setTimeout`, `queueMicrotask`, `crypto`, `structuredClone`, `MessageChannel`.
- **Present:** `Date.now`, `console`, and synchronous `new WebAssembly.Module(bytes)`.
- **Passing a compiled module:** `port.postMessage(WebAssembly.Module)` fails silently, so we pass bytes instead.
- **Memory:** views over `memory.toResizableBuffer()` survive `grow`. This works in Chrome 144+, Firefox 145+, Safari 26.2+ and Node 26.
- **Metrics:** `AudioContext.renderCapacity` is absent. `AudioContext.playbackStats.underrunEvents` is present (Chrome 146+).
- **Web MIDI:** Chrome and Firefox support it; Safari doesn't. In the pane, MIDI permission is denied, and the AudioContext starts running without a gesture.
- **SIMD:** ship plain simd128. Relaxed-SIMD isn't in Safari.

---

## Architecture

```
 Main thread (Svelte 5 + TS)          AudioWorklet                          Tools Web Worker
 UI · ParamBank · undo · IndexedDB ─cmd batches (ArrayBuffer)→ processor ─→ engine.wasm      tools.wasm
 MIDI/QWERTY/MPE · __synth test API ←telemetry+taps block/1024 frames─      (Rust, 0 imports) (mips, previews,
 shared rAF draw loop                                                                          STFT, formula VM)
```

**Rust workspace**
- `crates/dsp`: shared primitives.
  - fft (realfft/rustfft, planned at init, scratch pre-allocated)
  - interpolation, fast math, a seeded RNG
  - onepole, biquad and TPT SVF cores; halfband oversamplers; delay lines
  - the table/mip layout; WAV and `clm ` parsing
- `crates/engine`: voices, oscillators, warps, filters, modulation, FX, seq, command decoder, telemetry and taps.
- `crates/tools`: mip builder; previews and filter responses, which reuse the engine code so they match exactly; STFT and grain pyramids; resample; pitch detection; formula VM; factory generators.
- `crates/engine-wasm` and `crates/tools-wasm`: raw `extern "C"` cdylibs with no wasm-bindgen. Built with `panic=abort`, `lto`, `codegen-units=1` and `wasm-opt -O3`.
  - `engine.wasm` must have **zero imports**. No `getrandom` or wasm-bindgen anywhere in the dependency tree.
  - `tools.wasm` is kept separate, so `engine.wasm` stays small enough for synchronous compilation.
- `crates/cli`: native benchmarks, golden renders and fixture generation.
- **Crates:** `wide::f32x4` for SIMD (simd128 on wasm, NEON natively) and `realfft`.

**Parameters and command schema: one spec**
- The source of truth is per-module TOML under `params/`, for example `params/fx/delay.toml`. Each entry holds:
  - key, name, range, default and curve (lin/exp/pow/dB)
  - unit, flags (mod, smooth) and group
  - instanced groups (`osc.{a,b,c}`, `lfo.{1..10}`, FX instances)
  - `explain` text
- Generated from it:
  - `build.rs` generates the Rust tables and the command decoder.
  - `tools/gen-schema.mjs` generates `web/src/state/params.gen.ts` and the TS command encoder.
- `wt_abi_hash()` is asserted at startup, so a stale cached wasm fails loudly.
- Patches store **string keys**; numeric ids are per build.
- A test fails if the generated files are stale.
- Per-module files also keep parallel work conflict-free.

**Engine rules**
- **Control rate:**
  - The control grid is a fixed 16 samples, and blocks aren't split at events. A new voice starts at its exact frame by masking its first sub-block. Note-offs and param changes land on the grid, at most 0.33 ms late.
  - Interpolation inside a sub-block:
    - pitch: an exponential ramp of the phase increment
    - cutoff: through the TPT `g` coefficient
    - level: linear
    - S&H sources: stepped
- **Wavetables:**
  - Up to 256 frames of 2048 samples, with one mip level per octave (11 levels).
  - Level lengths follow N_L = clamp(8·H_L, 256, 2048). That's 9,216 floats per frame, so **9.4 MB per oscillator** instead of 23 MB.
  - The **level is chosen from peak phase velocity** in each sub-block: base increment × warp stretch × maximum unison detune × maximum pitch modulation. This band-limits bend, asym, PWM and windowed sync without oversampling.
  - Adjacent levels are crossfaded only in the top ¼ octave below each switch point.
  - Slots are double-buffered per oscillator. A partial update per frame lets the editor change one frame at a time.
- **Oversampling:** the voice path runs at 2× or 4× (Quality setting) only when the patch uses FM, PD, AM, RM, hard sync or distortion warps. Voices are summed, then decimated once per output bus. Distortion FX does its own 4× oversampling.
- **SIMD:** padded structure-of-arrays from day one.
  - Unison voices go in lanes for the oscillators.
  - `[vA.L vA.R vB.L vB.R]` for filters, amp and drive.
  - A scalar reference plus an equivalence test for every kernel.
  - Expect 1.5–2× from SIMD, because wasm has no gather instruction.
- **Cross-oscillator modulation:** coupled oscillators (FM, PD, RM, sync) render in a fused per-sample loop. Carrier lane k is driven by modulator lane k mod n. Cycles use the previous sample's value. Uncoupled oscillators use the fast block kernel.
- **Polyphonic → global modulation policy:** written as a table, with tests.
  - FX and global destinations follow the **most recent still-sounding voice**, and hold its values after it ends.
  - The same voice drives telemetry and the mod rings.
  - LFOs have a poly or global mode.
  - Macros and chaos sources broadcast to all voices.
- **Memory:**
  - Voices and buffers are pre-allocated at init, and memory is pre-sized with `--initial-memory`.
  - A counting global allocator asserts **zero allocations inside `wt_render`**.
  - Assets (tables, PCM, IRs) are allocated only on the command path. They're swapped at a block boundary, and the old copy is freed one block later.
  - A panic hook writes its message into a static buffer. JS reads it through `wt_panic_msg`/`len` and decodes it as ASCII, because the worklet has no TextDecoder.
  - Denormals: feedback state is flushed once per block.
- **Granular and spectral oscillators:**
  - Granular uses an engine-wide budget of 512 grains that steals the oldest, reads from an octave-decimated sample pyramid, and has a seeded RNG per voice.
  - Spectral uses an offline STFT asset. Per voice it does warp → phase propagation → 2048-point IFFT at hop 512 → overlap-add, synthesizing the first hop at note-on. Spectral unison is capped.
- **Arp and clip run in the engine** with sample accuracy. A ledger of sounding notes guarantees that clip edits never strand a note-off.
- **Performance gates** are measured in Node, which is the same V8 as Chrome, after warm-up, on this machine:
  - **Typical** patch (8 voices, 2 osc × 7 unison, 1 SVF, reverb and delay): **≤5% of real time**.
  - **Heavy** patch (16 voices, 3 osc × 16 unison, dual warps, 2 ladder filters with drive, 16 matrix slots, full FX, Quality 1×): **≤30%**.
  - The heavy patch at HQ is reported but not gated.
  - A CPU guard kicks in above 70% load: it turns HQ off first, then caps unison.

**Worklet host and protocol (TypeScript)**
- **Loading the engine:** the `.wasm` bytes arrive through `processorOptions`, followed by a synchronous compile. The processor caches the compiled Module, so trap recovery only needs a new `WebAssembly.Instance` and a resend of the patch.
  - `onmessageerror` handlers are attached on both ends.
  - Worklet logs are forwarded to the main thread's console.
- **Memory views:** built on `toResizableBuffer()`, with a `buffer !== cached` fallback check once per `process()`.
- **Warm-up:** about 0.5 s is rendered muted before audio is unmuted.
- **Commands:**
  - Binary batches travel in one ArrayBuffer.
  - Each command carries an **absolute frame**; 0 means as soon as possible.
  - Changes are coalesced **per microtask, not per rAF**, because rAF stops in hidden tabs and panes.
  - Live input is sent as soon as possible. Scheduled events (tours, tests, MIDI files) are mapped to frames through `getOutputTimestamp`.
- **Worklet → main:** one pooled, transferable block per 1024 frames, carrying telemetry and taps, stamped with its frame (the superhet pattern). Taps come as full signals or as decimated min/max for the mini-scopes.
- **Never on the audio thread:** JSON, and posting a Svelte `$state` proxy. Take `$state.snapshot` at the boundary.
- **CPU meter:** accumulated `Date.now()` deltas around `wt_render` over a 1 s window, plus `playbackStats.underrunEvents`.
- **Bundling:** the worklet is bundled with Vite's `?worker&url` and verified with `vite preview`, not only the dev server.

**UI: Svelte 5 + TypeScript + Vite**
- **ParamBank:** a `Float32Array` of normalized values with a subscription per parameter, rather than a deep `$state` holding about 1,500 keys. Structured state (matrix, curves, racks, clips) lives in `$state` objects.
- **Knobs are DOM elements with ARIA labels,** so pane tests can `find("Osc A WT Position")`.
  - Interaction is ported from the superhet dial (`superhet/js/core/dock.js:65-73,119-149`): pointer capture, wheel, keys.
  - Shift for fine adjustment, double-click to reset, Ctrl/Cmd-click to type a value.
  - Mod rings in each source's colour.
- **Drag-to-modulate:** drop a source handle onto any knob.
- **Animation:** one shared rAF loop draws only what's visible (mod dots, scopes, the 3D wavetable).
- **Layout:** a fixed logical faceplate, scaled to fit the window.
  - Pages: OSC, MIX, FX, MATRIX, GLOBAL, plus the ARP/CLIP editors and FLOW.
  - An always-visible lower strip: Env 1–4, LFO 1–10, Macros 1–8, Vel/Note, Wheels, Arp/Clip, keyboard, voicing.
  - A top bar: browser, menu, undo/redo, volume, CPU.
- **Theme:** tokens follow `superhet/css/site.css:14-126`, with a fixed "instrument" palette in the spirit of `--case*`/`--glass-*`. Colour roles stay fixed per source and signal, checked for colour-blind separation. Fonts are self-hosted with `@fontsource`.
- **Dev and test aids:**
  - `window.__synth`: telemetry, `midiIn(bytes, frame)`, `noteOn`/`Off`, `set`/`getParam`, `loadPatch`/`savePatch`, `tap(name)`, `stats`, and `renderOffline` → hash.
  - A deterministic `FakeEngine` and a `/gallery` page of components for screenshots.

**Assets and persistence**
- **Patch format:** versioned JSON with migrations, holding:
  - string-keyed params
  - structured state
  - asset references by content hash
- **What's stored:** source frames and PCM; mips are rebuilt on load in the tools worker.
- **User library:** IndexedDB, with `navigator.storage.persist()` because Safari may evict. Also a single-file patch export (JSON plus embedded assets).
- **Factory content:** named by content hash under `public/factory/`, so it can be cached as immutable.
- **Audio import:** any format, through `decodeAudioData`.

**Explainer**
- **Markup:** a `data-explain="<key>"` attribute on every section and control. Content comes from the TOML `explain` fields plus `web/src/explain/content/`. A lint check requires every key to resolve.
- **Explain mode (`?`):**
  - Dims the synth.
  - Clicking a section opens a callout with what it does, a live tap view (scope or spectrum), and undoable "try this" buttons.
- **Tours:** scripted steps that load a teaching patch, highlight controls, adjust them and show taps. Teaching switches include "band-limiting off", which is engine debug flag, so you can hear and see aliasing.
- **FLOW page:** a live routing graph with a mini-scope on every node.
- **Borrowed from superhet:** the 6 s peak-hold autorange (`superhet/js/core/probes.js:161-181`).

**Deploy (own container and hostname on the VM)**
- Files: `deploy/Containerfile` (nginx-unprivileged, serving `web/dist`), `nginx.conf`, `compose.yaml` and `deploy.sh`, modelled on `modulation_demo/deploy/`.
  - `deploy.sh` does a `buildx` build for linux/amd64. It then either streams the image over SSH or pushes it with `HUB=djbird` to `docker.io/djbird/birdsynth`.
  - `compose.yaml` defines the compose project `birdsynth` in `~/birdsynth/` on the VM, with ports `8001:8080` and a healthcheck.
- nginx settings:
  - HTTPS redirect based on `CF-Visitor` (as in `deploy/nginx.conf:12-14`)
  - `application/wasm` served with the right type and gzipped
  - `/assets/*` marked immutable for a year; `index.html` sent as `no-cache`
  - nosniff, a referrer policy, and `X-Robots-Tag: noindex` until the public launch
  - if a CSP is added, it needs `'wasm-unsafe-eval'`
- **Your step:** add the Cloudflare tunnel public hostname, for example `birdsynth.abusing.technology` → the VM on :8001.
- Deploys only work from the 192.168.2.x network.

### Repo layout
```
wavetable/
  Cargo.toml  rust-toolchain.toml (stable + wasm32-unknown-unknown)  .cargo/config.toml (+simd128)
  params/**/*.toml                     crates/{dsp,engine,tools,engine-wasm,tools-wasm,cli}/
  crates/engine/src/{engine,voice,alloc,cmd,telemetry,taps}.rs
    osc/{table,wavetable,unison,warp,sub,noise,sample,multisample,granular,spectral}.rs
    filter/{svf,ladder,diode,sallen_key,comb,phaser,formant,eq,misc}.rs
    modulation/{env,lfo,chaos,matrix,sources,curves,policy}.rs
    fx/{rack,hyper,distortion,flanger,phaser,chorus,delay,compressor,reverb,eq,filter,bode,convolve,utility,splitter}.rs
    seq/{transport,arp,clip,ledger}.rs
  web/  vite.config.ts (cargo plugin: rebuild wasm on .rs change → reload)
    src/audio/{host,protocol,telemetry,taps}.ts  src/audio/worklet/processor.ts
    src/tools/{worker,rpc}.ts  src/input/{midi,qwerty,mpe}.ts
    src/state/{params.gen,schema.gen,bank,patch,store.svelte,history,library}.ts
    src/ui/{primitives,graphs,panels,pages,editors,browser,gallery}/  src/explain/{overlay,flow,tours,content}/
    tests/ (vitest)  tests/engine/ (real .wasm in Node)  tests/worklet/ (processor in node:vm)
  tools/ (gen-schema.mjs, render.mjs: patch + MIDI → WAV)   deploy/   .claude/launch.json
```

---

## Reuse from modulation_demo (patterns, ported to TS/Rust)
- **Frame-stamped transferable tap pool:** `superhet/js/worklet/receiver.js:45-53,353-377` and `superhet/js/core/audio.js:37-39,88-163`.
- **Harness for running the processor in `node:vm` with fake worklet globals,** counting typed-array constructions: `superhet/tools/test_taps.mjs:21-30,230` and `tools/dsp-test.mjs:5-12`. Plus the Goertzel check at `tools/dsp-test.mjs:43-52`.
- **RIFF chunk walker** (the base for `clm `): `superhet/js/core/programs.js:297-317`.
- **Catmull-Rom read** (the HQ reference): `superhet/js/worklet/receiver.js:81-91`.
- **Canvas helpers:**
  - `fit()`, `trigger()` and `Waterfall` (`js/viz.js:24-36,80-88,289-330`)
  - the log-axis spectrum (`superhet/js/core/widgets.js:162-243`)
  - `arcD` (`superhet/js/core/svg.js:121-124`)
- **Deploy scaffolding:** `deploy/deploy.sh`, `compose.yaml` and `nginx.conf`, with every hard-coded "superhet" renamed. That includes the cleanup match at `deploy.sh:39`.
- **Pitfall to avoid:** allocating per sample, as `worklets/modulator.js:121` does.

---

## Phases
Each phase ends playable and committed, and must pass its gates.

### P0 — Toolchain and tracer bullet (freezes the ABI, schema and protocol)
1. **Toolchain:**
   - `brew uninstall rust` (optionally `brew autoremove`)
   - rustup official installer, minimal profile, `-t wasm32-unknown-unknown`
   - `brew install binaryen`
   - check that `which cargo` resolves to `~/.cargo/bin`
2. **Repo:** `git init`, the workspace, and Vite + Svelte 5 + TS with vitest.
   - Add `.claude/launch.json` (`birdsynth`: `npm run dev`), and switch this session's primary directory to `/Users/david.fogle/wavetable`.
3. **Engine:** a saw built additively per mip level, 8 voices, one ADSR, the full ABI, and the params/schema codegen with `wt_abi_hash`.
4. **Processor:** zero allocation, trap recovery, warm-up, taps block pool.
5. **Host side:** QWERTY and on-screen keyboard; one Knob with ARIA; a meter and a scope; `window.__synth`.
6. **Tests:** the Node engine tests and the `node:vm` processor harness.
7. **Unlisted staging deploy** to the VM on :8001. You add the hostname.

**Gates:**
- `WebAssembly.Module.imports(engine) = []`.
- A4 = 440.00 ± 0.01 Hz at 44.1 and 48 kHz.
- 0 allocator calls in `wt_render` over 60 s of random notes.
- 0 typed arrays created per `process()` after warm-up.
- Audio is back within 20 quanta of a forced trap.
- In the pane: 0 new underruns over 10 s of playing, and no console errors.
- The `vite preview` build plays.
- `curl -I` on staging shows `application/wasm`, immutable assets, a `no-cache` index, and the HTTPS redirect.

### P1 — Wavetable oscillator core (freezes the table format and the `OscSource`/`Filter` traits)
- **Tools worker pipeline:** mips → chunked upload → commit, including the partial-frame update.
- **Wavetable I/O:**
  - `clm ` import and export (including Surge's variant and chunkless files)
  - drag-and-drop import
  - about 20 generated factory tables
- **Osc A** (then B and C share the code):
  - OCT/SEM/FIN/CRS with Semitone/Harmonic/Ratio modes
  - phase, random phase and phase memory
  - unison 1–16, SIMD plus a scalar reference: detune modes, blend, width, range, stack, WT/warp spread
  - smooth frame interpolation
- **Phase warps:** Sync, Windowed Sync, Bend±, PWM, Asym±.
- **Filters:** SVF (LP/HP/BP/Notch 12/24) and a 4-pole ladder.
- **Voicing:** polyphony up to 32, mono/legato, glide (time, curve, always, scaled), steal priorities, and Env 1 → amp.
- **Minimal matrix engine:** envelope, velocity, note and mod wheel sources.
- **UI:** faceplate scaler, OSC page, 2D/3D wavetable view, keyboard strip, top bar.

**Gates:**
- Pitch within ±0.5 cent from C0 to C8 at 44.1, 48 and 96 kHz.
- Worst non-harmonic bin ≤ −60 dBc for a saw across a 20 Hz–12 kHz grid of fundamentals.
- Spectral centroid steps < 3% across a 2-octave glide.
- SIMD output equals scalar.
- SVF and ladder within ±0.3 dB of the analytic response.
- 60 s at maximum resonance: no NaN or Inf.
- `clm ` round-trip is bit-exact.
- 16 voices × 16 unison ≤ 6% of real time.
- Pane: moving `find("Osc A WT Position")` moves the 3D highlight.

### P2 — Full voice and modulation
- **Sound sources and routing:**
  - Osc B and C, Sub (shapes, octave, direct out), and Noise (our own generated library)
  - per-osc routing: F1↔F2 balance, Main/Direct/None, Bus 1/2 sends
  - Filter 2 (series or parallel)
- **Cross-modulation and quality:**
  - fused FM/PD/AM/RM from any osc, sub, noise or filter, plus PD Self
  - the oversampled voice path (Quality setting)
- **Envelopes:** 4 (AHDSR, curves, BPM mode, legato invert, retrigger).
- **LFOs:** 10.
  - Types: Normal (drawable), Path XY, Chaos Lorenz/Rössler, S&H.
  - Trigger modes: Free, Retrig, Env.
  - Rate controls: BPM, dotted/triplet, anchor, ×10.
  - Shaping: rise, delay, smooth, phase, direction, and poly/global.
- **Macros and sources:**
  - 8 macros
  - all sources: poly AT, release velocity, NoteOn randoms, voice index, Voice Mod 1/2, active voices, audio-rate osc/filter sources
- **Matrix:**
  - 64 slots with curves, aux, bypass and reorder
  - the policy table
  - global rate scaling
- **Drag-to-modulate** with rings and live dots.
- **Pages:** MIX and MATRIX.
- The explainer's `data-explain` keys are added from here onward.

**Gates:**
- Envelope timing ±1 ms.
- LFO period ±0.1%, with BPM sync phase-locked over 60 s.
- Matrix matches a TS reference model over 1,000 random routings.
- The fused FM loop, including A↔B cycles, is bit-exact against the scalar reference.
- The policy-table tests pass.
- Pane: dragging LFO 1 onto cutoff produces a matrix row, a ring and moving telemetry.
- The heavy patch without FX is ≤ 20% of real time, with 0 underruns over 30 s.

### P3 — FX racks and the rest of the filter and warp types
- **Racks:** Main, Bus 1 and Bus 2, with unlimited instances, reorder, bypass, rack presets and module presets.
- **Effects:**
  - Distortion (modes plus Overdrive, DC bias and key track, at 4× oversampling)
  - Flanger, Phaser, Chorus
  - Delay (Normal, Ping-Pong, Tap→Delay, HQ)
  - Compressor (single-band and 3-band upward/downward)
  - Reverb: 5 algorithms of our own
  - EQ, Filter, Hyper/Dimension, Bode
  - Convolve: uniformly partitioned convolution, zero latency, generated IRs plus user IRs
  - Utility
  - Splitters: L/H, L/M/H, M/S
- **Types:** the remaining ~80 filter types on the shared cores, and every remaining warp (distortion, filter, Even/Odd, Flip, Mirror, Remap 1–4, Quantize).
- **FX page** and the expanded views.

**Gates:**
- Delay time ±1 sample.
- RT60 within ±10%.
- EQ within ±0.3 dB of analytic.
- Compressor static curve within ±0.5 dB.
- Crossovers sum flat within ±0.1 dB.
- Bode shift within ±0.1 Hz.
- Convolve matches direct convolution to < −100 dB.
- Distortion aliasing ≤ −70 dBc.
- No subnormal state after an impulse followed by 60 s of silence.
- Reorder and bypass are click-free (bounded sample delta).
- Every filter type is stable under audio-rate modulation.
- Every warp matches its reference preview.
- Heavy patch ≤ 30% of real time.

### P4 — Presets, MIDI, GLOBAL and explainer MVP → **first public deploy**
- **Patches and library:**
  - the patch format, migrations, and the IndexedDB library with persist
  - single-file export
  - about 30 factory presets
- **Browser:** search, tags with all/any/exclude, ratings, metadata.
- **Undo and redo.**
- **MIDI:** Web MIDI (Chrome and Firefox) with MIDI learn, and on-screen expression for Safari.
- **GLOBAL page:**
  - bend range and Quality
  - tuning (`.tun`, `.scl`/`.kbm`) and master tune
  - velocity curve and voice-steal settings
- **Explainer MVP:** explain mode with live taps, plus the aliasing tour.
- **Deploy:** remove `noindex`.

**Gates:**
- Save → reload → render is bit-identical, by hash.
- The migration fixtures pass.
- Every `data-explain` key resolves.
- The tour passes under pane automation, with screenshots.
- `__synth.midiIn` and MIDI learn work.
- The public smoke script passes (curl, plus pane load, play, and a clean console).

### P5 — Wavetable editor
- **Drawing:** draw tools, grid, and frame thumbnails you can reorder and multi-select.
- **FFT bins** for magnitude and phase, with their operations.
- **Formula language:** a Pratt parser compiled to a stack VM in `tools.wasm`, with no `eval`. Variables `x w y z q in sel rand`; errors are reported with positions.
- **Process menu.**
- **Morph modes:** crossfade, spectral, zero-fundamental-phase, zero-all-phases.
- **Import:**
  - dynamic pitch
  - constant frame size
  - FFT split
  - frequency estimation
- **Audition** through the partial-frame update.
- Every operation is undoable.

**Gates:**
- < 50 ms from a pen stroke to the audible change, measured at the tap.
- Golden outputs match for the formula set.
- Spectral morph magnitudes are within ±0.1 dB.
- Pitch-detect import is within ±1 cent.
- Frame counts come out as expected.

### P6 — Sample-based oscillator types (can run in parallel with P7)
- **Sample:**
  - modulatable start, end and loop points
  - loop modes: forward, ping-pong, reverse, tailed, one-shot
  - crossfade and snap
  - rate control (tape-stop)
  - transient slicing and tails
- **Multisample:** an SFZ subset, plus a factory set rendered from our own physical models.
- **Granular:** everything in the checklist, including the XY pad.
- **Spectral:** analysis, warps, drawable spectral filter, and PNG import.
- **Sample editor view.**

**Gates:**
- Root-note pitch ±0.5 cent.
- Loop crossfades are click-free.
- ≥95% of transients sliced within ±5 ms.
- SFZ fixtures map correctly.
- Grain statistics within ±5%, and the budget is never exceeded.
- Spectral time-stretch keeps pitch within ±1 cent.
- Spectral pitch-shift keeps duration within ±1%.
- Spectral output starts at the note-on frame (zero latency).
- Each type stays within its CPU cap.

### P7 — Arp, clip, keyboard and Voice Control
- **Transport:** BPM, play/stop, swing. Optional MIDI clock-in and MIDI out.
- **Arp:** all shapes, the step lanes, repeats, gate above 100%, chance, velocity ramp, and retrigger modes.
- **Clip:** piano roll, recording, automation lanes, launch quantize, 12 trigger keys, MIDI file import.
- **Keyboard:** transpose, key/scale, and the per-osc note/velocity mapping (Range/Fold/Warp).
- **Voice Control:** the 8-step per-note sequencer.

**Gates:**
- Arp onsets are frame-exact: 1/16 notes at 120 BPM land every 6,000 frames at 48 kHz.
- The shape-order tests pass.
- A 10k-operation stuck-note fuzz test ends with 0 stranded notes.
- Clip playback and the MIDI-file fixtures match.

### P8 — Completion
- **Performance:** MPE (X/Y/Z per note).
- **Explainer:** the FLOW page and the full tour set.
- **Browser extras:** Hybridize; previews that play a clip, with macros during preview; auto-play.
- **QA:** Firefox and Safari; keyboard-only navigation and accessibility; CPU-guard tuning.
- **Final deploy.**

**Gates:**
- Injected-MPE tests pass.
- Every tour passes under automation.
- Every FLOW node is live.
- Perf gates are re-measured and pass.

---

## Verification (how the gates are run)
- `cargo test` covers the DSP gates. `npm test` runs:
  - vitest (protocol, patch, WAV, formula, schema freshness)
  - engine tests that load the real release `engine.wasm` in Node through the same ABI the worklet uses: pitch by Goertzel or FFT, aliasing bins, NaN/subnormal scans, the allocation counter, real-time factor after warm-up
  - the `node:vm` processor harness: typed-array counter and trap recovery
- `npm run bench` gives the perf gates. It runs in Node; the OfflineAudioContext bench page is only a sanity check, since it reads about 30% pessimistic.
- **Browser pane:**
  - `preview_start {name:"birdsynth"}`
  - drive the synth with `__synth` (inject MIDI with `midiIn`)
  - `find` knobs by their ARIA label
  - read the console and `playbackStats` underruns
  - screenshot each page and the `/gallery`
- **Listening:** `node tools/render.mjs patch.json notes.mid out.wav` produces a file for you to hear.
- **Real MIDI** (optional, by you): Web MIDI with a real controller in your Chrome, since MIDI is denied in the pane.

## Working conventions
- **Git:** a local repo in `/Users/david.fogle/wavetable`, committed at the end of each phase. Nothing is pushed unless you ask.
- **Memory:** after P0, I save a project memory with the path, commands, hostname and gates, linked to [[deploy-vm]].
- **Parallel lanes:** after P1 freezes the traits and formats, the work splits cleanly:
  - FX modules
  - filter models
  - each oscillator type together with its tools-side builder
  - UI panels built against FakeEngine
  - explainer content
  - presets
  - arp/clip

  Each lane would get its own git worktree. The engine core, protocol and schema stay with a single owner. Fanning out to parallel subagents is faster but costs proportionally more tokens. I'll only do it if you say so, for example "use a workflow for P3"; otherwise I build sequentially.

---

## Appendix: Serum 2 parity checklist
This is based on Serum 2 v2.1.5: Xfer's What's New PDF, the product page, the web manual and the changelog, plus the Serum 1 manual for behaviour that carried over. Items marked ⚠ couldn't be confirmed from those sources. We use our own names wherever Serum's are its own coinages.

**Oscillators**
- 3 main oscillators (A/B/C), each with 5 types:
  - Wavetable (P1)
  - Sample, Multisample, Granular and Spectral (P6)
- Sub and Noise (P2).
- Pitch controls (P1):
  - OCT, SEM, FIN, CRS
  - OCT/SEM modes: Semitones, Harmonics, Ratio. Serum's "Step" mode needs MTS-ESP, which browsers can't use, so we map it to scale steps from a .tun or .scl file.
  - pitch tracking; bend tracking for the sample types
- Other per-osc controls (P1):
  - level, pan, phase, random phase, phase memory
  - copy/paste, with or without mods; lock; init
- Unison 1–16 (P1):
  - detune, blend, width, range 0–48 st
  - stack modes: 12, 24, +7, Center
  - detune modes: Linear, Super, Exp, Inv, Random
  - WT-position and warp spread; the "enhanced unison" options
- Wavetable (P1): 256 frames × 2048 samples, with smooth interpolation between frames.
- Sample:
  - start, end and loop points, all modulatable
  - loop modes: forward, ping-pong, reverse, tailed, one-shot
  - loop crossfade and snap
  - rate control (tape-stop)
  - slicing with transient detection, and tails
- Multisample: SFZ import (subset) and our own factory set, rendered from built-in physical models.
- Granular:
  - up to 256 grains
  - grain envelope and window, timbre shift, warp
  - density, length, position and scan, randomization
  - manual mode, loop grains, XY pad
- Spectral:
  - resynthesis that keeps transients, with time and pitch independent
  - low/high frequency limits and scan rate
  - points, loops and slices
  - a drawable spectral filter
  - spectral warps: Smear, Spread, Detune±, Bend±, Harmonics/Subharmonics, Comb, Pitch Shift, Pitch Blend
  - PNG image import
- Sub (P2): shapes, octave, level, direct out.
- Noise (P2):
  - our own generated noise library
  - one-shot or loop, phase, random phase, keytrack, pitch, pan, level

**Warp:** two slots per oscillator, on every oscillator type. The phase modes land in P1, cross-modulation in P2, and the rest in P3.
- Phase modes: Sync, Windowed Sync, Bend±, PWM, Asym±, Flip, Mirror, Remap 1–4 (drawable), Quantize.
- Cross-modulation: FM, PD, AM and RM from another osc, sub, noise, Filter 1 or Filter 2, plus PD Self.
- Distortion modes: Soft Clip, Hard Clip, Soft Sat, Tape Sat, Tube, Diode 1/2, Lin Fold, Sin Fold, Sine Shaper, Asym, Stomp, Zero-Square.
- Filter modes: LPF, HPF, and Even/Odd.
- Serum 2's internal preset IDs also show FMX/FMP variants, whose meaning is unknown ⚠.

**Wavetable editor (P5)**
- Draw tools with grid, and frame thumbnails you can reorder and multi-select.
- FFT bins for magnitude and phase, with clear HF/LF, randomize, octave shift, odd/even only.
- Formula parser with the variables `x w y z q in sel rand`.
- Process:
  - normalize each or same, DC removal, flips, fades, filter, sample-rate reduction
  - remove fundamental, blur spectra, create PWM
- Morph: Crossfade, Spectral, Spectral zero-fundamental-phase, Spectral zero-all-phases.
- Sort, add and remove frames.
- Import:
  - dynamic pitch (zero-snap or follow)
  - constant frame size (pitch average or a typed length)
  - FFT 256–2048
  - frequency estimation
- Export `.wav` with a `clm ` chunk (below).

**Filters and mixer (P1 cores, P2 routing, P3 full list)**
- Routing:
  - 2 filters, in series or parallel
  - per osc: F1↔F2 balance, Main/Direct/None, and Bus 1/2 sends
- Controls:
  - type, cutoff, resonance
  - drive, including a Clean mode
  - var/fat, stereo, mix, level, key track
  - drag cutoff and resonance on the response graph
- About 90 types, grouped Normal / Multi / Flanges / Misc / Ladders, built on about 12 cores:
  - SVF
  - 4-pole ladders in several characters (Moog-, 303-, EMS- and "dirty"-style)
  - diode ladder, Sallen-Key
  - comb/flange, allpass/phaser
  - formant/vowel, EQ and shelves
  - DJ mixer, diffusor, ring mod, S&H, a drawable pole-zero SVF
- The exact Serum filter list isn't confirmed ⚠.
- MIX page: a graphical mixer (P2).

**Modulation (P1 minimal, P2 full)**
- 4 envelopes (Env 1 = amp):
  - AHDSR with curves
  - BPM mode, legato invert, retrigger options
- 10 LFOs:
  - types: drawable Normal, Path (XY), Chaos Lorenz, Chaos Rössler, S&H
  - trigger modes: Free, Retrig, Env
  - rate: BPM, dotted/triplet, anchor, ×10 range up to 1 kHz
  - rise, delay, smooth, modulatable phase, direction, follow swing
  - XY grid, presets, mono
- 8 macros, which can also be destinations.
- Sources:
  - velocity and note, with curves
  - mod wheel, pitch bend, aftertouch, poly AT, release velocity
  - NoteOn random 1/2 and discrete
  - voice index, Voice Mod 1/2, active voices, fixed
  - any osc or filter at audio rate
  - MPE X/Y/Z (P8)
- Matrix:
  - 64 slots, each with source, amount, destination, uni/bipolar, aux source and output
  - source and aux curves, bypass, reorder, expanded view, live visuals
  - a Retrigger destination
- Drag-to-modulate, with mod rings on knobs.
- Voice Control: an 8-step per-note sequencer (P7).
- Global env/LFO rate scaling.

**FX (P3)**
- Racks: Main, Bus 1 and Bus 2.
  - Unlimited instances of each effect, reorderable, bypassable.
  - Rack presets and module presets; graphic editing.
- Effects:
  - Distortion, with Overdrive, DC bias and key track
  - Flanger, Phaser, Chorus
  - Delay: Normal, Ping-Pong, Tap→Delay, and HQ mode
  - Compressor, including 3-band upward/downward "OTT-style"
  - Reverb: 5 algorithms of our own, corresponding to Hall, Plate, Vintage, Nitrous and Basin
  - EQ, Filter, Hyper/Dimension
  - Bode frequency shifter
  - Convolve, with generated factory IRs and user IRs
  - Utility
- Splitters: L/H, L/M/H, M/S.

**Arp / Clip / Keyboard (P7)**
- Banks of 12 slots.
- Arp:
  - shapes: Up, Down, Up/Down variants, Converge, Diverge, Thumb, Played, Chord, Random, Pattern
  - step lanes: length, velocity, chance, bend, strum
  - transpose shift and range
  - rate with dotted/triplet, offset, repeats, gate above 100%, chance, velocity ramp
  - retrigger on beat, launch or note
- Clip piano roll:
  - grid, overdub and extend recording, automation lanes
  - loop and offset markers, launch quantize, retrigger
  - gate, velocity trigger, static mode, transpose and offset
  - 12 trigger keys, per-note chance and bend
  - MIDI file import
- Keyboard:
  - transpose, key and scale, swing
  - per-osc note/velocity ranges (Range, Fold, Warp)
- Internal transport, standing in for DAW sync.
- Optional Web MIDI clock-in and MIDI out.

**Voicing / global (P1, P4)**
- Polyphony up to 32, default 8 ⚠.
- Mono and legato.
- Portamento: time, curve, always, scaled.
- Voice-steal priority: Newest, Oldest, Lowest, Highest, Velocity.
- Pitch-bend range.
- Quality / oversampling (applies to warps).
- MPE.
- Tuning: `.tun`, plus `.scl`/`.kbm`, in place of MTS-ESP.
- MIDI learn, standing in for DAW automation.
- Master tune.

**Browser and presets (P4, extras in P8)**
- A library tree (factory and user), search, tags (all/any, with exclude), ratings, metadata editing.
- Previews that play a clip, with macros usable during preview.
- Hybridize, and auto-play.
- Presets embed their tables and samples.

**UI (throughout)**
- Pages: OSC, MIX, FX, MATRIX, GLOBAL, plus the ARP/CLIP editors and our own EXPLAIN and FLOW.
- An always-visible lower strip: Env 1–4, LFOs, macros, vel/note, wheels, ARP, CLIP, keyboard, voicing.
- A top bar: browser, menu, undo/redo, volume, CPU.
- 2D/3D wavetable view, drag-and-drop modulation and rings.
- Expanded FX and Matrix views, scale-to-fit, tooltips.

**Wavetable WAV format**
- Mono 32-bit float, frames of 2048 samples.
- Reading: parse the `clm ` chunk. Offsets 3–6 hold the frame size, and flag digit 1 holds the interpolation (0 none, 1 crossfade, 2–4 spectral). Also accept Surge's variant. Files without the chunk go through the import dialog.
- Writing: `<!>2048 {interp}0000000 wavetable (<our name>)`. Flag digit 2, Xfer's factory flag, is left at 0.

**Not planned**
- Loading `.SerumPreset` files: a proprietary format known only by reverse-engineering.
- Serum's factory content.
- Plugin builds.
- MTS-ESP, which is native-only.
- The exact sound of Serum's own filters and effects. Ours are our own implementations of the same categories.
