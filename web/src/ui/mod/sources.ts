// How modulation sources look: one colour per kind of source, used on
// handles, rings, live dots and matrix rows alike.

import { SOURCES } from '../../gen/protocol';

const AUDIO: Record<string, string> = {
  'Osc A': 'var(--osc-a)',
  'Osc B': 'var(--osc-b)',
  'Osc C': 'var(--osc-c)',
  Sub: 'var(--sub)',
  Noise: 'var(--noise)',
  'Filter 1': 'var(--filter)',
  'Filter 2': 'var(--filter)',
};

export function sourceColor(s: number): string {
  const n = SOURCES[s] as string;
  if (!n || s === 0) return 'var(--text-faint)';
  if (n.startsWith('Env')) return 'var(--env)';
  if (n.startsWith('LFO')) return 'var(--lfo)';
  if (n.startsWith('Macro')) return 'var(--macro)';
  if (n.startsWith('MPE')) return 'var(--mpe)';
  if (AUDIO[n]) return AUDIO[n];
  if (n === 'Velocity' || n === 'Note' || n === 'Release Vel') return 'var(--vel)';
  if (n === 'Mod Wheel' || n === 'Pitch Bend' || n === 'Aftertouch' || n === 'Poly AT') return 'var(--ctl)';
  return 'var(--misc)';
}

/** Source groups for pickers. */
export const SOURCE_GROUPS: { name: string; sources: string[] }[] = [
  { name: 'Envelopes', sources: ['Env 1', 'Env 2', 'Env 3', 'Env 4'] },
  { name: 'LFOs', sources: SOURCES.filter((n) => n.startsWith('LFO')) },
  { name: 'Macros', sources: SOURCES.filter((n) => n.startsWith('Macro')) },
  { name: 'Note', sources: ['Velocity', 'Note', 'Release Vel', 'Rand 1', 'Rand 2', 'Rand Discrete', 'Voice Index', 'Voice Mod 1', 'Voice Mod 2', 'Active Voices', 'Fixed'] },
  { name: 'Controllers', sources: ['Mod Wheel', 'Pitch Bend', 'Aftertouch', 'Poly AT', 'MPE X', 'MPE Y', 'MPE Z'] },
  { name: 'Audio', sources: Object.keys(AUDIO) },
];
