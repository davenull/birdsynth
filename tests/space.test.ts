// The space bar: plays and stops, except where the focus wants the key.

import { describe, expect, it } from 'vitest';
import { installSpace, spaceIsTaken } from '../web/src/input/space';

/** Just enough of an element: its tag, type, role, and whether the keyboard brought the focus there. */
function el(tagName: string, o: { type?: string; role?: string; keyboard?: boolean; editable?: boolean } = {}) {
  const pressed = ['BUTTON', 'SUMMARY'].includes(tagName) || (tagName === 'INPUT' && ['checkbox', 'radio', 'button', 'submit', 'reset', 'color', 'file'].includes(o.type ?? '')) || ['button', 'checkbox', 'switch', 'tab', 'option'].includes(o.role ?? '');
  return {
    tagName,
    type: o.type ?? (tagName === 'INPUT' ? 'text' : undefined),
    isContentEditable: !!o.editable,
    matches: (sel: string) => (sel === ':focus-visible' ? !!o.keyboard : pressed),
  } as unknown as HTMLElement;
}

describe('space bar', () => {
  it('leaves the key to fields, and to controls reached with the keyboard', () => {
    expect(spaceIsTaken(el('INPUT'))).toBe(true);
    expect(spaceIsTaken(el('INPUT', { type: 'search' }))).toBe(true);
    expect(spaceIsTaken(el('TEXTAREA'))).toBe(true);
    expect(spaceIsTaken(el('SELECT'))).toBe(true);
    expect(spaceIsTaken(el('DIV', { editable: true }))).toBe(true);
    expect(spaceIsTaken(el('BUTTON', { keyboard: true }))).toBe(true);
    expect(spaceIsTaken(el('INPUT', { type: 'checkbox', keyboard: true }))).toBe(true);
    expect(spaceIsTaken(el('DIV', { role: 'switch', keyboard: true }))).toBe(true);
    // a button the mouse clicked, a slider, a knob, the page itself: the transport's
    expect(spaceIsTaken(el('BUTTON'))).toBe(false);
    expect(spaceIsTaken(el('INPUT', { type: 'range', keyboard: true }))).toBe(false);
    expect(spaceIsTaken(el('DIV', { role: 'slider', keyboard: true }))).toBe(false);
    expect(spaceIsTaken(el('BODY'))).toBe(false);
    expect(spaceIsTaken(null)).toBe(false);
  });

  it('toggles once per press, and not with a modifier or when something else took the key', () => {
    const target = new EventTarget();
    let toggles = 0;
    const off = installSpace(() => toggles++, target);
    const key = (type: string, o: Record<string, unknown> = {}) => {
      const e = new Event(type, { cancelable: true });
      for (const [k, v] of Object.entries({ code: 'Space', repeat: false, shiftKey: false, ctrlKey: false, metaKey: false, altKey: false, ...o })) Object.defineProperty(e, k, { value: v });
      target.dispatchEvent(e);
      return e;
    };
    const down = key('keydown');
    expect(toggles).toBe(1);
    expect(down.defaultPrevented, 'no page scroll').toBe(true);
    key('keydown', { repeat: true });
    expect(toggles, 'holding it down').toBe(1);
    expect(key('keyup').defaultPrevented, 'no press of a clicked button on release').toBe(true);
    key('keydown', { shiftKey: true });
    key('keydown', { metaKey: true });
    key('keydown', { code: 'KeyA' });
    expect(toggles).toBe(1);
    // the piano roll uses Space itself: it prevents the default before the window hears it
    const taken = new Event('keydown', { cancelable: true });
    Object.defineProperty(taken, 'code', { value: 'Space' });
    taken.preventDefault();
    target.dispatchEvent(taken);
    expect(toggles).toBe(1);
    key('keydown');
    expect(toggles).toBe(2);
    off();
    key('keydown');
    expect(toggles).toBe(2);
  });
});
