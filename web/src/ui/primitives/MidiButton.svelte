<!--
  MIDI input status, and the button that asks for access. Browsers without
  Web MIDI (Safari) show why there's none.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import type { Synth } from '../../synth';
  import type { MidiStatus } from '../../input/webmidi';

  let { synth }: { synth: Synth } = $props();
  let status = $state<MidiStatus>('off');
  let inputs = $state<string[]>([]);

  onMount(() => {
    const off = synth.midi.subscribe(() => {
      status = synth.midi.status;
      inputs = [...synth.midi.inputs];
    });
    void synth.midi.auto();
    return off;
  });

  const title = $derived(
    {
      unsupported: "This browser has no Web MIDI (Safari doesn't). Use the on-screen keyboard and wheels, or Chrome or Firefox.",
      off: 'Click to use MIDI controllers',
      asking: 'Waiting for permission…',
      denied: 'MIDI access was refused. Allow it in the site settings, then click again.',
      on: inputs.length ? `MIDI in: ${inputs.join(', ')}` : 'MIDI is on; no controller is connected',
    }[status],
  );
</script>

<button class="midi {status}" class:live={status === 'on' && inputs.length > 0} {title} aria-label={`MIDI: ${title}`} onclick={() => synth.midi.enable()} disabled={status === 'unsupported'}>
  <span class="dot"></span>MIDI{#if status === 'on'}<span class="n">{inputs.length}</span>{/if}
</button>

<style>
  .midi {
    display: flex;
    align-items: center;
    gap: 5px;
    font: 600 10px var(--font-ui);
    letter-spacing: 0.06em;
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    height: 22px;
    padding: 0 7px;
    cursor: pointer;
  }
  .midi:disabled {
    cursor: default;
    color: var(--text-faint);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-faint);
  }
  .on .dot {
    background: var(--ctl);
  }
  .live .dot {
    background: #2ecc71;
  }
  .denied .dot {
    background: var(--clip);
  }
  .n {
    font: 9px var(--font-num);
    color: var(--text-faint);
  }
</style>
