// Computer-keyboard playing, laid out like a piano across two rows:
//
//    W E   T Y U   O P        black keys
//   A S D F G H J K L ; '     white keys (A = C)
//
// Z / X shift the octave down / up; C / V lower / raise the velocity.
// Keys are matched by physical position (KeyboardEvent.code), so the layout
// works the same on any keyboard language.

const NOTES: Record<string, number> = {
  KeyA: 0, KeyW: 1, KeyS: 2, KeyE: 3, KeyD: 4, KeyF: 5, KeyT: 6, KeyG: 7, KeyY: 8, KeyH: 9, KeyU: 10, KeyJ: 11,
  KeyK: 12, KeyO: 13, KeyL: 14, KeyP: 15, Semicolon: 16, Quote: 17,
};

export interface QwertyTarget {
  noteOn(note: number, velocity: number): void;
  noteOff(note: number): void;
}

export interface QwertyState {
  /** Octave of the A key's C, where C4 = MIDI 60 (middle C). */
  octave: number;
  velocity: number;
}

function typing(e: KeyboardEvent): boolean {
  const t = e.target as HTMLElement | null;
  return !!t && (t.isContentEditable || t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT');
}

/** Start listening; returns a function that stops and releases held keys. */
export function installQwerty(target: QwertyTarget, onChange?: (s: QwertyState) => void): () => void {
  const state: QwertyState = { octave: 4, velocity: 0.8 };
  const held = new Map<string, number>(); // key code -> note it started

  const down = (e: KeyboardEvent) => {
    if (e.repeat || e.metaKey || e.ctrlKey || e.altKey || typing(e)) return;
    const off = NOTES[e.code];
    if (off !== undefined) {
      if (held.has(e.code)) return;
      const note = 12 * (state.octave + 1) + off;
      if (note < 0 || note > 127) return;
      held.set(e.code, note);
      target.noteOn(note, state.velocity);
      e.preventDefault();
      return;
    }
    switch (e.code) {
      case 'KeyZ':
        state.octave = Math.max(-1, state.octave - 1);
        break;
      case 'KeyX':
        state.octave = Math.min(8, state.octave + 1);
        break;
      case 'KeyC':
        state.velocity = Math.max(0.05, Math.round((state.velocity - 0.1) * 100) / 100);
        break;
      case 'KeyV':
        state.velocity = Math.min(1, Math.round((state.velocity + 0.1) * 100) / 100);
        break;
      default:
        return;
    }
    e.preventDefault();
    onChange?.({ ...state });
  };
  const up = (e: KeyboardEvent) => {
    const note = held.get(e.code);
    if (note === undefined) return;
    held.delete(e.code);
    target.noteOff(note);
  };
  const releaseAll = () => {
    for (const note of held.values()) target.noteOff(note);
    held.clear();
  };

  window.addEventListener('keydown', down);
  window.addEventListener('keyup', up);
  window.addEventListener('blur', releaseAll);
  onChange?.({ ...state });
  return () => {
    window.removeEventListener('keydown', down);
    window.removeEventListener('keyup', up);
    window.removeEventListener('blur', releaseAll);
    releaseAll();
  };
}
