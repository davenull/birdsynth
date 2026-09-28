// Drag-to-modulate: press on a source handle, drop on a knob. It uses
// pointer events rather than HTML drag and drop, so it works the same with
// a mouse, a pen, touch and test automation.

import { FLAG, PARAMS, PARAM_ID, type ParamKey } from '../../gen/params';
import type { ModMatrix } from '../../state/matrix';

/** The knob element under a point, if it can be modulated. */
function targetAt(x: number, y: number): HTMLElement | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  const knob = el?.closest<HTMLElement>('[data-param]') ?? null;
  const key = knob?.dataset.param as ParamKey | undefined;
  if (!key || !(key in PARAM_ID)) return null;
  return PARAMS[PARAM_ID[key]].flags & FLAG.mod ? knob : null;
}

/**
 * Start dragging `source` from a pointerdown on its handle. Dropping on a
 * modulatable knob routes the source there (reusing a slot that already
 * does). Returns the new slot through `onRouted`.
 */
export function startModDrag(e: PointerEvent, source: number, name: string, color: string, matrix: ModMatrix, onRouted?: (slot: number) => void): void {
  if (e.button !== 0) return;
  const handle = e.currentTarget as HTMLElement;
  handle.setPointerCapture(e.pointerId);
  e.preventDefault();

  const ghost = document.createElement('div');
  ghost.className = 'mod-ghost';
  ghost.textContent = name;
  document.body.appendChild(ghost);
  document.body.style.setProperty('--mod-color', color);
  document.body.classList.add('mod-dragging');
  let over: HTMLElement | null = null;
  const place = (x: number, y: number) => (ghost.style.transform = `translate(${x + 12}px, ${y + 10}px)`);
  place(e.clientX, e.clientY);

  const move = (ev: PointerEvent) => {
    place(ev.clientX, ev.clientY);
    const t = targetAt(ev.clientX, ev.clientY);
    if (t !== over) {
      over?.classList.remove('mod-over');
      over = t;
      over?.classList.add('mod-over');
    }
  };
  const end = (ev: PointerEvent, drop: boolean) => {
    handle.removeEventListener('pointermove', move);
    handle.removeEventListener('pointerup', up);
    handle.removeEventListener('pointercancel', cancel);
    if (handle.hasPointerCapture(ev.pointerId)) handle.releasePointerCapture(ev.pointerId);
    over?.classList.remove('mod-over');
    ghost.remove();
    document.body.classList.remove('mod-dragging');
    if (!drop) return;
    const t = targetAt(ev.clientX, ev.clientY);
    if (!t) return;
    const slot = matrix.add(source, PARAM_ID[t.dataset.param as ParamKey]);
    if (slot >= 0) onRouted?.(slot);
  };
  const up = (ev: PointerEvent) => end(ev, true);
  const cancel = (ev: PointerEvent) => end(ev, false);
  handle.addEventListener('pointermove', move);
  handle.addEventListener('pointerup', up);
  handle.addEventListener('pointercancel', cancel);
}
