// The synth as the UI sees it: the parameter bank, held notes and the
// engine host, behind one object. Inputs (QWERTY, the on-screen keyboard,
// MIDI) and the test API all play through here.

import { PARAM_ID, type ParamKey } from './gen/params';
import { DEBUG, TEL, TEL_COUNT, TAP, type CmdWriter, type TapName } from './gen/protocol';
import { EngineHost } from './audio/host';
import { BLOCK_MAX_TAPS } from './audio/block';
import { ParamBank } from './state/bank';
import { TableStore } from './state/tables';
import { LfoShapes } from './state/lfo';
import { ModMatrix, slotFlags, type ModSlot } from './state/matrix';
import { NoiseStore } from './state/noise';
import type { MidiSink } from './input/midi';

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

  constructor() {
    this.bank.onAny((id, v) => this.host?.send((w) => w.setParam(0, id, v)));
    this.matrix.attach((i, s) => this.host?.send((w) => writeSlot(w, i, s)));
    this.lfo.attach((lfo, kind, pts) =>
      this.host?.send((w) => w.setLfoShape(0, lfo, kind === 'path' ? 1 : 0, pts.length, pts.flatMap((p) => [p.x, p.y, p.c]))),
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
    this.tables.resync();
    this.noise.resync();
    host.send((w) => {
      for (const [noteId, n] of this.held) w.noteOn(0, n.note, n.channel, n.velocity, noteId);
    });
  }

  /** Route a matrix slot directly (the test API and presets; the UI uses `matrix`). */
  setModSlot(slot: number, source: number, dest: number, amount: number, flags = 0, aux = 0, curve = 0, output = 1): void {
    this.matrix.set(slot, source ? { source, aux, dest, amount, curve, output, bipolar: !!(flags & 1), bypass: !!(flags & 2) } : null);
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
    const id = this.nextId;
    this.nextId = this.nextId >= 0x7ffe_ffff ? 1 : this.nextId + 1;
    this.held.set(id, { note, channel, velocity });
    const k = `${channel}:${note}`;
    const ids = this.byKey.get(k);
    if (ids) ids.push(id);
    else this.byKey.set(k, [id]);
    this.host?.send((w) => w.noteOn(0, note, channel, velocity, id));
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
  }

  allNotesOff(): void {
    this.held.clear();
    this.byKey.clear();
    this.host?.send((w) => w.allNotesOff(0));
  }

  get heldNotes(): number[] {
    return [...this.held.values()].map((h) => h.note);
  }

  // ---------------------------------------------- MidiSink (the rest)
  pitchBend(channel: number, value: number): void {
    this.host?.send((w) => w.pitchBend(0, channel, value));
  }

  controller(channel: number, cc: number, value: number): void {
    if (cc === 123) return this.allNotesOff();
    this.host?.send((w) => w.controller(0, channel, cc, value));
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

  /** Make the engine panic, to exercise trap recovery. */
  debugTrap(): void {
    this.host?.send((w) => w.debug(0, DEBUG.Trap, 0));
  }
}

function writeSlot(w: CmdWriter, i: number, s: ModSlot | null): void {
  if (s) w.setModSlot(0, i, s.source, s.aux, slotFlags(s), s.dest, s.amount, s.curve, s.output);
  else w.setModSlot(0, i, 0, 0, 0, 0, 0, 0, 1);
}
