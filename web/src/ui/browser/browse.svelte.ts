// What the preset browser shows, shared with the top bar's previous/next
// buttons (which step through the same filtered list).

import { emptyQuery, search, type Entry, type Library, type Query } from '../../state/library';

export type TagMode = 'all' | 'any';

const AUTOPLAY_KEY = 'birdsynth.autoplay';
const readAutoplay = () => {
  try {
    return typeof localStorage !== 'undefined' && localStorage.getItem(AUTOPLAY_KEY) === '1';
  } catch {
    return false;
  }
};

class Browse {
  open = $state(false);
  /** Play each preset's preview phrase as it loads (a setting of this browser). */
  autoplay = $state(readAutoplay());
  query = $state<Query>(emptyQuery());
  /** Tags ticked to include (matched per `mode`) or excluded. */
  include = $state<string[]>([]);
  exclude = $state<string[]>([]);
  mode = $state<TagMode>('all');

  /** The query with the tag chips folded in. */
  get effective(): Query {
    const tags: Query['tags'] = {};
    for (const t of this.include) tags[t] = this.mode;
    for (const t of this.exclude) tags[t] = 'not';
    return { ...this.query, tags };
  }

  results(lib: Library | null): Entry[] {
    return lib ? search(lib.entries, this.effective) : [];
  }

  /** A chip click: off → include → exclude → off. */
  cycle(tag: string): void {
    if (this.include.includes(tag)) {
      this.include = this.include.filter((t) => t !== tag);
      this.exclude = [...this.exclude, tag];
    } else if (this.exclude.includes(tag)) {
      this.exclude = this.exclude.filter((t) => t !== tag);
    } else {
      this.include = [...this.include, tag];
    }
  }

  setAutoplay(on: boolean): void {
    this.autoplay = on;
    try {
      if (on) localStorage.setItem(AUTOPLAY_KEY, '1');
      else localStorage.removeItem(AUTOPLAY_KEY);
    } catch {
      // private mode: for this session only
    }
  }

  clear(): void {
    this.query = emptyQuery();
    this.include = [];
    this.exclude = [];
  }
}

export const browse = new Browse();
