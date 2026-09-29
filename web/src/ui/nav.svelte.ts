// Which page the faceplate shows (shared, so tours and explain mode can turn
// pages). It's kept in this browser, so a visit comes back to the same page.

export type PageId = 'osc' | 'mix' | 'fx' | 'matrix' | 'arp' | 'clip' | 'global' | 'flow';

const PAGES: readonly PageId[] = ['osc', 'mix', 'fx', 'matrix', 'arp', 'clip', 'global', 'flow'];
const KEY = 'birdsynth.page';

function stored(): PageId {
  try {
    const p = typeof localStorage === 'undefined' ? null : localStorage.getItem(KEY);
    return PAGES.includes(p as PageId) ? (p as PageId) : 'osc';
  } catch {
    return 'osc';
  }
}

class Nav {
  page = $state<PageId>(stored());
}

export const nav = new Nav();

$effect.root(() => {
  $effect(() => {
    try {
      localStorage.setItem(KEY, nav.page);
    } catch {
      // private mode: the page isn't remembered
    }
  });
});
