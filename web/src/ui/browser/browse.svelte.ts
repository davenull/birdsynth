// What the preset browser shows, shared with the top bar's previous/next
// buttons (which step through the same filtered list).

import { emptyQuery, search, type Entry, type Library, type Query } from '../../state/library';

export type TagMode = 'all' | 'any';

class Browse {
  open = $state(false);
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

  clear(): void {
    this.query = emptyQuery();
    this.include = [];
    this.exclude = [];
  }
}

export const browse = new Browse();
