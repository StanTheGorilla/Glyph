<script lang="ts">
  // Toggle switch with title + description. Accessible (native checkbox).
  let {
    checked = $bindable(),
    title,
    sub,
  }: { checked: boolean; title: string; sub?: string } = $props();
</script>

<label class="switch">
  <input type="checkbox" bind:checked />
  <span class="track"><span class="thumb"></span></span>
  <span class="stext">
    <span class="stitle">{title}</span>
    {#if sub}<span class="ssub">{sub}</span>{/if}
  </span>
</label>

<style>
  .switch { display: flex; align-items: flex-start; gap: 12px; cursor: pointer; user-select: none; padding: 2px 0; }
  .switch input { position: absolute; opacity: 0; pointer-events: none; }
  .track {
    width: 38px; height: 22px; border-radius: 999px;
    background: var(--surface-3);
    border: 1px solid var(--hairline-strong);
    position: relative; flex: none;
    transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease);
    margin-top: 1px;
  }
  .thumb {
    position: absolute; top: 2px; left: 2px;
    width: 16px; height: 16px; border-radius: 50%;
    background: var(--dim);
    transition: transform var(--dur) var(--ease), background var(--dur) var(--ease);
  }
  .switch input:checked + .track {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch input:checked + .track .thumb {
    transform: translateX(16px);
    background: #fff;
  }
  .switch input:focus-visible + .track {
    box-shadow: 0 0 0 3px var(--accent-tint-2);
  }
  .stext { display: flex; flex-direction: column; gap: 2px; }
  .stitle { font-size: 13.5px; color: var(--text); line-height: 1.3; }
  .ssub { font-size: 11.5px; color: var(--faint); line-height: 1.4; }
</style>
