import '@fontsource/inter/400.css';
import '@fontsource/inter/600.css';
import '@fontsource/jetbrains-mono/400.css';
import './styles/tokens.css';
import './styles/app.css';

import { mount } from 'svelte';
import App from './App.svelte';
import { Synth } from './synth';
import { installTestApi } from './test-api';

const synth = new Synth();
installTestApi(synth);
mount(App, {
  target: document.getElementById('app')!,
  context: new Map<string, unknown>([
    ['synth', synth],
    ['bank', synth.bank],
  ]),
});

// Put back the sound from the last visit, and load the engine right away.
// Most browsers keep audio suspended until the first click or key press;
// the page shows a prompt until then.
void synth.restoreSession();
synth.start().catch(() => {});
