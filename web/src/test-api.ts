// window.__synth: a small surface for driving and inspecting the synth from
// tests and the browser console, e.g.
//   __synth.noteOn(60); __synth.telemetry(); __synth.midiIn([0x90, 64, 100])

import { PARAMS, type ParamKey } from './gen/params';
import type { TapName } from './gen/protocol';
import { parseMidi } from './input/midi';
import type { Synth } from './synth';

interface PlaybackStats {
  underrunEvents?: number;
  underrunDuration?: number;
}

export function installTestApi(synth: Synth): void {
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
    params: () => PARAMS.map((p) => p.key),
    /** Feed raw MIDI bytes through the same parser Web MIDI uses. */
    midiIn: (bytes: number[]) => parseMidi(bytes, synth),
    telemetry: () => synth.telemetry(),
    tap: (name: TapName, frames = 1024) => Array.from(synth.tap(name, frames)),
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
  };
  (window as unknown as { __synth: typeof api }).__synth = api;
}
