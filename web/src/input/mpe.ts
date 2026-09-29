// MPE on the host. With MPE on, channel 1 is the zone's master channel (its
// bend, pressure and controllers reach every note, as without MPE) and
// channels 2–16 each carry one note's own expression: pitch bend is X, CC 74
// is Y and channel pressure is Z. The engine takes them per note id.

export const MPE_X = 0;
export const MPE_Y = 1;
export const MPE_Z = 2;
/** The controller MPE uses for Y (slide, "timbre"). */
export const MPE_Y_CC = 74;

export class MpeChannels {
  on = false;
  /** Each channel's latest X, Y and Z: a note starting on it takes these. */
  private readonly state = Array.from({ length: 16 }, () => [0, 0, 0]);

  /** Whether a message on `channel` (0-based) belongs to the note(s) on it. */
  member(channel: number): boolean {
    return this.on && channel >= 1 && channel <= 15;
  }

  set(channel: number, kind: number, value: number): void {
    this.state[channel][kind] = value;
  }

  get(channel: number): readonly number[] {
    return this.state[channel];
  }

  reset(): void {
    for (const s of this.state) s.fill(0);
  }
}
