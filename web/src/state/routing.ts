// Where a source goes: its Route (by option name, so the list can change)
// and, into the filters, how Split shares it between them.

import { PARAMS, PARAM_ID, type ParamKey } from '../gen/params';

export type RouteTo = 'f1' | 'f2' | 'main' | 'direct' | 'none';

const BY_NAME: Record<string, RouteTo> = { 'Filter 1': 'f1', 'Filter 2': 'f2', Main: 'main', Direct: 'direct', None: 'none' };

/** The option index of each route (for patches written in plain values). */
export const ROUTE: Record<RouteTo, number> = { f1: 0, f2: 1, main: 2, direct: 3, none: 4 };

/** What a source's Route option means: `source` is its key ('osc.a', 'sub', 'noise'), `option` the plain value. */
export function routeTo(source: string, option: number): RouteTo {
  const c = PARAMS[PARAM_ID[`${source}.route` as ParamKey]].curve;
  return (c.kind === 'enum' && BY_NAME[c.options[option]]) || 'none';
}

/** Into the filters: the shares for Filter 1 and Filter 2 (Split sends that much to the other one). */
export function filterShares(to: 'f1' | 'f2', split: number): [number, number] {
  return to === 'f1' ? [1 - split, split] : [split, 1 - split];
}

/**
 * How a route list reads while the filters run in series: they're one chain,
 * so one choice, Filters. The source keeps its real target (it enters the
 * chain there), which the list shows again in parallel.
 */
export const IN_SERIES = { of: [ROUTE.f1, ROUTE.f2], label: 'Filters' } as const;
