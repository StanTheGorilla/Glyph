<script lang="ts">
  // Labeled field wrapper — label + optional hint + control (passed as children).
  import type { Snippet } from 'svelte';
  let {
    label,
    hint,
    full = false,
    children,
  }: { label: string; hint?: string; full?: boolean; children: Snippet } = $props();
</script>

<div class="field" class:full>
  <span class="flabel">
    {label}
    {#if hint}<span class="fhint">{hint}</span>{/if}
  </span>
  {@render children()}
</div>

<style>
  .field { display: flex; flex-direction: column; gap: 7px; min-width: 0; }
  .field.full { grid-column: 1 / -1; }
  .flabel {
    font-size: 12px;
    font-weight: 500;
    color: var(--dim);
    letter-spacing: 0.01em;
    display: inline-flex;
    align-items: baseline;
    gap: 8px;
  }
  .fhint {
    font-weight: 400;
    font-size: 11px;
    color: var(--faint-2);
  }
</style>
