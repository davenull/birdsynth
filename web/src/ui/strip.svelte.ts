// Which envelope and LFO the modulation strip shows. The strip is only on the
// OSC page, so this outlives it while other pages are open.

class Strip {
  env = $state(1);
  lfo = $state(1);
}

export const strip = new Strip();
