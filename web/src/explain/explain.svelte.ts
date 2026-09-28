// Explain mode: when on, clicking any part of the synth opens a callout
// about it instead of operating it.

class ExplainMode {
  on = $state(false);
  /** The part being explained, and where it is on screen. */
  key = $state<string | null>(null);
  rect = $state<{ x: number; y: number; w: number; h: number } | null>(null);

  toggle(): void {
    this.on = !this.on;
    if (!this.on) this.close();
  }

  show(key: string, el: Element): void {
    const r = el.getBoundingClientRect();
    this.key = key;
    this.rect = { x: r.left, y: r.top, w: r.width, h: r.height };
  }

  close(): void {
    this.key = null;
    this.rect = null;
  }
}

export const explainMode = new ExplainMode();
