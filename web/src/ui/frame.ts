// One shared requestAnimationFrame loop for everything that animates
// (scopes, meters, mod dots), so the page does a single pass per frame.

type FrameFn = (now: number) => void;

const fns = new Set<FrameFn>();
let running = false;

function tick(now: number): void {
  for (const fn of fns) fn(now);
  if (fns.size) requestAnimationFrame(tick);
  else running = false;
}

export function onFrame(fn: FrameFn): () => void {
  fns.add(fn);
  if (!running) {
    running = true;
    requestAnimationFrame(tick);
  }
  return () => fns.delete(fn);
}

/** Size a canvas's backing store for the screen's pixel density (capped at 2x). */
export function fitCanvas(c: HTMLCanvasElement): { w: number; h: number; dpr: number } {
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const w = Math.max(1, Math.round(c.clientWidth * dpr));
  const h = Math.max(1, Math.round(c.clientHeight * dpr));
  if (c.width !== w || c.height !== h) {
    c.width = w;
    c.height = h;
  }
  return { w, h, dpr };
}
