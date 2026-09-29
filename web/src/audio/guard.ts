// The CPU guard. When the engine runs above 70% of real time for half a second
// (or the output underruns while the engine is busy), it steps up a level: 1 turns oversampling off,
// 2–4 cap unison at 8, 4 and then 2 voices. After ten quiet seconds below
// 35% it steps back down. The patch never changes; the engine just limits
// what it renders (SetGuard).

export const GUARD_HIGH = 70;
/** Above this, one reading is enough: the output is already about to break up. */
export const GUARD_SEVERE = 85;
export const GUARD_LOW = 35;
export const GUARD_MAX = 4;
/** Seconds over the line before stepping up, and under before stepping down. */
const UP_AFTER = 0.5;
const DOWN_AFTER = 10;

export class CpuGuard {
  level = 0;
  private over = 0;
  private under = 0;

  /**
   * One reading: the load (percent of real time) over the last `dt` seconds,
   * and whether the output underran meanwhile. Returns the new level when it
   * changes, else null.
   */
  update(cpu: number, dt: number, underran = false): number | null {
    // an underrun with the engine nearly idle is someone else's doing: easing the engine won't help
    if (cpu > GUARD_HIGH || (underran && cpu > GUARD_LOW)) {
      this.over += cpu > GUARD_SEVERE ? UP_AFTER : dt;
      this.under = 0;
    } else if (cpu < GUARD_LOW) {
      this.under += dt;
      this.over = 0;
    } else {
      this.over = 0;
      this.under = 0;
    }
    if (this.over >= UP_AFTER && this.level < GUARD_MAX) {
      this.over = 0;
      return ++this.level;
    }
    if (this.under >= DOWN_AFTER && this.level > 0) {
      this.under = 0;
      return --this.level;
    }
    return null;
  }

  reset(): void {
    this.level = 0;
    this.over = 0;
    this.under = 0;
  }
}

/** What a level does, for the CPU readout's tooltip. */
export function guardText(level: number): string {
  return ['', 'oversampling off', 'oversampling off, unison capped at 8', 'oversampling off, unison capped at 4', 'oversampling off, unison capped at 2'][level] ?? '';
}
