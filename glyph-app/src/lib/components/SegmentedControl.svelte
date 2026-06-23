<script lang="ts" generics="T extends string">
  // Segmented control — mutually exclusive choice with rich labels.
  // Used for the whisper-model picker. Accessible via radiogroup semantics.
  type Option = { id: T; label: string; note?: string };

  let {
    value = $bindable(),
    options,
    disabled = [],
  }: { value: T; options: Option[]; disabled?: T[] } = $props();

  const isDisabled = (id: T) => disabled.includes(id);
</script>

<div class="seg" role="radiogroup">
  {#each options as opt}
    <button
      type="button"
      role="radio"
      aria-checked={value === opt.id}
      class="seg-btn"
      class:on={value === opt.id}
      class:off={isDisabled(opt.id)}
      disabled={isDisabled(opt.id)}
      onclick={() => !isDisabled(opt.id) && (value = opt.id)}
    >
      <span class="seg-text">
        <span class="seg-label">{opt.label}</span>
        {#if opt.note}<span class="seg-note">{isDisabled(opt.id) ? 'not downloaded' : opt.note}</span>{/if}
      </span>
    </button>
  {/each}
</div>

<style>
  .seg {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(0, 1fr));
    gap: 8px;
  }
  .seg-btn {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    text-align: left;
    background: var(--surface-2);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    padding: 11px 12px;
    cursor: pointer;
    font-family: inherit;
    color: var(--dim);
    transition: border-color var(--dur) var(--ease), background var(--dur) var(--ease), color var(--dur) var(--ease);
  }
  .seg-btn:hover { border-color: var(--hairline-strong); color: var(--text); background: var(--surface-3); }
  .seg-btn.on {
    border-color: var(--hairline-accent);
    background: var(--accent-tint);
    color: var(--text);
  }
  .seg-btn.off {
    opacity: 0.45;
    cursor: not-allowed;
    border-style: dashed;
  }
  .seg-btn.off:hover { border-color: var(--hairline); color: var(--dim); background: var(--surface-2); }
  .seg-text { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; }
  .seg-label { font-size: 13px; font-weight: 600; letter-spacing: -0.005em; }
  .seg-note { font-size: 11px; color: var(--faint); font-weight: 400; }
</style>
