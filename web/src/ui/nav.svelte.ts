// Which page the faceplate shows (shared, so tours and explain mode can turn pages).

export type PageId = 'osc' | 'mix' | 'fx' | 'matrix' | 'arp' | 'clip' | 'global';

class Nav {
  page = $state<PageId>('osc');
}

export const nav = new Nav();
