// How far another instance's clock is from this one's, from ping round trips
// (NTP's way). A ping leaves at t0 (read here), is answered at t1 (read
// there) and the answer lands at t2 (here): if both legs took as long, the
// other clock read t1 at (t0 + t2) / 2 here. They rarely take exactly as
// long, but the quickest round trips are the most even, so the estimate
// averages the quickest quarter of the last few seconds' pings and moves
// toward each new value smoothly. Machines' clocks also drift apart (a few
// ms a minute at worst), which the short window follows.

/** Pings kept (at four a second: the last four seconds). */
export const CLOCK_WINDOW = 16;
/** Pings before the estimate is trusted (and taken as it is, not smoothed). */
export const CLOCK_SETTLE = 4;
const SMOOTH = 0.2;

interface Sample {
  offset: number;
  rtt: number;
}

export class ClockEstimate {
  private samples: Sample[] = [];
  private count = 0;
  /** Settled before a restart: the old estimate still stands meanwhile. */
  private held = false;
  /** The other clock minus this one (ms); null before the first pong. */
  offset: number | null = null;
  /** The quickest recent round trip (ms). */
  rtt: number | null = null;

  /** Whether enough pings have come back to go by. */
  get settled(): boolean {
    return this.held || this.count >= CLOCK_SETTLE;
  }

  add(t0: number, t1: number, t2: number): void {
    const rtt = t2 - t0;
    if (!(rtt >= 0) || !Number.isFinite(t1)) return;
    this.samples.push({ offset: t1 - (t0 + t2) / 2, rtt });
    if (this.samples.length > CLOCK_WINDOW) this.samples.shift();
    this.count++;
    const best = [...this.samples].sort((a, b) => a.rtt - b.rtt).slice(0, Math.max(1, this.samples.length >> 2));
    const est = best.reduce((s, x) => s + x.offset, 0) / best.length;
    this.rtt = best[0].rtt;
    this.offset = this.offset === null || this.count <= CLOCK_SETTLE ? est : this.offset + (est - this.offset) * SMOOTH;
  }

  /** Start over on a new path (keeping the estimate until the first new ping replaces it). */
  restart(): void {
    this.held = this.settled;
    this.samples = [];
    this.count = 0;
  }
}
