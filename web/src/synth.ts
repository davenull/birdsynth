// The synth as the UI sees it: the parameter bank, held notes and the
// engine host, behind one object. Inputs (QWERTY, the on-screen keyboard,
// MIDI) and the test API all play through here.

import { PARAMS, PARAM_ID, type ParamKey } from './gen/params';
import { DEBUG, TEL, TEL_COUNT, TAP, type CmdWriter, type TapName } from './gen/protocol';
import { EngineHost } from './audio/host';
import { BLOCK_FRAMES, BLOCK_MAX_TAPS } from './audio/block';
import { toNorm, toPlain } from './state/param-math';
import { ParamBank } from './state/bank';
import { TableStore } from './state/tables';
import { LfoShapes } from './state/lfo';
import { ModMatrix, slotFlags, type ModSlot } from './state/matrix';
import { NoiseStore } from './state/noise';
import { RemapCurves } from './state/remap';
import { FxRacks, CONVOLVE } from './state/fx';
import { IrStore } from './state/ir';
import { TuningStore } from './state/tuning';
import { RecordingStore } from './state/recordings';
import { MultiStore } from './state/multis';
import { SpectralFilters, pictureToAnalysis } from './state/spectral';
import { zoneInfo } from './state/multis';
import { ArpPatterns, ClipStore } from './state/seq';
import { History } from './state/history';
import { Library, type Entry, type Session } from './state/library';
import { applyPatch, capture, emptyMeta, hashBytes, migrate, type Patch, type PatchMeta, type PatchTarget } from './state/patch';
import { parseMidi, type MidiSink } from './input/midi';
import { MidiLearn } from './input/learn';
import { WebMidi } from './input/webmidi';
import { MidiClock } from './input/clock';
import { MpeChannels, MPE_X, MPE_Y, MPE_Y_CC, MPE_Z } from './input/mpe';
import { CpuGuard } from './audio/guard';
import { phraseFor } from './state/preview';
import { hybridize, type HybridOptions } from './state/hybrid';
import { Link } from './sync/link';

export type SynthStatus = 'idle' | 'starting' | 'running' | 'suspended' | 'error';

interface Held {
  note: number;
  channel: number;
  velocity: number;
}

export interface Telemetry {
  voicesActive: number;
  peakL: number;
  peakR: number;
  focusVoice: number;
  queueDrops: number;
  unknownCmds: number;
  /** MIDI notes of the sounding voices. */
  notes: number[];
  cpuPct: number;
  traps: number;
  dropped: number;
  blocks: number;
  frame: number;
}

export class Synth implements MidiSink {
  readonly bank = new ParamBank();
  readonly tables = new TableStore();
  readonly matrix = new ModMatrix();
  readonly lfo = new LfoShapes();
  readonly noise = new NoiseStore(this.bank);
  readonly remap = new RemapCurves();
  readonly fx = new FxRacks(this.bank);
  readonly ir = new IrStore(this.bank);
  readonly tuning = new TuningStore();
  readonly recordings = new RecordingStore(this.bank);
  readonly multis = new MultiStore();
  readonly specFilters = new SpectralFilters();
  readonly arp = new ArpPatterns();
  readonly clips = new ClipStore();
  /** Recording into a clip: its slot and where each held key started (in beats). */
  recording: { slot: number; open: Map<string, { start: number; beat: number; velocity: number }> } | null = null;
  private transportSubs = new Set<() => void>();
  readonly target: PatchTarget;
  readonly history: History;
  /** The current preset: its metadata and where it came from ("" for none). */
  meta: PatchMeta = emptyMeta('Init');
  presetId = 'factory:Init';
  library: Library | null = null;
  readonly learn = new MidiLearn(this.bank);
  readonly midi = new WebMidi((bytes, time) => parseMidi(bytes, this, time));
  private readonly clock = new MidiClock();
  /** Per-note expression from MPE controllers (on with the voice.mpe parameter). */
  readonly mpe = new MpeChannels();
  /** The working state kept between visits (see restoreSession): when it was last saved, and whether it's being kept. */
  sessionSaved = 0;
  sessionState: 'starting' | 'on' | 'memory' | 'failed' = 'starting';
  private restoring: Promise<void> | null = null;
  private sessionTimer: ReturnType<typeof setTimeout> | null = null;
  private sessionSaving: Promise<void> | null = null;
  private sessionAgain = false;
  private sessionHolds = 0;
  private persistAsked = false;
  private readonly sessionSubs = new Set<() => void>();
  /**
   * The patch reference each file-born asset (a wavetable, recording, IR or
   * multisample) had when last collected, so the quick save on leaving the
   * page can name them without hashing. Any change to one drops it (and
   * bumps the generation, so a collect already under way doesn't store a
   * stale one).
   */
  private assetRefs = new WeakMap<object, unknown>();
  private assetGen = 0;
  /** Linked with other instances: a shared transport and tempo (see sync/). */
  readonly link: Link;
  /** The frame a parameter change lands on in the engine (0: as soon as it can); see setParamAt. */
  private paramFrame = 0;
  /** Eases the engine's load when it runs too high (a setting of this browser; on unless switched off). */
  readonly guard = new CpuGuard();
  guardOn = readSetting('birdsynth.cpu-guard') !== '0';
  private guardTimer: ReturnType<typeof setInterval> | null = null;
  /** Underruns counted so far (null until the first reading), and readings taken. */
  private underruns: number | null = null;
  private loadChecks = 0;
  /** Whether incoming MIDI clock sets the tempo and runs the transport (a setting of this browser, not the patch). */
  followClock = readSetting('birdsynth.midi-clock') === '1';
  /** Whether the oscillators band-limit (always, except in the aliasing tour). */
  bandlimit = true;
  /** The wheels as last moved (by MIDI or on screen): bend -1..1, mod 0..1. */
  readonly wheels = { bend: 0, mod: 0 };
  host: EngineHost | null = null;
  status: SynthStatus = 'idle';
  error = '';

  private readonly statusSubs = new Set<(s: SynthStatus) => void>();
  private starting: Promise<void> | null = null;
  private nextId = 1;
  /** noteId -> the note it plays; resent to a restarted engine. */
  private readonly held = new Map<number, Held>();
  /** "channel:note" -> noteIds, newest last (a key can be held from two inputs). */
  private readonly byKey = new Map<string, number[]>();
  /** Taps something on screen is showing, with a count of users each. A block carries at most BLOCK_MAX_TAPS. */
  private readonly tapUse = new Map<TapName, number>();
  private readonly patchSubs = new Set<() => void>();
  private readonly wheelSubs = new Set<() => void>();
  private opening: Promise<Library> | null = null;

  constructor() {
    this.target = { bank: this.bank, matrix: this.matrix, lfo: this.lfo, remap: this.remap, fx: this.fx, arp: this.arp, clips: this.clips };
    this.history = new History(this.target);
    this.bank.onAny((id, v) => {
      const frame = this.paramFrame;
      this.host?.send((w) => w.setParam(frame, id, v));
    });
    this.link = new Link(this);
    const mpeOn = () => {
      const on = this.bank.get(PARAM_ID['voice.mpe']) >= 0.5;
      if (on !== this.mpe.on) this.mpe.reset();
      this.mpe.on = on;
    };
    mpeOn();
    this.bank.subscribe(PARAM_ID['voice.mpe'], mpeOn);
    // anything that changes the sound (or which preset it is) is kept for next time, once it settles
    const keep = () => this.touchSession();
    this.bank.onAny(keep);
    for (const store of [this.matrix, this.lfo, this.remap, this.fx, this.arp, this.clips, this.recordings, this.multis, this.specFilters, this.ir, this.history]) store.subscribe(keep);
    this.tables.onChange(keep);
    this.onPatch(keep);
    // an asset that changes loses its remembered reference (tables change in place as the editor draws)
    this.tables.onChange((_, t) => {
      this.assetGen++;
      this.assetRefs.delete(t);
    });
    this.recordings.subscribe((o) => {
      this.assetGen++;
      for (const x of [this.recordings.osc[o], this.recordings.picture[o]]) if (x) this.assetRefs.delete(x);
    });
    this.multis.subscribe((o) => {
      this.assetGen++;
      const m = this.multis.osc[o];
      if (m) this.assetRefs.delete(m);
    });
    this.ir.subscribe((i) => {
      this.assetGen++;
      const u = this.ir.userIr(i);
      if (u) this.assetRefs.delete(u);
    });
    this.matrix.attach((i, s) => this.host?.send((w) => writeSlot(w, i, s)));
    this.remap.attach((osc, lut) => this.host?.send((w) => w.setOscCurve(0, osc, lut.length, lut)));
    this.fx.attach((chain, refs) =>
      this.host?.send((w) =>
        w.setChain(
          0,
          chain,
          refs.length,
          refs.map((r) => r.type * 256 + r.inst),
        ),
      ),
    );
    // convolvers build their response once they're in a rack
    this.ir.active = (inst) => this.fx.used(CONVOLVE, inst);
    this.fx.subscribe(() => {
      for (let i = 0; i < 4; i++) if (this.fx.used(CONVOLVE, i)) void this.ir.load(i);
    });
    this.lfo.attach((lfo, kind, pts) =>
      this.host?.send((w) =>
        w.setLfoShape(
          0,
          lfo,
          kind === 'path' ? 1 : 0,
          pts.length,
          pts.flatMap((p) => [p.x, p.y, p.c]),
        ),
      ),
    );
  }

  onStatus(fn: (s: SynthStatus) => void): () => void {
    this.statusSubs.add(fn);
    fn(this.status);
    return () => this.statusSubs.delete(fn);
  }

  private setStatus(s: SynthStatus): void {
    this.status = s;
    for (const fn of this.statusSubs) fn(s);
  }

  /** Load the engine. Safe to call more than once. */
  start(): Promise<void> {
    if (this.starting) return this.starting;
    this.setStatus('starting');
    this.starting = (async () => {
      try {
        const host = await EngineHost.create({
          onTrap: () => this.resync(),
          onFatal: (msg) => {
            this.error = msg;
            this.setStatus('error');
          },
        });
        this.host = host;
        this.tables.attach(host);
        this.noise.attach(host);
        this.ir.attach(host, host.ctx.sampleRate);
        this.tuning.attach({ setTuning: (t) => host.send((w) => w.setTuning(0, t.length, t)) });
        this.recordings.attach(host);
        this.multis.attach(host, host.ctx.sampleRate);
        this.specFilters.attach({ setSpectralFilter: (o, p) => host.send((w) => w.setSpectralFilter(0, o, p.length, p)) });
        this.arp.attach({ setArpPattern: (b, v) => host.send((w) => w.setArpPattern(0, b, v.length, v)) });
        this.clips.attach({
          setClip: (slot, notes, count, length) => host.send((w) => w.setClip(0, slot, count, length, notes)),
          setClipLane: (slot, lane, param, pts, count) => host.send((w) => w.setClipLane(0, slot, lane, param, count, pts)),
        });
        const follow = () => {
          if (this.status !== 'error') this.setStatus(host.ctx.state === 'running' ? 'running' : 'suspended');
        };
        host.ctx.addEventListener('statechange', follow);
        this.resync();
        this.guardTimer ??= setInterval(() => this.checkLoad(), 250);
        this.link.join();
        if (host.ctx.state !== 'running') await host.ctx.resume().catch(() => {});
        follow();
        // every oscillator starts on the factory saw (the engine's built-in
        // saw covers the moment before it arrives), unless the last session
        // being restored gives it its own
        await this.restoring;
        await Promise.all([0, 1, 2].map((o) => (this.tables.osc[o] ? null : this.tables.loadFactory(o, 'Saw'))));
        await this.noise.load();
      } catch (e) {
        this.error = String((e as Error)?.message ?? e);
        this.setStatus('error');
        this.starting = null;
        throw e;
      }
    })();
    return this.starting;
  }

  /** Four times a second: the CPU guard's reading (the worklet measures the load over the same quarter second). */
  private checkLoad(): void {
    const h = this.host;
    if (!h || h.ctx.state !== 'running') return;
    const stats = (h.ctx as unknown as { playbackStats?: { underrunEvents?: number } }).playbackStats;
    const n = stats?.underrunEvents ?? 0;
    const underran = this.underruns !== null && n > this.underruns;
    this.underruns = n;
    // the first seconds include the warm-up render and the context starting
    if (!this.guardOn || ++this.loadChecks <= 12) return;
    const level = this.guard.update(h.cpuPct, 0.25, underran);
    if (level !== null) h.send((w) => w.setGuard(0, level));
  }

  setGuardOn(on: boolean): void {
    this.guardOn = on;
    writeSetting('birdsynth.cpu-guard', on ? null : '0');
    if (!on && this.guard.level) {
      this.guard.reset();
      this.host?.send((w) => w.setGuard(0, 0));
    }
  }

  /** Browsers start audio suspended until a user gesture; call from one. */
  resume(): void {
    if (this.host && this.host.ctx.state !== 'running') void this.host.ctx.resume();
  }

  /** Send the whole state: parameters, matrix, LFO shapes, tables, noise, taps and the notes still held. */
  resync(): void {
    const host = this.host;
    if (!host) return;
    this.pushTaps();
    host.send((w) => {
      const v = this.bank.values;
      for (let id = 0; id < v.length; id++) w.setParam(0, id, v[id]);
      w.clearMod(0);
      w.setGuard(0, this.guard.level);
    });
    this.matrix.resync();
    this.lfo.resync();
    this.remap.resync();
    this.fx.resync();
    this.ir.resync();
    this.tuning.resync();
    this.recordings.resync();
    this.multis.resync();
    this.specFilters.resync();
    this.arp.resync();
    this.clips.resync();
    this.tables.resync();
    this.noise.resync();
    host.send((w) => {
      if (!this.bandlimit) w.debug(0, DEBUG.NoBandlimit, 1);
      for (const [noteId, n] of this.held) {
        w.noteOn(0, n.note, n.channel, n.velocity, noteId);
        if (this.mpe.member(n.channel)) this.mpe.get(n.channel).forEach((v, k) => w.noteExpression(0, k, v, noteId));
      }
    });
  }

  /** Route a matrix slot directly (the test API and presets; the UI uses `matrix`). */
  setModSlot(slot: number, source: number, dest: number, amount: number, flags = 0, aux = 0, curve = 0, output = 1): void {
    this.matrix.set(slot, source ? { source, aux, dest, amount, curve, output, bipolar: !!(flags & 1), bypass: !!(flags & 2) } : null);
  }

  // ----------------------------------------------------------- patches
  /** Called when a different preset is loaded or the current one is renamed or saved. */
  onPatch(fn: () => void): () => void {
    this.patchSubs.add(fn);
    return () => this.patchSubs.delete(fn);
  }

  private emitPatch(): void {
    for (const fn of this.patchSubs) fn();
  }

  /** The current state as a patch, with the wavetables and impulse responses that came from files. */
  async savePatch(meta: PatchMeta = this.meta): Promise<{ patch: Patch; assets: Map<string, ArrayBuffer> }> {
    const { patch, assets } = await this.collectPatch(meta);
    return { patch, assets: new Map([...assets].map(([h, get]) => [h, get()])) };
  }

  /** The current state as a patch, and its file-born assets by hash, each copied only when asked for (the session keeps just the new ones). */
  async collectPatch(meta: PatchMeta = this.meta): Promise<{ patch: Patch; assets: Map<string, () => ArrayBuffer> }> {
    const patch = capture(this.target, meta);
    const assets = new Map<string, () => ArrayBuffer>();
    const gen = this.assetGen;
    const refs: [object, unknown][] = [];
    for (let o = 0; o < patch.tables.length; o++) {
      const t = this.tables.osc[o];
      if (!t) continue;
      if (t.source.startsWith('factory:')) {
        patch.tables[o] = { name: t.name, source: t.source, count: t.count };
      } else {
        const hash = await hashBytes(t.frames);
        const frames = t.frames;
        assets.set(hash, () => frames.slice().buffer);
        patch.tables[o] = { name: t.name, source: t.source, count: t.count, hash };
        refs.push([t, patch.tables[o]]);
      }
    }
    for (let i = 0; i < patch.irs.length; i++) {
      const u = this.ir.userIr(i);
      if (!u) continue;
      const data = new Float32Array(u.l.length + u.r.length);
      data.set(u.l);
      data.set(u.r, u.l.length);
      const hash = await hashBytes(data);
      assets.set(hash, () => data.buffer);
      patch.irs[i] = { name: u.name, hash, rate: u.rate };
      refs.push([u, patch.irs[i]]);
    }
    for (let o = 0; o < patch.recordings.length; o++) {
      const pic = this.recordings.picture[o];
      const r = this.recordings.osc[o];
      if (pic) {
        const hash = await hashBytes(pic.bytes);
        const bytes = pic.bytes;
        assets.set(hash, () => bytes.slice(0));
        patch.recordings[o] = { kind: 'picture', name: pic.name, hash, seconds: pic.seconds };
        refs.push([pic, patch.recordings[o]]);
      } else if (r) {
        const n = r.channels[0].length;
        const data = new Float32Array(n * r.channels.length);
        r.channels.forEach((c, i) => data.set(c, i * n));
        const hash = await hashBytes(data);
        assets.set(hash, () => data.buffer);
        patch.recordings[o] = { kind: 'audio', name: r.name, hash, rate: r.rate, channels: r.channels.length, frames: n, slices: [...r.slices] };
        refs.push([r, patch.recordings[o]]);
      }
      const m = this.multis.osc[o];
      if (m?.source.startsWith('factory:')) patch.multis[o] = { name: m.name, source: m.source };
      else if (m) {
        const hash = await hashBytes(m.data);
        const mdata = m.data;
        assets.set(hash, () => mdata.slice().buffer);
        patch.multis[o] = { name: m.name, source: m.source, hash, zones: m.zones.length };
        refs.push([m, patch.multis[o]]);
      }
      if (!this.specFilters.isFlat(o)) patch.specFilter[o] = Array.from(this.specFilters.points[o]);
    }
    if (gen === this.assetGen) for (const [obj, ref] of refs) this.assetRefs.set(obj, ref);
    return { patch, assets };
  }

  /**
   * The current state as a patch, at once (no hashing): assets are named by
   * the references they had at their last collect, and one changed since is
   * left out. For the quick save on leaving the page.
   */
  private snapshotNow(): Patch {
    const patch = capture(this.target, this.meta);
    const ref = <T>(obj: object | null | undefined) => (obj ? ((this.assetRefs.get(obj) as T | undefined) ?? null) : null);
    for (let o = 0; o < patch.tables.length; o++) {
      const t = this.tables.osc[o];
      patch.tables[o] = t?.source.startsWith('factory:') ? { name: t.name, source: t.source, count: t.count } : ref(t);
      patch.recordings[o] = ref(this.recordings.picture[o] ?? this.recordings.osc[o]);
      const m = this.multis.osc[o];
      patch.multis[o] = m?.source.startsWith('factory:') ? { name: m.name, source: m.source } : ref(m);
      if (!this.specFilters.isFlat(o)) patch.specFilter[o] = Array.from(this.specFilters.points[o]);
    }
    for (let i = 0; i < patch.irs.length; i++) patch.irs[i] = ref(this.ir.userIr(i));
    return patch;
  }

  /**
   * Put the synth in a patch's state. `asset` finds a file's data by hash (the
   * library, or a file's embedded assets). Returns what couldn't be restored.
   */
  async loadPatch(patch: Patch, asset: (hash: string) => Promise<ArrayBuffer | undefined> = async () => undefined, id = ''): Promise<string[]> {
    this.allNotesOff();
    const warnings = applyPatch(this.target, patch);
    this.history.reset();
    this.meta = { ...patch.meta, tags: [...patch.meta.tags] };
    this.presetId = id;
    this.emitPatch();
    const tables = patch.tables.map(async (r, o) => {
      const cur = this.tables.osc[o];
      if (r?.hash) {
        const data = await asset(r.hash);
        if (data) return this.tables.set(o, { name: r.name, source: r.source, frames: new Float32Array(data), count: r.count });
        warnings.push(`the wavetable "${r.name}" is missing`);
      }
      const name = r?.source.startsWith('factory:') ? r.source.slice(8) : 'Saw';
      if (cur?.source === `factory:${name}`) return;
      try {
        await this.tables.loadFactory(o, name);
      } catch {
        warnings.push(`there's no factory wavetable "${name}"`);
        if (cur?.source !== 'factory:Saw') await this.tables.loadFactory(o, 'Saw');
      }
    });
    const irs = patch.irs.map(async (r, i) => {
      if (!r) return;
      const data = await asset(r.hash);
      if (!data) return void warnings.push(`the impulse response "${r.name}" is missing`);
      const f = new Float32Array(data);
      const n = f.length / 2;
      this.ir.setUser(i, r.name, [f.slice(0, n), f.slice(n)], r.rate);
    });
    const recs = patch.recordings.map(async (r, o) => {
      if (!r) {
        if (this.recordings.osc[o] || this.recordings.picture[o]) await this.recordings.set(o, null);
        return;
      }
      const data = await asset(r.hash);
      if (!data) return void warnings.push(`the recording "${r.name}" is missing`);
      if (r.kind === 'picture') {
        const bmp = await createImageBitmap(new Blob([data]));
        const c = new OffscreenCanvas(Math.min(1024, bmp.width), Math.min(512, bmp.height));
        const g = c.getContext('2d')!;
        g.drawImage(bmp, 0, 0, c.width, c.height);
        await this.recordings.setPicture(
          o,
          { name: r.name, bytes: data, seconds: r.seconds },
          pictureToAnalysis(g.getImageData(0, 0, c.width, c.height), r.seconds, this.host?.ctx.sampleRate ?? 48_000),
        );
        return;
      }
      const f = new Float32Array(data);
      const channels = Array.from({ length: r.channels }, (_, c) => f.slice(c * r.frames, (c + 1) * r.frames));
      await this.recordings.set(o, { name: r.name, rate: r.rate, channels, slices: [...r.slices] }, false);
    });
    const multis = patch.multis.map(async (m, o) => {
      if (!m) {
        if (this.multis.osc[o]) this.multis.set(o, null);
        return;
      }
      if (m.source.startsWith('factory:')) {
        try {
          await this.multis.loadFactory(o, m.source.slice(8));
        } catch {
          warnings.push(`there's no factory multisample "${m.name}"`);
        }
        return;
      }
      const data = m.hash ? await asset(m.hash) : undefined;
      if (!data || !m.zones) return void warnings.push(`the multisample "${m.name}" is missing`);
      const f = new Float32Array(data);
      this.multis.set(o, { name: m.name, source: m.source, zones: zoneInfo(f, m.zones, m.name), data: f, warnings: [] });
    });
    patch.specFilter.forEach((pts, o) => (pts ? this.specFilters.set(o, pts) : this.specFilters.isFlat(o) || this.specFilters.reset(o)));
    await Promise.all([...tables, ...irs, ...recs, ...multis]);
    return warnings;
  }

  /** The library, opened on first use. */
  openLibrary(): Promise<Library> {
    return (this.opening ??= Library.open().then((lib) => (this.library = lib)));
  }

  /** Load a library preset by id. */
  async loadEntry(id: string): Promise<string[]> {
    const lib = await this.openLibrary();
    const e = lib.entry(id);
    if (!e) return [`there's no preset ${id}`];
    return this.loadPatch(e.patch, (h) => lib.asset(h), id);
  }

  /** Save the current state to the library: over the current user preset, or as a new one. */
  async saveToLibrary(meta: PatchMeta = this.meta, asNew = false): Promise<Entry> {
    const lib = await this.openLibrary();
    const { patch, assets } = await this.savePatch(meta);
    const e = await lib.save(patch, assets, asNew ? undefined : this.presetId);
    this.meta = { ...e.patch.meta, tags: [...e.patch.meta.tags] };
    this.presetId = e.id;
    this.history.markClean();
    this.emitPatch();
    void lib.persist();
    return e;
  }

  /** Change the current preset's name, tags and so on (not an undo step). */
  setMeta(meta: Partial<PatchMeta>): void {
    this.meta = { ...this.meta, ...meta };
    this.emitPatch();
  }

  // ------------------------------------------------------------ session

  onSession(fn: () => void): () => void {
    this.sessionSubs.add(fn);
    return () => this.sessionSubs.delete(fn);
  }

  private emitSession(): void {
    for (const fn of this.sessionSubs) fn();
  }

  /**
   * At startup: put back the sound from the last visit (the library keeps
   * it), then keep it as it changes. If the last session can't be read, it's
   * left alone and nothing is saved over it.
   */
  restoreSession(): Promise<void> {
    return (this.restoring ??= (async () => {
      try {
        const lib = await Promise.race([this.openLibrary(), new Promise<never>((_, fail) => setTimeout(() => fail(new Error('the library took too long to open')), 5000))]);
        if (!lib.durable) {
          this.sessionState = 'memory';
          return;
        }
        const stored = await lib.loadSession();
        // the quick save from leaving the page, if it's newer than the last full one
        let tail: Session | null = null;
        try {
          const t = readTail();
          if (t && (!stored || t.saved > stored.saved)) tail = { ...t, patch: migrate(t.patch) };
        } catch {
          // unreadable: the full session stands
        }
        const s = tail ?? stored;
        if (s) {
          const warnings = await this.loadPatch(s.patch, (h) => lib.asset(h), s.presetId);
          if (s.dirty) this.history.markDirty();
          if (warnings.length) console.warn(`birdsynth: restoring your last session: ${warnings.join('; ')}`);
          this.sessionSaved = s.saved;
        }
        this.sessionState = 'on';
        // a newer tail becomes the full session (and the clean-up waits for that, so its assets are counted)
        if (s && s === tail) await this.saveSession();
        else writeSetting(TAIL_KEY, null);
        void lib.collectGarbage();
        if (typeof window !== 'undefined') {
          // leaving (or hiding) the page: a full save may not finish in time, so the state
          // also goes to localStorage at once (without assets: they're stored already)
          const leave = () => {
            if (this.sessionTimer || this.sessionSaving) this.writeTail();
            void this.saveSession();
          };
          window.addEventListener('pagehide', leave);
          document.addEventListener('visibilitychange', () => document.visibilityState === 'hidden' && leave());
        }
      } catch (e) {
        this.sessionState = 'failed';
        console.warn("birdsynth: the last session could not be restored, so this one won't be kept:", e);
      } finally {
        this.emitSession();
      }
    })());
  }

  private touchSession(): void {
    if (this.sessionState !== 'on' || this.sessionHolds) return;
    if (this.sessionTimer) clearTimeout(this.sessionTimer);
    this.sessionTimer = setTimeout(() => void this.saveSession(), SESSION_SETTLE_MS);
  }

  /** Keep the current state now (it's also kept a moment after each change). */
  saveSession(): Promise<void> {
    if (this.sessionTimer) clearTimeout(this.sessionTimer);
    this.sessionTimer = null;
    if (this.sessionState !== 'on' || this.sessionHolds) return Promise.resolve();
    if (this.sessionSaving) {
      this.sessionAgain = true;
      return this.sessionSaving;
    }
    this.sessionSaving = (async () => {
      try {
        const started = Date.now();
        const lib = await this.openLibrary();
        const { patch, assets } = await this.collectPatch();
        await lib.saveSession(patch, assets, this.presetId, this.history.dirty);
        this.sessionSaved = Date.now();
        const tail = readTail();
        if (tail && tail.saved <= started) writeSetting(TAIL_KEY, null);
        if (!this.persistAsked) {
          this.persistAsked = true;
          void lib.persist();
        }
      } catch (e) {
        console.warn('birdsynth: could not keep the session:', e);
      } finally {
        this.sessionSaving = null;
        this.emitSession();
        if (this.sessionAgain) {
          this.sessionAgain = false;
          void this.saveSession();
        }
      }
    })();
    return this.sessionSaving;
  }

  /** The quick save: the state as it is this moment, to localStorage (read back at the next start if it's newer). */
  private writeTail(): void {
    if (this.sessionState !== 'on' || this.sessionHolds) return;
    try {
      writeSetting(TAIL_KEY, JSON.stringify({ saved: Date.now(), patch: this.snapshotNow(), presetId: this.presetId, dirty: this.history.dirty }));
    } catch {
      // too big for localStorage: the full save is all there is
    }
  }

  /** Stop keeping the session while something borrows the synth (a tour's teaching patches), and start again after. */
  holdSession(on: boolean): void {
    this.sessionHolds = Math.max(0, this.sessionHolds + (on ? 1 : -1));
    if (!this.sessionHolds) this.touchSession();
  }

  /**
   * Start over: the Init sound, as a fresh session. The library's presets
   * and this browser's setup (MIDI mappings, tuning, settings) stay.
   */
  async resetSession(): Promise<void> {
    this.stopPreview();
    this.transport(false);
    this.setBandlimit(true);
    await this.loadEntry('factory:Init');
    writeSetting(TAIL_KEY, null);
    await this.saveSession();
    if (this.sessionState === 'on') void (await this.openLibrary()).collectGarbage();
  }

  // ----------------------------------------------------------- previews
  private previewTimers: ReturnType<typeof setTimeout>[] = [];
  private previewNotes = new Set<number>();
  private readonly previewSubs = new Set<() => void>();

  /** Whether a preview phrase is playing. */
  get previewing(): boolean {
    return this.previewTimers.length > 0;
  }

  onPreview(fn: () => void): () => void {
    this.previewSubs.add(fn);
    return () => this.previewSubs.delete(fn);
  }

  /**
   * Play the current preset's preview phrase (its first clip, or a phrase for
   * its category) on a channel of its own, so it never takes a key you hold.
   */
  preview(): void {
    this.stopPreview();
    const phrase = phraseFor({ meta: this.meta, clips: this.clips.clips });
    const beat = 60_000 / toPlain(PARAMS[PARAM_ID['global.bpm']], this.bank.get(PARAM_ID['global.bpm']));
    const at = (fn: () => void, beats: number) => this.previewTimers.push(setTimeout(fn, beats * beat));
    for (const nt of phrase.notes) {
      at(() => {
        this.noteOn(nt.key, nt.velocity, PREVIEW_CHANNEL);
        this.previewNotes.add(nt.key);
      }, nt.at);
      at(() => {
        this.noteOff(nt.key, PREVIEW_CHANNEL);
        this.previewNotes.delete(nt.key);
      }, nt.at + nt.len);
    }
    at(() => this.stopPreview(), phrase.beats + 0.5);
    for (const fn of this.previewSubs) fn();
  }

  stopPreview(): void {
    if (!this.previewTimers.length) return;
    for (const t of this.previewTimers) clearTimeout(t);
    this.previewTimers = [];
    for (const k of this.previewNotes) this.noteOff(k, PREVIEW_CHANNEL);
    this.previewNotes.clear();
    for (const fn of this.previewSubs) fn();
  }

  /** Load a hybrid of two library presets (see state/hybrid.ts); unsaved, like an edit. */
  async loadHybrid(idA: string, idB: string, opt: HybridOptions): Promise<string[]> {
    const lib = await this.openLibrary();
    const a = lib.entry(idA);
    const b = lib.entry(idB);
    if (!a || !b) return [`there's no preset ${a ? idB : idA}`];
    return this.loadPatch(hybridize(a.patch, b.patch, opt), (h) => lib.asset(h));
  }

  // ---------------------------------------------------------- transport
  /** Start or stop the transport (clips play while it runs); while linked, for the whole group. */
  transport(play: boolean): void {
    if (this.link.linked) this.link.request({ kind: play ? 'play' : 'stop' });
    else this.host?.send((w) => w.transport(0, play ? 1 : 0));
    if (!play) this.stopRecording();
    for (const fn of this.transportSubs) fn();
  }

  /** Set a parameter so it lands in the engine at `frame` (a linked tempo change, on the timeline's moment). */
  setParamAt(frame: number, key: ParamKey, plain: number): void {
    this.paramFrame = frame;
    try {
      this.setParam(key, toNorm(PARAMS[PARAM_ID[key]], plain));
    } finally {
      this.paramFrame = 0;
    }
  }

  onTransport(fn: () => void): () => void {
    this.transportSubs.add(fn);
    return () => this.transportSubs.delete(fn);
  }

  get playing(): boolean {
    return (this.host?.tel[TEL.playing] ?? 0) >= 0.5;
  }

  /** Play if stopped, stop if playing (the space bar). While linked it goes by the group's timeline, which a change reaches before the engine does. */
  toggleTransport(): void {
    const t = this.link.linked ? this.link.timeline : null;
    this.transport(!(t ? t.playing : this.playing));
  }

  /** The transport's position now, in beats (from the newest telemetry, moved on by the time since). */
  beatNow(): number {
    const h = this.host;
    if (!h) return 0;
    const bpm = toPlain(PARAMS[PARAM_ID['global.bpm']], this.bank.get(PARAM_ID['global.bpm']));
    // the telemetry is from the end of the newest block rendered; the listener is behind that
    const rendered = h.frame + BLOCK_FRAMES;
    return h.tel[TEL.beat] + ((h.heardFrame() - rendered) / h.ctx.sampleRate) * (bpm / 60);
  }

  /** Where a clip is now, in beats from its start (the transport's beat when it isn't the one playing). */
  clipBeatNow(slot: number): number {
    const h = this.host;
    const now = this.beatNow();
    return h && h.tel[TEL.clipPlaying] === slot ? h.tel[TEL.clipPos] + (now - h.tel[TEL.beat]) : now;
  }

  /** Record played notes into a clip (overdubbing what's there) while the transport runs. */
  record(slot: number): void {
    this.recording = { slot, open: new Map() };
    if (!this.playing) this.transport(true);
  }

  stopRecording(): void {
    const r = this.recording;
    if (!r) return;
    const now = this.beatNow();
    for (const [k, o] of r.open) this.writeNote(r.slot, Number(k.split(':')[1]), o.start, now - o.beat, o.velocity);
    this.recording = null;
  }

  /** A recorded note: its start in the clip (wrapped into it) and its length in beats (however many loops it was held over). */
  private writeNote(slot: number, key: number, start: number, length: number, velocity: number): void {
    const len = this.clips.clips[slot].length;
    const s = ((start % len) + len) % len;
    this.clips.edit(slot, (clip) => clip.notes.push({ start: Math.round(s * 960) / 960, length: Math.max(1 / 64, Math.min(len, length)), key, velocity, chance: 1, bend: 0 }));
  }

  // ------------------------------------------------------------- taps
  /** Ask for a tap while something shows it; call the returned function when done. */
  useTap(name: TapName): () => void {
    this.tapUse.set(name, (this.tapUse.get(name) ?? 0) + 1);
    this.pushTaps();
    let done = false;
    return () => {
      if (done) return;
      done = true;
      const n = (this.tapUse.get(name) ?? 1) - 1;
      if (n > 0) this.tapUse.set(name, n);
      else this.tapUse.delete(name);
      this.pushTaps();
    };
  }

  /** Taps being recorded now (the oldest requests win when more than a block carries are wanted). */
  get activeTaps(): TapName[] {
    return [...this.tapUse.keys()].slice(0, BLOCK_MAX_TAPS);
  }

  private pushTaps(): void {
    this.host?.setTaps(this.activeTaps);
  }

  // ----------------------------------------------------------- params
  setParam(key: ParamKey, norm: number): void {
    this.bank.set(PARAM_ID[key], norm);
  }

  getParam(key: ParamKey): number {
    return this.bank.get(PARAM_ID[key]);
  }

  // ------------------------------------------------------------ notes
  noteOn(note: number, velocity = 0.8, channel = 0): number {
    if (velocity <= 0) {
      this.noteOff(note, channel);
      return 0;
    }
    this.resume();
    if (note >= 24 && note < 36 && this.bank.get(PARAM_ID['clip.enable']) >= 0.5 && this.bank.get(PARAM_ID['clip.trigger_keys']) >= 0.5) {
      // C1..B1 launch clips 1..12 (here, so the slot the page shows is the one playing)
      this.bank.set(PARAM_ID['clip.slot'], (note - 24) / 11);
      return 0;
    }
    const id = this.nextId;
    this.nextId = this.nextId >= 0x7ffe_ffff ? 1 : this.nextId + 1;
    this.held.set(id, { note, channel, velocity });
    const k = `${channel}:${note}`;
    const ids = this.byKey.get(k);
    if (ids) ids.push(id);
    else this.byKey.set(k, [id]);
    this.host?.send((w) => {
      w.noteOn(0, note, channel, velocity, id);
      // an MPE note starts with its channel's expression (controllers send it just before the note)
      if (this.mpe.member(channel)) this.mpe.get(channel).forEach((v, k) => w.noteExpression(0, k, v, id));
    });
    if (this.recording) this.recording.open.set(`${channel}:${note}`, { start: this.clipBeatNow(this.recording.slot), beat: this.beatNow(), velocity });
    return id;
  }

  noteOff(note: number, channel = 0, velocity = 0): void {
    const k = `${channel}:${note}`;
    const ids = this.byKey.get(k);
    const id = ids?.shift();
    if (id === undefined) return;
    if (!ids!.length) this.byKey.delete(k);
    this.held.delete(id);
    this.host?.send((w) => w.noteOff(0, note, channel, velocity, id));
    const rec = this.recording?.open.get(`${channel}:${note}`);
    if (rec && this.recording) {
      this.recording.open.delete(`${channel}:${note}`);
      this.writeNote(this.recording.slot, note, rec.start, this.beatNow() - rec.beat, rec.velocity);
    }
  }

  allNotesOff(): void {
    this.held.clear();
    this.byKey.clear();
    this.host?.send((w) => w.allNotesOff(0));
  }

  get heldNotes(): number[] {
    return [...this.held.values()].map((h) => h.note);
  }

  setFollowClock(on: boolean): void {
    this.followClock = on;
    this.clock.reset();
    writeSetting('birdsynth.midi-clock', on ? '1' : null);
  }

  realtime(status: number, time: number): void {
    if (!this.followClock) return;
    if (status === 0xf8) {
      const bpm = this.clock.tick(time);
      if (bpm !== null) this.setParam('global.bpm', toNorm(PARAMS[PARAM_ID['global.bpm']], bpm));
    } else if (status === 0xfa || status === 0xfb) {
      if (!this.playing || status === 0xfa) this.transport(true);
    } else if (status === 0xfc) {
      this.transport(false);
    }
  }

  // ---------------------------------------------- MidiSink (the rest)
  /** One MPE channel's expression: to every note held on it. */
  private expression(channel: number, kind: number, value: number): void {
    this.mpe.set(channel, kind, value);
    const ids = [...this.held].filter(([, h]) => h.channel === channel).map(([id]) => id);
    if (ids.length) this.host?.send((w) => ids.forEach((id) => w.noteExpression(0, kind, value, id)));
  }

  pitchBend(channel: number, value: number): void {
    if (this.mpe.member(channel)) return this.expression(channel, MPE_X, value);
    this.wheels.bend = value;
    this.emitWheels();
    this.host?.send((w) => w.pitchBend(0, channel, value));
  }

  controller(channel: number, cc: number, value: number): void {
    if (cc === 123) return this.allNotesOff();
    if (cc === MPE_Y_CC && this.mpe.member(channel)) return this.expression(channel, MPE_Y, value);
    this.learn.controller(channel, cc, value);
    if (cc === 1) {
      this.wheels.mod = value;
      this.emitWheels();
    }
    this.host?.send((w) => w.controller(0, channel, cc, value));
  }

  /** Called when a wheel moves. */
  onWheels(fn: () => void): () => void {
    this.wheelSubs.add(fn);
    return () => this.wheelSubs.delete(fn);
  }

  private emitWheels(): void {
    for (const fn of this.wheelSubs) fn();
  }

  channelPressure(channel: number, value: number): void {
    if (this.mpe.member(channel)) return this.expression(channel, MPE_Z, value);
    this.host?.send((w) => w.channelPressure(0, channel, value));
  }

  polyPressure(note: number, channel: number, value: number): void {
    const id = this.byKey.get(`${channel}:${note}`)?.at(-1) ?? 0;
    this.host?.send((w) => w.polyPressure(0, note, channel, value, id));
  }

  // --------------------------------------------------------- readouts
  telemetry(): Telemetry {
    const h = this.host;
    const t = h?.tel;
    const notes: number[] = [];
    if (t) {
      for (let i = 0; i < TEL_COUNT.voiceNote; i++) {
        const n = t[TEL.voiceNote + i];
        if (n >= 0 && t[TEL.voiceLevel + i] > 0) notes.push(n);
      }
    }
    return {
      voicesActive: t?.[TEL.voicesActive] ?? 0,
      peakL: t?.[TEL.peakL] ?? 0,
      peakR: t?.[TEL.peakR] ?? 0,
      focusVoice: t?.[TEL.focusVoice] ?? -1,
      queueDrops: t?.[TEL.queueDrops] ?? 0,
      unknownCmds: t?.[TEL.unknownCmds] ?? 0,
      notes,
      cpuPct: h?.cpuPct ?? 0,
      traps: h?.traps ?? 0,
      dropped: h?.dropped ?? 0,
      blocks: h?.blocks ?? 0,
      frame: h?.frame ?? 0,
    };
  }

  /** The last `frames` samples of a tap, ending at the newest recorded frame. */
  tap(name: TapName, frames = 1024): Float32Array {
    const out = new Float32Array(frames);
    const h = this.host;
    if (h) h.taps.read(TAP[name], h.taps.latest(TAP[name]), out);
    return out;
  }

  /** Switch the oscillators' band-limiting off to hear aliasing (a teaching switch). */
  setBandlimit(on: boolean): void {
    this.bandlimit = on;
    this.host?.send((w) => w.debug(0, DEBUG.NoBandlimit, on ? 0 : 1));
  }

  /** Make the engine panic, to exercise trap recovery. */
  debugTrap(): void {
    this.host?.send((w) => w.debug(0, DEBUG.Trap, 0));
  }
}

function writeSlot(w: CmdWriter, i: number, s: ModSlot | null): void {
  if (s) w.setModSlot(0, i, s.source, s.aux, slotFlags(s), s.dest, s.amount, s.curve, s.output);
  else w.setModSlot(0, i, 0, 0, 0, 0, 0, 0, 1);
}

/** Quiet time after a change before the session is saved. */
const SESSION_SETTLE_MS = 700;
/** Where the quick save on leaving the page goes. */
const TAIL_KEY = 'birdsynth.session-tail';

function readTail(): { saved: number; patch: Patch; presetId: string; dirty: boolean } | null {
  try {
    const t = JSON.parse(readSetting(TAIL_KEY) ?? 'null');
    return t && typeof t.saved === 'number' && t.patch ? t : null;
  } catch {
    return null;
  }
}

/** Preview phrases play on their own MIDI channel (16). */
const PREVIEW_CHANNEL = 15;

/** A setting of this browser (localStorage, which may be missing or refuse). */
function readSetting(key: string): string | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeSetting(key: string, value: string | null): void {
  try {
    if (typeof localStorage === 'undefined') return;
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // private mode: the setting lasts this session only
  }
}
