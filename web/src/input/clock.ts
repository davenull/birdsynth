// Following an external MIDI clock: 24 ticks to the beat set the tempo;
// Start, Continue and Stop run the transport.

export const TICKS_PER_BEAT = 24;

export class MidiClock {
  /** Times (ms) of the most recent ticks, a beat's worth. */
  private ticks: number[] = [];
  private bpm = 0;

  /**
   * A tick at `time` (ms). Returns the tempo once a beat of ticks has
   * arrived, and again whenever it moves by more than 0.05 BPM; null
   * otherwise.
   */
  tick(time: number): number | null {
    const t = this.ticks;
    // a gap of over a second: the clock stopped and started again
    if (t.length && time - t[t.length - 1] > 1000) t.length = 0;
    t.push(time);
    if (t.length > TICKS_PER_BEAT + 1) t.shift();
    if (t.length < TICKS_PER_BEAT + 1) return null;
    const perTick = (t[t.length - 1] - t[0]) / (t.length - 1);
    if (perTick <= 0) return null;
    const bpm = Math.round((60_000 / (perTick * TICKS_PER_BEAT)) * 100) / 100;
    if (Math.abs(bpm - this.bpm) <= 0.05) return null;
    this.bpm = bpm;
    return bpm;
  }

  reset(): void {
    this.ticks = [];
    this.bpm = 0;
  }
}
