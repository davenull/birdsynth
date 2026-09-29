<!--
  The preset browser, shown in place of the page. Left: search and
  filters (source, category, rating, tags: click a tag to require it, again
  to exclude it; All/Any sets how required tags combine). Middle: the
  results; click or use the arrow keys to load. Right: the current preset's
  details, with save, export, import and delete.
-->
<script lang="ts">
  import { getContext, onMount } from 'svelte';
  import type { Synth } from '../../synth';
  import { tagCloud, type Entry, type Library } from '../../state/library';
  import { exportFile, type PatchMeta } from '../../state/patch';
  import { browse } from './browse.svelte';
  import { download, fileName, pick } from './files';
  import Knob from '../primitives/Knob.svelte';
  import type { ParamKey } from '../../gen/params';

  const synth = getContext<Synth>('synth');
  let lib = $state<Library | null>(null);
  let version = $state(0);
  let current = $state(synth.presetId);
  let meta = $state<PatchMeta>({ ...synth.meta, tags: [...synth.meta.tags] });
  let tagText = $state(synth.meta.tags.join(', '));
  let message = $state('');
  let confirmDelete = $state(false);
  let list = $state<HTMLElement>();
  let previewing = $state(false);
  let partner = $state('');
  let blend = $state(0);
  let seed = $state(1);
  /** The two parents of the hybrid now loaded (so Roll again makes another of the same two). */
  let parents = $state<[string, string] | null>(null);

  onMount(() => {
    let offLib = () => {};
    void synth.openLibrary().then((l) => {
      lib = l;
      offLib = l.subscribe(() => version++);
    });
    const offPreview = synth.onPreview(() => (previewing = synth.previewing));
    const offPatch = synth.onPatch(() => {
      current = synth.presetId;
      meta = { ...synth.meta, tags: [...synth.meta.tags] };
      tagText = meta.tags.join(', ');
      confirmDelete = false;
    });
    queueMicrotask(() => document.querySelector<HTMLInputElement>('.browser .search')?.focus());
    // Escape goes back to the synth (unless explain mode, a menu or a field's own Escape is using it)
    const onkey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || document.body.classList.contains('explaining')) return;
      browse.open = false;
    };
    window.addEventListener('keydown', onkey);
    return () => {
      offLib();
      offPatch();
      offPreview();
      synth.stopPreview();
      window.removeEventListener('keydown', onkey);
    };
  });

  // `version` bumps when the library changes (its entries aren't reactive themselves)
  const all = $derived.by(() => {
    void version;
    return lib?.entries ?? [];
  });
  const results = $derived.by(() => {
    void version;
    return browse.results(lib);
  });
  const categories = $derived.by(() => {
    const n = new Map<string, number>();
    for (const e of all) n.set(e.patch.meta.category || 'Other', (n.get(e.patch.meta.category || 'Other') ?? 0) + 1);
    return [...n];
  });
  const tags = $derived(tagCloud(all).slice(0, 36));
  const isUser = $derived(current.startsWith('user:'));

  function say(text: string): void {
    message = text;
    setTimeout(() => {
      if (message === text) message = '';
    }, 4000);
  }

  async function load(e: Entry, play = browse.autoplay): Promise<void> {
    synth.stopPreview();
    const w = await synth.loadEntry(e.id);
    parents = null;
    if (w.length) say(w.join('; '));
    if (play) synth.preview();
    queueMicrotask(() => list?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }));
  }

  async function hybrid(again = false): Promise<void> {
    const pair: [string, string] | null = again && parents ? parents : current && partner ? [current, partner] : null;
    if (!pair) return;
    seed = again ? seed + 1 : Math.floor(Math.random() * 1e6);
    synth.stopPreview();
    const w = await synth.loadHybrid(pair[0], pair[1], { seed, blend });
    parents = pair;
    say(w.length ? w.join('; ') : `A new hybrid (roll ${seed}); save it to keep it`);
    if (browse.autoplay) synth.preview();
  }

  function onlistkey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    e.stopPropagation();
    if (!results.length) return;
    const i = results.findIndex((r) => r.id === current);
    const d = e.key === 'ArrowDown' ? 1 : -1;
    void load(results[i < 0 ? 0 : Math.max(0, Math.min(results.length - 1, i + d))]);
  }

  function edit(p: Partial<PatchMeta>): void {
    synth.setMeta(p);
  }

  async function rate(e: Entry, stars: number): Promise<void> {
    const rating = e.patch.meta.rating === stars ? 0 : stars;
    await lib?.setMeta(e.id, { rating });
    if (e.id === current) edit({ rating });
  }

  async function save(asNew: boolean): Promise<void> {
    edit({ tags: tagText.split(',').map((t) => t.trim().toLowerCase()).filter(Boolean) });
    const e = await synth.saveToLibrary(synth.meta, asNew || !isUser);
    say(`Saved "${e.patch.meta.name}" to your library`);
  }

  async function exportCurrent(): Promise<void> {
    const { patch, assets } = await synth.savePatch();
    download(fileName(patch.meta.name), exportFile(patch, assets));
  }

  async function importFiles(): Promise<void> {
    const files = await pick('.json,application/json', true);
    let last: Entry | null = null;
    for (const f of files) {
      try {
        last = (await lib?.importText(await f.text())) ?? null;
      } catch (err) {
        say(`${f.name}: ${(err as Error).message}`);
      }
    }
    if (last) {
      await load(last);
      say(files.length > 1 ? `Imported ${files.length} presets` : `Imported "${last.patch.meta.name}"`);
    }
  }

  async function remove(): Promise<void> {
    if (!confirmDelete) {
      confirmDelete = true;
      return;
    }
    const name = meta.name;
    await lib?.remove(current);
    confirmDelete = false;
    synth.presetId = '';
    say(`Deleted "${name}" from your library (the sound stays loaded)`);
  }
</script>

<section class="browser panel" aria-label="Preset browser" data-explain="browser">
  <div class="filters">
    <input class="search" type="search" placeholder="Search presets" aria-label="Search presets" bind:value={browse.query.text} />
    <div class="seg" role="group" aria-label="Source">
      {#each [['all', 'All'], ['factory', 'Factory'], ['user', 'Mine']] as [v, t] (v)}
        <button class:on={browse.query.source === v} onclick={() => (browse.query.source = v as 'all')}>{t}</button>
      {/each}
    </div>
    <div class="head">Category</div>
    <div class="cats">
      <button class:on={!browse.query.category} onclick={() => (browse.query.category = '')}>Any <span>{all.length}</span></button>
      {#each categories as [c, n] (c)}
        <button class:on={browse.query.category === c} onclick={() => (browse.query.category = browse.query.category === c ? '' : c)}>{c} <span>{n}</span></button>
      {/each}
    </div>
    <div class="head">
      Tags
      <span class="seg small" role="group" aria-label="Required tags match">
        <button class:on={browse.mode === 'all'} title="Presets with every ticked tag" onclick={() => (browse.mode = 'all')}>All</button>
        <button class:on={browse.mode === 'any'} title="Presets with any ticked tag" onclick={() => (browse.mode = 'any')}>Any</button>
      </span>
    </div>
    <div class="tags">
      {#each tags as t (t.tag)}
        <button
          class="tag"
          class:inc={browse.include.includes(t.tag)}
          class:exc={browse.exclude.includes(t.tag)}
          aria-pressed={browse.include.includes(t.tag) ? 'true' : browse.exclude.includes(t.tag) ? 'mixed' : 'false'}
          title="Click: require · again: exclude · again: clear"
          onclick={() => browse.cycle(t.tag)}>{browse.exclude.includes(t.tag) ? '−' : ''}{t.tag}</button
        >
      {/each}
    </div>
    <div class="head">Rating</div>
    <div class="seg" role="group" aria-label="Minimum rating">
      {#each [0, 1, 2, 3, 4, 5] as r (r)}
        <button class:on={browse.query.minRating === r} onclick={() => (browse.query.minRating = r)}>{r ? `${r}★` : 'Any'}</button>
      {/each}
    </div>
    <button class="clear" onclick={() => browse.clear()}>Clear filters</button>
  </div>

  <div class="results">
    <div class="bar">
      <span>{results.length} preset{results.length === 1 ? '' : 's'}</span>
      <label class="auto" title="Play a short phrase with each preset as it loads"
        ><input type="checkbox" checked={browse.autoplay} onchange={(e) => browse.setAutoplay(e.currentTarget.checked)} /> Auto-play</label
      >
      <label
        >Sort
        <select bind:value={browse.query.sort} aria-label="Sort presets">
          <option value="category">Category</option>
          <option value="name">Name</option>
          <option value="rating">Rating</option>
          <option value="modified">Newest</option>
        </select></label
      >
    </div>
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="list" role="listbox" aria-label="Presets" tabindex="0" bind:this={list} onkeydown={onlistkey}>
      {#each results as e (e.id)}
        <div class="row" role="option" aria-selected={e.id === current} tabindex="-1" onclick={() => load(e)} onkeydown={() => {}}>
          <span class="stars" aria-label={`${e.patch.meta.rating} stars`}>
            {#each [1, 2, 3, 4, 5] as s (s)}
              <button
                class="star"
                class:lit={s <= e.patch.meta.rating}
                aria-label={`Rate ${e.patch.meta.name} ${s} star${s > 1 ? 's' : ''}`}
                onclick={(ev) => {
                  ev.stopPropagation();
                  void rate(e, s);
                }}>★</button
              >
            {/each}
          </span>
          <button
            class="play"
            aria-label={`Preview ${e.patch.meta.name}`}
            title="Load and play its preview"
            onclick={(ev) => {
              ev.stopPropagation();
              void load(e, true);
            }}>▶</button
          >
          <span class="pname">{e.patch.meta.name}</span>
          <span class="pcat">{e.patch.meta.category}</span>
          <span class="ptags">{e.patch.meta.tags.join(' · ')}</span>
          <span class="psrc">{e.factory ? 'factory' : 'mine'}</span>
        </div>
      {:else}
        {#if lib}
          <div class="empty">Nothing matches. <button onclick={() => browse.clear()}>Clear filters</button></div>
        {:else}
          <div class="empty" role="status">Opening your presets…</div>
        {/if}
      {/each}
    </div>
  </div>

  <div class="details">
    <div class="head">This preset {#if current.startsWith('factory:')}<span class="faint">(factory; saving makes your own copy)</span>{/if}</div>
    <label>Name <input value={meta.name} aria-label="Preset name" oninput={(e) => edit({ name: e.currentTarget.value })} /></label>
    <label>Author <input value={meta.author} aria-label="Preset author" oninput={(e) => edit({ author: e.currentTarget.value })} /></label>
    <label>Category <input value={meta.category} list="preset-categories" aria-label="Preset category" oninput={(e) => edit({ category: e.currentTarget.value })} /></label>
    <datalist id="preset-categories">
      {#each categories as [c] (c)}<option value={c}></option>{/each}
    </datalist>
    <label>Tags <input bind:value={tagText} placeholder="comma, separated" aria-label="Preset tags" onchange={() => edit({ tags: tagText.split(',').map((t) => t.trim().toLowerCase()).filter(Boolean) })} /></label>
    <label>Notes <textarea rows="2" value={meta.notes} aria-label="Preset notes" oninput={(e) => edit({ notes: e.currentTarget.value })}></textarea></label>
    <div class="head">Preview</div>
    <div class="actions">
      <button class:on={previewing} aria-pressed={previewing} onclick={() => (previewing ? synth.stopPreview() : synth.preview())}>{previewing ? '■ Stop' : '▶ Play preview'}</button>
      <span class="faint">its first clip, or a phrase for its category</span>
    </div>
    <div class="macros" data-explain="macro" aria-label="Macros">
      {#each [1, 2, 3, 4, 5, 6, 7, 8] as i (i)}<Knob param={`macro.${i}.value` as ParamKey} label={`M${i}`} size={22} color="var(--macro)" compact />{/each}
    </div>
    <div class="head">Hybridize</div>
    <div class="hybrid" data-explain="hybrid">
      <select bind:value={partner} aria-label="Hybridize with">
        <option value="">with…</option>
        {#each all.filter((e) => e.id !== current) as e (e.id)}<option value={e.id}>{e.patch.meta.name}</option>{/each}
      </select>
      <label title="How far continuous values also move toward the other preset">Blend <input type="range" min="0" max="1" step="0.05" bind:value={blend} aria-label="Hybrid blend" /></label>
      <button disabled={!partner || !current} onclick={() => hybrid()}>Hybridize</button>
      <button disabled={!parents} onclick={() => hybrid(true)} title="Another hybrid of the same two">Roll again</button>
    </div>
    <div class="actions">
      <button class="primary" onclick={() => save(false)}>{isUser ? 'Save' : 'Save copy'}</button>
      <button onclick={() => save(true)}>Save as new</button>
      <button onclick={exportCurrent} title="One file with the preset and any wavetables and impulse responses it uses">Export…</button>
      <button onclick={importFiles}>Import…</button>
      <button onclick={() => synth.loadEntry('factory:Init')}>Init</button>
      {#if isUser}
        <button class="danger" onclick={remove}>{confirmDelete ? 'Really delete?' : 'Delete'}</button>
      {/if}
    </div>
    {#if lib && !lib.durable}
      <p class="warn">This browser won't let the page store data, so presets you save last until you close the tab. Export the ones you want to keep.</p>
    {/if}
    {#if message}<p class="msg" role="status">{message}</p>{/if}
    <button class="close" onclick={() => (browse.open = false)}>Back to the synth (Esc)</button>
  </div>
</section>

<style>
  .browser {
    height: 100%;
    display: grid;
    grid-template-columns: 236px 1fr 300px;
    gap: 12px;
    padding: 10px;
  }
  .filters,
  .details {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
    overflow: auto;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font: 600 10px var(--font-ui);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-dim);
    margin-top: 6px;
  }
  .faint {
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
    color: var(--text-faint);
  }
  input,
  textarea,
  select {
    font: 12px var(--font-ui);
    color: var(--text);
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 5px 7px;
    width: 100%;
  }
  textarea {
    resize: vertical;
  }
  select {
    width: auto;
    padding: 2px 4px;
  }
  button {
    font: 11px var(--font-ui);
    color: var(--text);
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 4px;
    padding: 4px 8px;
    cursor: pointer;
  }
  button:hover {
    border-color: var(--text-faint);
  }
  .seg {
    display: flex;
    gap: 2px;
  }
  .seg button {
    flex: 1;
  }
  .seg.small button {
    padding: 1px 6px;
    font-size: 10px;
    letter-spacing: 0;
    text-transform: none;
  }
  .on {
    border-color: var(--accent);
    color: var(--accent);
  }
  .cats {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 3px;
  }
  .cats button {
    display: flex;
    justify-content: space-between;
  }
  .cats span {
    color: var(--text-faint);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
  }
  .tag {
    padding: 2px 7px;
    border-radius: 10px;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .tag.inc {
    color: #111;
    background: var(--accent);
    border-color: var(--accent);
  }
  .tag.exc {
    color: var(--clip);
    border-color: var(--clip);
    text-decoration: line-through;
  }
  .clear {
    margin-top: 8px;
    align-self: flex-start;
  }
  .results {
    display: flex;
    flex-direction: column;
    min-height: 0;
    gap: 6px;
  }
  .bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    color: var(--text-dim);
  }
  .bar label {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: var(--glass);
    border: 1px solid var(--line);
    border-radius: 4px;
    outline: none;
  }
  .list:focus-visible {
    border-color: var(--accent);
  }
  .row {
    display: grid;
    grid-template-columns: 72px 20px 1.3fr 0.6fr 1.4fr 48px;
    gap: 8px;
    align-items: center;
    padding: 4px 8px;
    font-size: 12px;
    cursor: pointer;
    border-bottom: 1px solid color-mix(in srgb, var(--line) 50%, transparent);
  }
  .row:hover {
    background: var(--panel);
  }
  .play {
    width: 20px;
    height: 20px;
    padding: 0;
    font-size: 9px;
    color: var(--text-dim);
    border-radius: 50%;
  }
  .play:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 4px;
    margin-left: auto;
    margin-right: 10px;
    color: var(--text-dim);
  }
  .auto input {
    width: auto;
  }
  .macros {
    display: grid;
    grid-template-columns: repeat(8, minmax(0, 1fr));
    justify-items: center;
    row-gap: 2px;
  }
  .hybrid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 5px;
    align-items: center;
  }
  .hybrid select {
    width: 100%;
    grid-column: 1 / -1;
  }
  .hybrid label {
    grid-column: 1 / -1;
    display: flex;
    gap: 6px;
    align-items: center;
    font-size: 11px;
    color: var(--text-dim);
  }
  .hybrid input[type='range'] {
    padding: 0;
  }
  .row[aria-selected='true'] {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .pname {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pcat,
  .ptags,
  .psrc {
    color: var(--text-dim);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .psrc {
    color: var(--text-faint);
    text-align: right;
  }
  .stars {
    display: flex;
  }
  .star {
    border: 0;
    background: none;
    padding: 0 1px;
    font-size: 12px;
    color: var(--knob-track);
  }
  .star.lit {
    color: var(--macro);
  }
  .empty {
    padding: 20px;
    color: var(--text-dim);
    text-align: center;
  }
  .details label {
    display: grid;
    gap: 2px;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 4px;
  }
  .primary {
    border-color: var(--accent);
  }
  .danger {
    color: var(--clip);
  }
  .warn {
    font-size: 11px;
    color: var(--macro);
    margin: 0;
  }
  .msg {
    font-size: 11px;
    color: var(--text-dim);
    margin: 0;
  }
  .close {
    margin-top: auto;
    align-self: flex-end;
  }
</style>
