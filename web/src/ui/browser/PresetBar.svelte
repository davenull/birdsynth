<!--
  The top bar's preset controls: previous / next (through the browser's
  current results), the preset name (opens the browser; * when edited),
  save, and undo / redo with their shortcuts (Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z
  or Ctrl+Y, Cmd/Ctrl+S).
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { Synth } from '../../synth';
  import type { Library } from '../../state/library';
  import { browse } from './browse.svelte';

  const synth = getContext<Synth>('synth');
  let name = $state(synth.meta.name);
  let dirty = $state(false);
  let canUndo = $state(false);
  let canRedo = $state(false);
  let lib = $state<Library | null>(null);
  let note = $state('');

  function typing(e: KeyboardEvent): boolean {
    const t = e.target as HTMLElement | null;
    return !!t && (t.isContentEditable || t.tagName === 'INPUT' || t.tagName === 'TEXTAREA');
  }

  onMount(() => {
    const offPatch = synth.onPatch(() => (name = synth.meta.name));
    const readHistory = () => {
      dirty = synth.history.dirty;
      canUndo = synth.history.canUndo;
      canRedo = synth.history.canRedo;
    };
    readHistory();
    const offHistory = synth.history.subscribe(readHistory);
    void synth.openLibrary().then((l) => (lib = l));
    const onkey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod || typing(e)) return;
      const k = e.key.toLowerCase();
      if (k === 'z' && !e.shiftKey) synth.history.undo();
      else if ((k === 'z' && e.shiftKey) || (k === 'y' && e.ctrlKey)) synth.history.redo();
      else if (k === 's') void save();
      else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', onkey);
    return () => {
      offPatch();
      offHistory();
      window.removeEventListener('keydown', onkey);
    };
  });

  function step(d: number): void {
    const list = browse.results(lib);
    if (!list.length) return;
    const i = list.findIndex((e) => e.id === synth.presetId);
    const next = i < 0 ? (d > 0 ? 0 : list.length - 1) : (i + d + list.length) % list.length;
    void synth.loadEntry(list[next].id);
  }

  async function save(): Promise<void> {
    // factory presets are read-only: saving one makes a copy in the user library
    const e = await synth.saveToLibrary(synth.meta, !synth.presetId.startsWith('user:'));
    note = `Saved "${e.patch.meta.name}"`;
    setTimeout(() => (note = ''), 2000);
  }
</script>

<div class="presets" data-explain="presets">
  <button class="icon" aria-label="Previous preset" title="Previous preset" onclick={() => step(-1)}>‹</button>
  <button class="name" aria-label="Browse presets" aria-expanded={browse.open} title="Browse presets" onclick={() => (browse.open = !browse.open)}>
    <span class="text">{name}</span>{#if dirty}<span class="dirty" title="Edited since it was loaded or saved">*</span>{/if}
  </button>
  <button class="icon" aria-label="Next preset" title="Next preset" onclick={() => step(1)}>›</button>
  <button class="save" aria-label="Save preset" title="Save (Cmd/Ctrl+S). A factory preset is saved as a copy in your library." onclick={save}>Save</button>
  <div class="history">
    <button class="icon" aria-label="Undo" title="Undo (Cmd/Ctrl+Z)" disabled={!canUndo} onclick={() => synth.history.undo()}>↶</button>
    <button class="icon" aria-label="Redo" title="Redo (Shift+Cmd/Ctrl+Z)" disabled={!canRedo} onclick={() => synth.history.redo()}>↷</button>
  </div>
  {#if note}<span class="note" role="status">{note}</span>{/if}
</div>

<style>
  .presets {
    display: flex;
    align-items: center;
    gap: 3px;
    position: relative;
  }
  button {
    font: 12px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    height: 26px;
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    border-color: var(--text-faint);
  }
  button:disabled {
    color: var(--text-faint);
    cursor: default;
  }
  .icon {
    width: 24px;
    padding: 0;
    font-size: 15px;
    line-height: 1;
  }
  .name {
    width: 150px;
    display: flex;
    align-items: center;
    gap: 3px;
    padding: 0 10px;
    background: var(--glass);
    text-align: left;
  }
  .name .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dirty {
    color: var(--macro);
  }
  .name[aria-expanded='true'] {
    border-color: var(--accent);
  }
  .save {
    padding: 0 10px;
    font-size: 11px;
  }
  .history {
    display: flex;
    gap: 2px;
    margin-left: 6px;
  }
  .note {
    position: absolute;
    top: 30px;
    left: 30px;
    z-index: 20;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--panel-2);
    border: 1px solid var(--line);
    white-space: nowrap;
  }
</style>
