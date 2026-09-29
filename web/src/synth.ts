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
import { Library, type Entry } from './state/library';
import { applyPatch, capture, emptyMeta, hashBytes, type Patch, type PatchMeta, type PatchTarget } from './state/patch';
import { parseMidi, type MidiSink } from './input/midi';
import { MidiLearn } from './input/learn';
import { WebMidi } from './input/webmidi';
import { MidiClock } from './input/clock';

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
    this.bank.onAny((id, v) => this.host?.send((w) => w.setParam(0, id, v)));
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
        if (host.ctx.state !== 'running') await host.ctx.resume().catch(() => {});
        follow();
        // every oscillator starts on the factory saw (the engine's built-in
        // saw covers the moment before it arrives)
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
      for (const [noteId, n] of this.held) w.noteOn(0, n.note, n.channel, n.velocity, noteId);
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
    const patch = capture(this.target, meta);
    const assets = new Map<string, ArrayBuffer>();
    for (let o = 0; o < patch.tables.length; o++) {
      const t = this.tables.osc[o];
      if (!t) continue;
      if (t.source.startsWith('factory:')) {
        patch.tables[o] = { name: t.name, source: t.source, count: t.count };
      } else {
        const hash = await hashBytes(t.frames);
        assets.set(hash, t.frames.slice().buffer);
        patch.tables[o] = { name: t.name, source: t.source, count: t.count, hash };
      }
    }
    for (let i = 0; i < patch.irs.length; i++) {
      const u = this.ir.userIr(i);
      if (!u) continue;
      const data = new Float32Array(u.l.length + u.r.length);
      data.set(u.l);
      data.set(u.r, u.l.length);
      const hash = await hashBytes(data);
      assets.set(hash, data.buffer);
      patch.irs[i] = { name: u.name, hash, rate: u.rate };
    }
    for (let o = 0; o < patch.recordings.length; o++) {
      const pic = this.recordings.picture[o];
      const r = this.recordings.osc[o];
      if (pic) {
        const hash = await hashBytes(pic.bytes);
        assets.set(hash, pic.bytes.slice(0));
        patch.recordings[o] = { kind: 'picture', name: pic.name, hash, seconds: pic.seconds };
      } else if (r) {
        const n = r.channels[0].length;
        const data = new Float32Array(n * r.channels.length);
        r.channels.forEach((c, i) => data.set(c, i * n));
        const hash = await hashBytes(data);
        assets.set(hash, data.buffer);
        patch.recordings[o] = { kind: 'audio', name: r.name, hash, rate: r.rate, channels: r.channels.length, frames: n, slices: [...r.slices] };
      }
      const m = this.multis.osc[o];
      if (m?.source.startsWith('factory:')) patch.multis[o] = { name: m.name, source: m.source };
      else if (m) {
        const hash = await hashBytes(m.data);
        assets.set(hash, m.data.slice().buffer);
        patch.multis[o] = { name: m.name, source: m.source, hash, zones: m.zones.length };
      }
      if (!this.specFilters.isFlat(o)) patch.specFilter[o] = Array.from(this.specFilters.points[o]);
    }
    return { patch, assets };
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

  // ---------------------------------------------------------- transport
  /** Start or stop the transport (clips play while it runs). */
  transport(play: boolean): void {
    this.host?.send((w) => w.transport(0, play ? 1 : 0));
    if (!play) this.stopRecording();
    for (const fn of this.transportSubs) fn();
  }

  onTransport(fn: () => void): () => void {
    this.transportSubs.add(fn);
    return () => this.transportSubs.delete(fn);
  }

  get playing(): boolean {
    return (this.host?.tel[TEL.playing] ?? 0) >= 0.5;
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
    this.host?.send((w) => w.noteOn(0, note, channel, velocity, id));
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
  pitchBend(channel: number, value: number): void {
    this.wheels.bend = value;
    this.emitWheels();
    this.host?.send((w) => w.pitchBend(0, channel, value));
  }

  controller(channel: number, cc: number, value: number): void {
    if (cc === 123) return this.allNotesOff();
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
