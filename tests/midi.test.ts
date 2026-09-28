// MIDI learn: arm a parameter, move a controller, and that CC drives it from
// then on (kept across sessions); the parser feeds it the same way Web MIDI does.

import { describe, expect, it } from 'vitest';
import { PARAM_ID } from '../web/src/gen/params';
import { MidiLearn } from '../web/src/input/learn';
import { parseMidi, type MidiSink } from '../web/src/input/midi';
import { ParamBank } from '../web/src/state/bank';

function memory() {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

function sink(learn: MidiLearn): MidiSink {
  const nop = () => {};
  return { noteOn: nop, noteOff: nop, pitchBend: nop, channelPressure: nop, polyPressure: nop, controller: (ch, cc, v) => learn.controller(ch, cc, v) };
}

describe('MIDI learn', () => {
  it('ties the next controller to the armed parameter and keeps it', () => {
    const store = memory();
    const bank = new ParamBank();
    const learn = new MidiLearn(bank, store);
    const cut = PARAM_ID['filter.1.cutoff'];
    const before = bank.get(cut);
    parseMidi([0xb0, 20, 100], sink(learn)); // not mapped: nothing happens
    expect(bank.get(cut)).toBe(before);
    learn.arm('filter.1.cutoff');
    expect(learn.armed).toBe('filter.1.cutoff');
    parseMidi([0xb3, 20, 64], sink(learn)); // any channel
    expect(learn.armed).toBeNull();
    expect(learn.mapping('filter.1.cutoff')).toEqual({ cc: 20, channel: null, param: 'filter.1.cutoff' });
    expect(bank.get(cut)).toBeCloseTo(64 / 127, 6);
    parseMidi([0xb0, 20, 127, 20, 0], sink(learn)); // running status
    expect(bank.get(cut)).toBe(0);

    // learning the same CC for another parameter moves it
    learn.arm('osc.a.wt_pos');
    parseMidi([0xb0, 20, 10], sink(learn));
    expect(learn.mapping('filter.1.cutoff')).toBeUndefined();
    expect(bank.get(PARAM_ID['osc.a.wt_pos'])).toBeCloseTo(10 / 127, 6);

    // a new session reads the mappings back
    const again = new MidiLearn(new ParamBank(), store);
    expect(again.maps).toEqual([{ cc: 20, channel: null, param: 'osc.a.wt_pos' }]);
    again.clear('osc.a.wt_pos');
    expect(new MidiLearn(new ParamBank(), store).maps).toEqual([]);
  });

  it('ignores mappings to parameters this build does not have', () => {
    const store = memory();
    store.setItem('birdsynth.midi-learn', JSON.stringify([{ cc: 7, channel: null, param: 'osc.z.nothing' }, { cc: 8, channel: 2, param: 'master.volume' }]));
    const learn = new MidiLearn(new ParamBank(), store);
    expect(learn.maps).toEqual([{ cc: 8, channel: 2, param: 'master.volume' }]);
  });
});
