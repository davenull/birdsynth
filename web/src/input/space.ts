// The space bar starts and stops the transport, as in a DAW, unless the focus
// wants the key: typing in a field, a button or switch reached with Tab
// (Space is how those are pressed), or an editor that uses Space itself (the
// piano roll adds a note with it, and keeps the key from coming this far).
// After a mouse click, the clicked button keeps the focus but not the key.

/** Controls that Space presses. */
const PRESSED =
  'button, summary, input[type=checkbox], input[type=radio], input[type=button], input[type=submit], input[type=reset], input[type=color], input[type=file], ' +
  '[role=button], [role=checkbox], [role=switch], [role=radio], [role=tab], [role=option], [role=menuitem], [role=menuitemcheckbox], [role=menuitemradio]';
/** Inputs typed into (where a space is a character). */
const TYPED = new Set(['text', 'search', 'email', 'number', 'password', 'tel', 'url', 'date', 'time', 'datetime-local', 'month', 'week']);

/** Whether Space pressed with this element focused is the element's rather than the transport's. */
export function spaceIsTaken(el: EventTarget | null): boolean {
  const h = el as HTMLElement | null;
  if (!h || typeof h.matches !== 'function') return false;
  if (h.isContentEditable || h.tagName === 'TEXTAREA' || h.tagName === 'SELECT') return true;
  if (h.tagName === 'INPUT' && TYPED.has((h as HTMLInputElement).type)) return true;
  return h.matches(PRESSED) && h.matches(':focus-visible');
}

/** Start listening (on `target`, the window by default); returns a function that stops. */
export function installSpace(toggle: () => void, target: Pick<EventTarget, 'addEventListener' | 'removeEventListener'> = window): () => void {
  let held = false;
  const down = (e: Event) => {
    const k = e as KeyboardEvent;
    if (k.code !== 'Space' || k.metaKey || k.ctrlKey || k.altKey || k.shiftKey || k.defaultPrevented || spaceIsTaken(k.target)) return;
    // no page scroll, and no second press of a button the mouse left focused
    k.preventDefault();
    held = true;
    if (!k.repeat) toggle();
  };
  const up = (e: Event) => {
    if ((e as KeyboardEvent).code !== 'Space' || !held) return;
    held = false;
    e.preventDefault();
  };
  target.addEventListener('keydown', down);
  target.addEventListener('keyup', up);
  return () => {
    target.removeEventListener('keydown', down);
    target.removeEventListener('keyup', up);
  };
}
