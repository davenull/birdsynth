// window.__synth: a small surface for driving and inspecting the synth from
// tests and the browser console, e.g.
//   __synth.noteOn(60); __synth.telemetry(); __synth.midiIn([0x90, 64, 100])

import { PARAMS, PARAM_ID, type ParamKey } from './gen/params';
import { toNorm, toPlain } from './state/param-math';
import { SOURCES, TAP, TEL, TEL_COUNT, type TapName } from './gen/protocol';
import { parseMidi } from './input/midi';
import { exportFile, importFile } from './state/patch';
import { pictureToAnalysis } from './state/spectral';
import { explain } from './explain/content';
import { explainMode } from './explain/explain.svelte';
import { inharmonicDbc } from './explain/measure';
import { tours } from './explain/tour.svelte';
import { TOURS } from './explain/tours';
import { checkStep } from './explain/check';
import { nav, type PageId } from './ui/nav.svelte';
import { editorView } from './editor/editor.svelte';
import type { Scope } from './editor/model';
import type { Synth } from './synth';
import type { ClipNote, LaneName } from './state/seq';
import { parseSmf } from './midi/smf';

interface PlaybackStats {
  underrunEvents?: number;
  underrunDuration?: number;
}

export function installTestApi(synth: Synth): void {
  const kept = new Set<TapName>();
  const api = {
    synth,
    start: () => synth.start(),
    status: () => synth.status,
    error: () => synth.error,
    noteOn: (note: number, velocity = 0.8, channel = 0) => synth.noteOn(note, velocity, channel),
    noteOff: (note: number, channel = 0) => synth.noteOff(note, channel),
    allNotesOff: () => synth.allNotesOff(),
    setParam: (key: ParamKey, norm: number) => synth.setParam(key, norm),
    getParam: (key: ParamKey) => synth.getParam(key),
    /** Set a parameter in its own unit: __synth.setPlain('filter.1.cutoff', 800) */
    setPlain: (key: ParamKey, plain: number) => synth.setParam(key, toNorm(PARAMS[PARAM_ID[key]], plain)),
    getPlain: (key: ParamKey) => toPlain(PARAMS[PARAM_ID[key]], synth.getParam(key)),
    params: () => PARAMS.map((p) => p.key),
    /** Feed raw MIDI bytes through the same parser Web MIDI uses. */
    midiIn: (bytes: number[]) => parseMidi(bytes, synth),
    telemetry: () => synth.telemetry(),
    /** Every telemetry slot by name (arrays for multi-slot entries). */
    telemetryAll: () => {
      const t = synth.host?.tel;
      const out: Record<string, number | number[]> = {};
      if (!t) return out;
      for (const [name, at] of Object.entries(TEL)) {
        if (name === 'LEN') continue;
        const n = TEL_COUNT[name as keyof typeof TEL_COUNT];
        out[name] = n > 1 ? Array.from(t.subarray(at, at + n)) : t[at];
      }
      return out;
    },
    sources: () => [...SOURCES],
    /** Route a matrix slot: __synth.mod(0, 'Env 2', 'filter.1.cutoff', 0.5) */
    mod: (slot: number, source: string, dest: ParamKey, amount: number, bipolar = false, curve = 0, output = 1, aux = 'None') =>
      synth.setModSlot(
        slot,
        SOURCES.indexOf(source as (typeof SOURCES)[number]),
        PARAMS.find((p) => p.key === dest)!.id,
        amount,
        bipolar ? 1 : 0,
        SOURCES.indexOf(aux as (typeof SOURCES)[number]),
        curve,
        output,
      ),
    /** The routed slots, readable: [{ slot, source, dest, amount, ... }]. */
    matrix: () =>
      synth.matrix.slots.flatMap((s, slot) =>
        s ? [{ slot, source: SOURCES[s.source], aux: SOURCES[s.aux], dest: PARAMS[s.dest].key, amount: s.amount, curve: s.curve, output: s.output, bipolar: s.bipolar, bypass: s.bypass }] : [],
      ),
    /** Keep a tap recording (scopes do this themselves while shown). */
    useTap: (name: TapName) => synth.useTap(name),
    loadTable: (osc: number, name: string) => synth.tables.loadFactory(osc, name),
    /** The newest frames of a tap. A tap read here stays recorded from then on. */
    tap: (name: TapName, frames = 1024) => {
      if (!kept.has(name)) {
        kept.add(name);
        synth.useTap(name);
      }
      return Array.from(synth.tap(name, frames));
    },
    stats: () => {
      const ctx = synth.host?.ctx;
      const ps = (ctx as unknown as { playbackStats?: PlaybackStats } | undefined)?.playbackStats;
      return {
        state: ctx?.state ?? 'none',
        sampleRate: ctx?.sampleRate ?? 0,
        baseLatency: ctx?.baseLatency ?? 0,
        outputLatency: ctx?.outputLatency ?? 0,
        underrunEvents: ps?.underrunEvents ?? null,
        underrunDuration: ps?.underrunDuration ?? null,
        cpuPct: synth.host?.cpuPct ?? 0,
        dropped: synth.host?.dropped ?? 0,
        traps: synth.host?.traps ?? 0,
      };
    },
    debugTrap: () => synth.debugTrap(),
    // ------------------------------------------------------------ presets
    /** The current state as a single-file patch (JSON text, assets embedded). */
    savePatch: async () => {
      const { patch, assets } = await synth.savePatch();
      return exportFile(patch, assets);
    },
    /** Load a single-file patch; returns what couldn't be restored. */
    loadPatch: async (text: string) => {
      const { patch, assets } = importFile(text);
      return synth.loadPatch(patch, async (h) => assets.get(h));
    },
    /** Library preset names ("factory:Reese", "user:…"). */
    presets: async () => (await synth.openLibrary()).entries.map((e) => ({ id: e.id, name: e.patch.meta.name, category: e.patch.meta.category })),
    loadPreset: (idOrName: string) => synth.openLibrary().then((lib) => synth.loadEntry(lib.entry(idOrName)?.id ?? lib.entries.find((e) => e.patch.meta.name === idOrName)?.id ?? idOrName)),
    preset: () => ({ id: synth.presetId, meta: synth.meta, dirty: synth.history.dirty }),
    undo: () => synth.history.undo(),
    redo: () => synth.history.redo(),
    /** End the current gesture now, so it's an undo step without waiting. */
    commit: () => synth.history.commit(),
    // --------------------------------------------------------------- MIDI
    /** Arm MIDI learn for a parameter; the next CC through midiIn (or Web MIDI) is tied to it. */
    learn: (key: ParamKey) => synth.learn.arm(key),
    learned: () => synth.learn.maps.map((m) => ({ ...m })),
    forget: (key: ParamKey) => synth.learn.clear(key),
    midi: () => ({ status: synth.midi.status, inputs: [...synth.midi.inputs] }),
    wheels: () => ({ ...synth.wheels }),
    /** Turn to a page: 'osc' | 'mix' | 'fx' | 'matrix' | 'global'. */
    page: (id?: PageId) => {
      if (id) nav.page = id;
      return nav.page;
    },
    // ------------------------------------------------------------ explainer
    explain: {
      /** Every data-explain key on screen now, and whether it has notes. */
      keys: () => [...new Set([...document.querySelectorAll('[data-explain]')].map((e) => e.getAttribute('data-explain')!))].map((key) => ({ key, ok: explain(key) !== null })),
      missing: () => [...document.querySelectorAll('[data-explain]')].map((e) => e.getAttribute('data-explain')!).filter((k) => explain(k) === null),
      text: (key: string) => explain(key),
      mode: (on?: boolean) => {
        if (on !== undefined && on !== explainMode.on) explainMode.toggle();
        return explainMode.on;
      },
      open: (key: string) => {
        const el = document.querySelector(`[data-explain="${key}"]`);
        if (!el) return false;
        if (!explainMode.on) explainMode.toggle();
        explainMode.show(key, el);
        return true;
      },
      /** The strongest partial that isn't a harmonic of the sounding note, in dB below the strongest harmonic. */
      inharmonic: () => {
        const h = synth.host;
        if (!h) return null;
        if (!kept.has('master.l')) {
          kept.add('master.l');
          synth.useTap('master.l');
        }
        const x = new Float32Array(8192);
        const heard = Math.min(h.heardFrame(), h.taps.latest(0));
        if (!h.taps.read(0, heard, x)) return null;
        const note = h.tel[TEL.focusPitch];
        return inharmonicDbc(x, h.ctx.sampleRate, 440 * 2 ** ((note - 69) / 12));
      },
    },
    // ------------------------------------------------------------- editor
    editor: {
      open: (osc = 0) => editorView.open(synth.tables, osc),
      close: () => editorView.close(),
      state: () => {
        const e = editorView.editor(synth.tables);
        return { open: editorView.osc, name: e.name, count: e.count, current: e.current, selected: [...e.selected].sort((a, b) => a - b), canUndo: e.canUndo, canRedo: e.canRedo };
      },
      /** A frame's samples (every `step`-th). */
      frame: (i: number, step = 1) =>
        Array.from(
          editorView
            .editor(synth.tables)
            .frame(i)
            .filter((_, k) => k % step === 0),
        ),
      select: (i: number, mode: 'only' | 'toggle' | 'range' = 'only') => editorView.editor(synth.tables).select(i, mode),
      /** Draw a stroke through [sample index, value] points on the current frame (one undo step). */
      draw: (points: [number, number][]) => {
        const e = editorView.editor(synth.tables);
        e.beginStroke();
        for (let k = 1; k < points.length; k++) e.segment(points[k - 1][0], points[k - 1][1], points[k][0], points[k][1]);
        e.endStroke();
      },
      formula: (src: string, scope: Scope = 'all') => editorView.editor(synth.tables).formula(src, scope),
      process: (kind: number, scope: Scope = 'all', a = 0, b = 0) => editorView.editor(synth.tables).process(kind, scope, a, b),
      morph: (mode: number, target: number) => editorView.editor(synth.tables).morph(mode, target),
      pwm: (n: number) => editorView.editor(synth.tables).pwm(n),
      undo: () => editorView.editor(synth.tables).undo(),
      redo: () => editorView.editor(synth.tables).redo(),
      settled: () => editorView.editor(synth.tables).settled(),
      /** Import a generated tone: `hz` for `secs`, a saw plus noise if asked. */
      importTone: (hz: number, secs: number, mode: number, arg?: number) => {
        const sr = 48_000;
        const audio = Float32Array.from({ length: Math.round(sr * secs) }, (_, i) => {
          let v = 0;
          for (let h = 1; h * hz < sr / 2 && h <= 40; h++) v += Math.sin((2 * Math.PI * hz * h * i) / sr) / h;
          return v * 0.5;
        });
        return editorView.editor(synth.tables).importAudio(audio, sr, mode, arg ?? sr / hz);
      },
      /**
       * Pen stroke to sound: hold a note on a rising ramp, then flip it to a
       * falling one as a stroke, and time how long until Osc A's tap shows
       * it. Returns ms to the rendered change, and that plus the output
       * latency (what you'd hear). The frame is put back afterwards.
       */
      latency: async () => {
        const e = editorView.editor(synth.tables);
        const h = synth.host;
        if (!h) return null;
        const tap = TAP['focus.osc.a'];
        synth.useTap('focus.osc.a');
        const ramp = Float32Array.from({ length: 2048 }, (_, i) => (2 * i) / 2048 - 1);
        e.beginStroke();
        e.writeFrame(ramp);
        e.endStroke();
        await e.settled();
        synth.noteOn(57, 0.8);
        await new Promise((r) => setTimeout(r, 400));
        const buf = new Float32Array(512);
        /** Which way the wave mostly moves: + rising, - falling. */
        const slope = (end: number) => {
          if (!h.taps.read(tap, end, buf)) return 0;
          let d = 0;
          for (let i = 1; i < buf.length; i++) d += Math.sign(buf[i] - buf[i - 1]);
          return d;
        };
        const before = h.taps.latest(tap);
        const t0 = performance.now();
        // the frame the context is rendering as the stroke lands
        const at = Math.round(h.ctx.currentTime * h.ctx.sampleRate);
        e.beginStroke();
        e.writeFrame(ramp.map((v) => -v));
        e.endStroke();
        let ms = -1;
        let end = before;
        for (let tries = 0; tries < 500 && ms < 0; tries++) {
          await new Promise((r) => setTimeout(r, 1));
          end = h.taps.latest(tap);
          if (end > before && slope(end) < -100) ms = performance.now() - t0;
        }
        synth.noteOff(57);
        e.undo();
        e.undo();
        if (ms < 0) return null;
        // where in the audio the falling ramp starts: the first 128-frame window that falls
        let change = end;
        const w = new Float32Array(128);
        for (let f = before - 128; f < end; f += 32) {
          if (!h.taps.read(tap, f + 128, w)) continue;
          let d = 0;
          for (let i = 1; i < w.length; i++) d += Math.sign(w[i] - w[i - 1]);
          if (d < -64) {
            change = f;
            break;
          }
        }
        const out = (h.ctx.outputLatency || 0) * 1000;
        // (the page's currentTime trails the render by up to a quantum, so this can read a hair early)
        const toSound = Math.max(0, ((change - at) / h.ctx.sampleRate) * 1000);
        return {
          /** until the tap block showing it arrived (taps come 1024 frames at a time) */
          detected: Math.round(ms * 10) / 10,
          /** until the engine played the new frame, from the stroke */
          rendered: Math.round(toSound * 10) / 10,
          /** and until it leaves the speakers */
          heard: Math.round((toSound + out) * 10) / 10,
        };
      },
    },
    // --------------------------------------------------------- recordings
    recording: {
      /** Put a generated recording on an oscillator: a saw at `hz` (harmonics to 40) for `secs`, at `rate`. */
      tone: (osc: number, hz = 220, secs = 2, rate = 48_000, stereo = false) => {
        const n = Math.round(secs * rate);
        const make = (detune: number) =>
          Float32Array.from({ length: n }, (_, i) => {
            let v = 0;
            for (let h = 1; h <= 40 && h * hz < rate / 2; h++) v += Math.sin((2 * Math.PI * hz * detune * h * i) / rate) / h;
            return v * 0.4;
          });
        const channels = stereo ? [make(1), make(1.003)] : [make(1)];
        return synth.recordings.set(osc, { name: `tone ${hz} Hz`, rate, channels, slices: [] });
      },
      /** A drum-like loop: hits at the given times (seconds), for slicing. */
      hits: (osc: number, times: number[], secs = 2, rate = 48_000) => {
        const x = new Float32Array(Math.round(secs * rate));
        let seed = 7;
        for (const t of times) {
          const a = Math.round(t * rate);
          for (let i = 0; i < rate * 0.15 && a + i < x.length; i++) {
            seed = (seed * 1664525 + 1013904223) >>> 0;
            x[a + i] += ((seed / 4294967296) * 2 - 1) * Math.exp(-i / (rate * 0.03)) * 0.6;
          }
        }
        return synth.recordings.set(osc, { name: 'hits', rate, channels: [x], slices: [] });
      },
      state: (osc: number) => {
        const r = synth.recordings.osc[osc];
        return r ? { name: r.name, rate: r.rate, channels: r.channels.length, frames: r.channels[0].length, slices: [...r.slices] } : null;
      },
      spectrum: (osc: number) => synth.recordings.spectrum(osc)?.frames ?? 0,
      picture: async (osc: number, w = 64, h = 64) => {
        // a rising line: a sweep
        const data = new Uint8ClampedArray(w * h * 4);
        for (let x = 0; x < w; x++) {
          const y = Math.round((1 - x / w) * (h - 1));
          const i = (y * w + x) * 4;
          data[i] = data[i + 1] = data[i + 2] = 255;
          data[i + 3] = 255;
        }
        const c = new OffscreenCanvas(w, h);
        c.getContext('2d')!.putImageData(new ImageData(data, w, h), 0, 0);
        const blob = await c.convertToBlob({ type: 'image/png' });
        const bytes = await blob.arrayBuffer();
        return synth.recordings.setPicture(osc, { name: 'sweep', bytes, seconds: 2 }, pictureToAnalysis({ width: w, height: h, data }, 2, synth.host?.ctx.sampleRate ?? 48_000));
      },
    },
    multi: {
      list: () => synth.multis.factoryList(),
      factory: (osc: number, name: string) => synth.multis.loadFactory(osc, name),
      state: (osc: number) => {
        const m = synth.multis.osc[osc];
        return m ? { name: m.name, zones: m.zones.length, source: m.source } : null;
      },
    },
    /** What the engine reports about each oscillator's assets and grains. */
    oscAssets: () => {
      const t = synth.host?.tel;
      if (!t) return null;
      return {
        play: [0, 1, 2].map((o) => t[TEL.oscPlay + o]),
        recFrames: [0, 1, 2].map((o) => t[TEL.oscAssets + o * 3]),
        zones: [0, 1, 2].map((o) => t[TEL.oscAssets + o * 3 + 1]),
        specFrames: [0, 1, 2].map((o) => t[TEL.oscAssets + o * 3 + 2]),
        grains: t[TEL.grains],
        stolen: t[TEL.grainsStolen],
      };
    },
    /** Linked instances (tabs of this browser): the shared timeline and who's in the group. */
    link: {
      state: () => ({ on: synth.link.on, linked: synth.link.linked, id: synth.link.id, name: synth.link.name, keeping: synth.link.keeping, members: synth.link.members.map((m) => ({ ...m })), timeline: synth.link.timeline && { ...synth.link.timeline }, anchor: synth.link.lastAnchor && { ...synth.link.lastAnchor } }),
      on: (on = true) => synth.link.setOn(on),
      name: (name: string) => synth.link.setName(name),
      nudge: (ms: number) => synth.link.setNudge(ms),
      corrections: () => ({ count: synth.link.corrections, lastMs: synth.link.lastCorrectionMs }),
    },
    /** The working state kept between visits. */
    session: {
      state: () => ({ state: synth.sessionState, saved: synth.sessionSaved, preset: synth.presetId, dirty: synth.history.dirty }),
      save: () => synth.saveSession(),
      reset: () => synth.resetSession(),
    },
    /** The transport, the arpeggiator's patterns and the clips. */
    seq: {
      play: (on = true) => synth.transport(on),
      state: () => {
        const t = synth.host?.tel;
        return t
          ? { playing: t[TEL.playing] >= 0.5, beat: t[TEL.beat], arpStep: t[TEL.arpStep], clipPlaying: t[TEL.clipPlaying], clipPos: t[TEL.clipPos], notes: t[TEL.seqNotes], beatNow: synth.beatNow() }
          : null;
      },
      arpStep: (bank: number, lane: LaneName, step: number, v: number) => synth.arp.set(bank, lane, step, v),
      clip: (slot: number) => structuredClone(synth.clips.clips[slot]),
      setClip: (slot: number, notes: Partial<ClipNote>[], length?: number) =>
        synth.clips.edit(slot, (c) => {
          c.notes = notes.map((n) => ({ start: 0, length: 0.25, key: 60, velocity: 0.8, chance: 1, bend: 0, ...n }));
          if (length) c.length = length;
        }),
      setLane: (slot: number, lane: number, param: ParamKey | null, points: [number, number][]) =>
        synth.clips.edit(slot, (c) => {
          c.lanes[lane].param = param;
          c.lanes[lane].points = points.map((p) => [p[0], p[1]]);
        }),
      /** Put a MIDI file (its bytes) in a clip, as Import does. */
      importMidi: (slot: number, bytes: number[]) => {
        const smf = parseSmf(new Uint8Array(bytes).buffer);
        synth.clips.importSmf(slot, smf);
        return smf.notes.length;
      },
      record: (slot: number) => synth.record(slot),
      stopRecording: () => synth.stopRecording(),
    },
    tour: {
      list: () => TOURS.map((t) => ({ id: t.id, title: t.title, steps: t.steps.length })),
      /** Measure what the current step expects (see explain/check.ts). */
      check: async () => {
        const s = tours.step;
        return s?.expect ? checkStep(synth, s.expect) : { ok: true, checks: [] };
      },
      /** Run tours start to finish, checking every step after it settles; a report per step. */
      runAll: async (ids?: string[], settle = 1200) => {
        const report: { tour: string; step: number; title: string; ok: boolean; checks: unknown[]; shown: boolean }[] = [];
        for (const t of TOURS.filter((t) => !ids || ids.includes(t.id))) {
          await tours.start(synth, t.id);
          for (let i = 0; i < t.steps.length; i++) {
            await tours.settled();
            await new Promise((r) => setTimeout(r, settle));
            const st = tours.step!;
            const shown = !st.target || !!document.querySelector(`[data-explain="${st.target}"]`);
            const r = st.expect ? await checkStep(synth, st.expect) : { ok: true, checks: [] };
            report.push({ tour: t.id, step: i + 1, title: st.title, ok: r.ok && shown, checks: r.checks, shown });
            await tours.next();
          }
          await tours.settled();
        }
        return { ok: report.every((r) => r.ok), failed: report.filter((r) => !r.ok), steps: report.length };
      },
      start: (id: string) => tours.start(synth, id),
      next: () => tours.next(),
      back: () => tours.back(),
      exit: () => tours.exit(),
      settled: () => tours.settled(),
      state: () => {
        const s = tours.step;
        const el = s?.target ? document.querySelector(`[data-explain="${s.target}"]`) : null;
        return tours.tour
          ? {
              id: tours.tour.id,
              index: tours.index,
              steps: tours.tour.steps.length,
              title: s?.title ?? '',
              target: s?.target ?? null,
              targetShown: !!el,
              page: nav.page,
              bandlimit: synth.bandlimit,
              expect: s?.expect ? { ...s.expect } : null,
            }
          : null;
      },
    },
  };
  (window as unknown as { __synth: typeof api }).__synth = api;
}
