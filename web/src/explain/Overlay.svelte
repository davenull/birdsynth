<!--
  Explain mode and tours, drawn over the faceplate (outside its scaling, in
  window coordinates). In explain mode a click on any part of the synth
  opens a callout about it instead of operating it: what it does, a live
  view where one helps, and "try this" changes you can undo. Press ? or
  the button to toggle, Escape to close.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { Synth } from '../synth';
  import Scope from '../ui/primitives/Scope.svelte';
  import { onFrame } from '../ui/frame';
  import { explain, type Try } from './content';
  import { explainMode } from './explain.svelte';
  import Spectrum from './Spectrum.svelte';
  import { tours } from './tour.svelte';
  import { TOURS } from './tours';

  const synth = getContext<Synth>('synth');
  const ex = $derived(explainMode.key ? explain(explainMode.key) : null);
  let tried = $state('');
  let holding = $state(false);
  let target = $state<{ x: number; y: number; w: number; h: number } | null>(null);

  function typing(e: KeyboardEvent): boolean {
    const t = e.target as HTMLElement | null;
    return !!t && (t.isContentEditable || t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT');
  }

  function release(): void {
    if (holding) synth.noteOff(60);
    holding = false;
  }

  onMount(() => {
    const inUi = (t: EventTarget | null) => t instanceof Element && !!t.closest('.explain-ui');
    const grab = (e: Event) => {
      if (!explainMode.on || inUi(e.target)) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.type !== 'pointerdown') return;
      const el = (e.target as Element).closest('[data-explain]');
      tried = '';
      release();
      if (el) explainMode.show(el.getAttribute('data-explain')!, el);
      else explainMode.close();
    };
    const opts = { capture: true };
    for (const t of ['pointerdown', 'mousedown', 'click', 'dblclick', 'contextmenu']) window.addEventListener(t, grab, opts);
    const onkey = (e: KeyboardEvent) => {
      if (typing(e)) return;
      if (e.key === '?') {
        explainMode.toggle();
        release();
        e.preventDefault();
      } else if (e.key === 'Escape') {
        if (explainMode.key) explainMode.close();
        else if (explainMode.on) explainMode.toggle();
        else if (tours.tour) void tours.exit();
        release();
      }
    };
    window.addEventListener('keydown', onkey);
    // follow the tour's target as the page re-renders or resizes
    const offFrame = onFrame(() => {
      const key = tours.step?.target;
      const el = key ? document.querySelector(`[data-explain="${key}"]`) : null;
      const r = el?.getBoundingClientRect();
      const next = r ? { x: r.left, y: r.top, w: r.width, h: r.height } : null;
      if (next?.x !== target?.x || next?.y !== target?.y || next?.w !== target?.w || next?.h !== target?.h) target = next;
    });
    return () => {
      for (const t of ['pointerdown', 'mousedown', 'click', 'dblclick', 'contextmenu']) window.removeEventListener(t, grab, opts);
      window.removeEventListener('keydown', onkey);
      offFrame();
      release();
    };
  });

  $effect(() => {
    document.body.classList.toggle('explaining', explainMode.on);
  });

  async function run(t: Try): Promise<void> {
    await t.run(synth);
    tried = t.label;
  }

  function hold(): void {
    if (holding) release();
    else {
      synth.noteOn(60, 0.8);
      holding = true;
    }
  }

  // the callout sits beside the part it explains, kept on screen
  const W = 340;
  const pos = $derived.by(() => {
    const r = explainMode.rect;
    if (!r) return { left: 0, top: 0 };
    const right = r.x + r.w + 12;
    const left = right + W < window.innerWidth ? right : Math.max(8, r.x - W - 12);
    const top = Math.max(8, Math.min(r.y, window.innerHeight - 420));
    return { left, top };
  });
</script>

{#if explainMode.on}
  <div class="banner explain-ui" role="status">
    <span><b>Explain mode</b>: click any part of the synth to see what it does.</span>
    <span class="tours">
      Tour:
      {#each TOURS as t (t.id)}
        <button
          title={t.summary}
          onclick={() => {
            explainMode.toggle();
            void tours.start(synth, t.id);
          }}>{t.title}</button
        >
      {/each}
    </span>
    <button onclick={() => explainMode.toggle()}>Done (Esc)</button>
  </div>
{/if}

{#if explainMode.on && explainMode.rect}
  {@const r = explainMode.rect}
  <div class="ring" style:left={`${r.x - 3}px`} style:top={`${r.y - 3}px`} style:width={`${r.w + 6}px`} style:height={`${r.h + 6}px`}></div>
  <div class="callout explain-ui" role="dialog" aria-label={ex?.title ?? 'Explanation'} style:left={`${pos.left}px`} style:top={`${pos.top}px`} style:width={`${W}px`}>
    {#if ex}
      <h3>{ex.title}</h3>
      <p>{ex.text}</p>
      {#if ex.view === 'spectrum'}
        <Spectrum {synth} tap={ex.tap ?? 'master.l'} height={110} />
      {:else if ex.view === 'scope' && ex.tap}
        <div class="scope"><Scope {synth} tap={ex.tap} label={ex.title} /></div>
      {/if}
      <div class="actions">
        {#if ex.view}<button class:on={holding} onclick={hold}>{holding ? 'Let go of the note' : 'Hold a note'}</button>{/if}
        {#each ex.tries ?? [] as t (t.label)}
          <button class="try" onclick={() => run(t)}>Try: {t.label}</button>
        {/each}
      </div>
      {#if tried}<p class="undo">Done: {tried}. Undo (Cmd/Ctrl+Z) puts it back.</p>{/if}
    {:else}
      <h3>{explainMode.key}</h3>
      <p>No notes about this part yet.</p>
    {/if}
    <button class="close" aria-label="Close" onclick={() => explainMode.close()}>×</button>
  </div>
{/if}

{#if tours.tour && tours.step}
  {@const s = tours.step}
  {#if target}
    <div class="ring tour" style:left={`${target.x - 4}px`} style:top={`${target.y - 4}px`} style:width={`${target.w + 8}px`} style:height={`${target.h + 8}px`}></div>
  {/if}
  <div class="card explain-ui" role="dialog" aria-label={`Tour: ${tours.tour.title}`}>
    <div class="count">{tours.tour.title} · {tours.index + 1} of {tours.tour.steps.length}</div>
    <h3>{s.title}</h3>
    <p>{s.text}</p>
    {#if s.view === 'spectrum'}
      <Spectrum {synth} tap={s.tap ?? 'master.l'} height={120} />
    {:else if s.view === 'scope' && s.tap}
      <div class="scope"><Scope {synth} tap={s.tap} label={s.title} /></div>
    {/if}
    <div class="nav">
      <button onclick={() => tours.exit()}>Leave tour</button>
      <span></span>
      <button disabled={tours.index === 0} onclick={() => tours.back()}>Back</button>
      <button class="primary" onclick={() => tours.next()}>{tours.index === tours.tour.steps.length - 1 ? 'Finish' : 'Next'}</button>
    </div>
  </div>
{/if}

<style>
  .banner {
    position: fixed;
    top: 8px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 1500;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 7px 12px;
    font-size: 12px;
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--accent);
    border-radius: 8px;
    box-shadow: 0 8px 30px rgba(0, 0, 0, 0.5);
    white-space: nowrap;
  }
  .tours {
    display: flex;
    gap: 4px;
    align-items: center;
    color: var(--text-dim);
  }
  .ring {
    position: fixed;
    z-index: 1400;
    pointer-events: none;
    border: 2px solid var(--accent);
    border-radius: 8px;
    box-shadow: 0 0 0 4000px rgba(0, 0, 0, 0.35);
  }
  .ring.tour {
    border-color: var(--macro);
    box-shadow: 0 0 0 4000px rgba(0, 0, 0, 0.25);
    transition:
      left 0.2s,
      top 0.2s,
      width 0.2s,
      height 0.2s;
  }
  .callout,
  .card {
    position: fixed;
    z-index: 1600;
    display: grid;
    gap: 8px;
    padding: 12px 14px;
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 8px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.6);
  }
  .card {
    right: 16px;
    bottom: 16px;
    width: 380px;
    border-color: var(--macro);
  }
  h3 {
    margin: 0;
    font-size: 14px;
    padding-right: 20px;
  }
  p {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .count {
    font: 600 10px var(--font-ui);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--macro);
  }
  .scope {
    height: 90px;
    display: grid;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  button {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 4px 8px;
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    border-color: var(--text-faint);
  }
  button:disabled {
    color: var(--text-faint);
    cursor: default;
  }
  button.on,
  .primary {
    border-color: var(--accent);
  }
  .try {
    color: var(--accent);
  }
  .undo {
    color: var(--text-faint);
    font-size: 11px;
  }
  .close {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 22px;
    height: 22px;
    padding: 0;
    font-size: 14px;
    line-height: 1;
  }
  .nav {
    display: grid;
    grid-template-columns: auto 1fr auto auto;
    gap: 4px;
  }
</style>
