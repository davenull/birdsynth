<!--
  The transport, in the row above the keyboard on every page: play and
  stop, record (what you play goes into the clip picked on the CLIP page,
  over what's there), and where the song is. While linked, play and stop
  run everything linked (other tabs, other computers).
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import { PARAMS, PARAM_ID } from '../../gen/params';
  import { TEL } from '../../gen/protocol';
  import { toPlain } from '../../state/param-math';
  import type { Synth } from '../../synth';
  import { onFrame } from '../frame';

  const synth = getContext<Synth>('synth');
  let playing = $state(false);
  let recording = $state(false);
  let beat = $state(0);
  let linked = $state(synth.link.linked);

  onMount(() => {
    const offs = [
      onFrame(() => {
        const t = synth.host?.tel;
        if (!t) return;
        playing = t[TEL.playing] >= 0.5;
        beat = t[TEL.beat];
        recording = !!synth.recording;
      }),
      synth.link.subscribe(() => (linked = synth.link.linked)),
    ];
    return () => offs.forEach((f) => f());
  });

  const slot = () => toPlain(PARAMS[PARAM_ID['clip.slot']], synth.bank.get(PARAM_ID['clip.slot'])) - 1;
  // bar.beat, counting from 1 (in 4/4)
  const pos = $derived(playing ? `${Math.floor(beat / 4) + 1}.${(Math.floor(beat) % 4) + 1}` : '–');
</script>

<div class="transport" role="group" aria-label="Transport" data-explain="transport">
  <button
    class="play"
    class:on={playing}
    aria-pressed={playing}
    aria-label={playing ? 'Stop' : 'Play'}
    title={linked ? 'Play or stop everything linked' : 'Play or stop (clips, and the arpeggiator on the Beat retrigger)'}
    onclick={() => synth.transport(!playing)}>{playing ? '■' : '▶'}</button
  >
  <button
    class="rec"
    class:on={recording}
    aria-pressed={recording}
    aria-label="Record into the current clip"
    title="Record what you play into the clip picked on the CLIP page (over what's there)"
    onclick={() => (synth.recording ? synth.stopRecording() : synth.record(slot()))}>●</button
  >
  <span class="pos" aria-label={`Song position ${pos}`}>{pos}</span>
  {#if linked}<span class="linked" title="Linked: play, stop and the tempo run every linked birdsynth">LINK</span>{/if}
</div>

<style>
  .transport {
    display: flex;
    align-items: center;
    gap: 3px;
  }
  button {
    width: 24px;
    height: 18px;
    padding: 0;
    font: 10px var(--font-ui);
    line-height: 1;
    color: var(--text-dim);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    cursor: pointer;
  }
  .play.on {
    color: #111;
    background: #2ecc71;
    border-color: #2ecc71;
  }
  .rec {
    color: var(--clip);
  }
  .rec.on {
    color: #fff;
    background: var(--clip);
    border-color: var(--clip);
  }
  .pos {
    min-width: 34px;
    font: 10.5px var(--font-num);
    color: var(--text-dim);
    text-align: center;
  }
  .linked {
    font: 600 9px var(--font-ui);
    letter-spacing: 0.08em;
    color: #111;
    background: var(--ctl);
    border-radius: 3px;
    padding: 1px 4px;
  }
</style>
