<!--
  The app's context menu (see menu.svelte.ts). It sits outside the scaled
  faceplate so it opens exactly at the pointer, and closes on a click
  elsewhere, Escape, or scrolling.
-->
<script lang="ts">
  import { menu } from './menu.svelte';

  let el = $state<HTMLElement>();
  $effect(() => {
    if (!menu.open) return;
    queueMicrotask(() => el?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus());
    const away = (e: Event) => {
      if (!el?.contains(e.target as Node)) menu.close();
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') menu.close();
    };
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('keydown', key, true);
    window.addEventListener('wheel', away, true);
    return () => {
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('keydown', key, true);
      window.removeEventListener('wheel', away, true);
    };
  });
  // keep the menu on screen
  const left = $derived(Math.min(menu.x, window.innerWidth - 190));
  const top = $derived(Math.min(menu.y, window.innerHeight - 24 * menu.items.length - 12));
</script>

{#if menu.open}
  <div class="menu" role="menu" bind:this={el} style:left={`${left}px`} style:top={`${top}px`}>
    {#each menu.items as item (item.label)}
      <button
        role="menuitem"
        disabled={item.disabled}
        onclick={() => {
          menu.close();
          item.action();
        }}>{item.label}</button
      >
    {/each}
  </div>
{/if}

<style>
  .menu {
    position: fixed;
    z-index: 2000;
    min-width: 170px;
    padding: 4px;
    display: grid;
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 6px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.55);
  }
  button {
    font: 12px var(--font-ui);
    color: var(--text);
    text-align: left;
    background: none;
    border: 0;
    border-radius: 4px;
    padding: 5px 10px;
    cursor: pointer;
  }
  button:hover:not(:disabled),
  button:focus-visible {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    outline: none;
  }
  button:disabled {
    color: var(--text-faint);
    cursor: default;
  }
</style>
