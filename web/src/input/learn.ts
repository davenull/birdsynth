// MIDI learn: tie a controller (CC) to any parameter. Arm a parameter
// (right-click a knob, "MIDI learn"), move a control, and from then on that
// CC sets the parameter. Mappings belong to the setup, not to a patch, so
// they're kept in localStorage and survive preset changes.

import { PARAM_ID, type ParamKey } from '../gen/params';
import type { ParamBank } from '../state/bank';

export interface Mapping {
  cc: number;
  /** null: any channel */
  channel: number | null;
  param: ParamKey;
}

const KEY = 'birdsynth.midi-learn';

export class MidiLearn {
  armed: ParamKey | null = null;
  maps: Mapping[] = [];
  private readonly subs = new Set<() => void>();

  constructor(
    private readonly bank: ParamBank,
    private readonly storage: Pick<Storage, 'getItem' | 'setItem'> | null = typeof localStorage === 'undefined' ? null : localStorage,
  ) {
    try {
      const raw = JSON.parse(this.storage?.getItem(KEY) ?? '[]') as Mapping[];
      this.maps = raw.filter((m) => m.param in PARAM_ID && m.cc >= 0 && m.cc < 128);
    } catch {
      this.maps = [];
    }
  }

  subscribe(fn: () => void): () => void {
    this.subs.add(fn);
    return () => this.subs.delete(fn);
  }

  private changed(save = true): void {
    if (save) {
      try {
        this.storage?.setItem(KEY, JSON.stringify(this.maps));
      } catch {
        // storage full or blocked: the mapping lasts for the session
      }
    }
    for (const fn of this.subs) fn();
  }

  /** The next controller that moves gets tied to `param`. */
  arm(param: ParamKey): void {
    this.armed = param;
    this.changed(false);
  }

  cancel(): void {
    this.armed = null;
    this.changed(false);
  }

  clear(param: ParamKey): void {
    this.maps = this.maps.filter((m) => m.param !== param);
    this.changed();
  }

  mapping(param: ParamKey): Mapping | undefined {
    return this.maps.find((m) => m.param === param);
  }

  /** A controller moved (value 0..1): learn it, or set the parameters it's tied to. */
  controller(channel: number, cc: number, value: number): void {
    if (this.armed) {
      const param = this.armed;
      this.armed = null;
      this.maps = this.maps.filter((m) => m.param !== param && m.cc !== cc);
      this.maps.push({ cc, channel: null, param });
      this.changed();
    }
    for (const m of this.maps) {
      if (m.cc === cc && (m.channel === null || m.channel === channel)) this.bank.set(PARAM_ID[m.param], value);
    }
  }
}
