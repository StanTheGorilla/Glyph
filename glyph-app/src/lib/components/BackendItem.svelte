<script lang="ts">
  // One catalog item, rendered as a list of independently-installable variant
  // rows (e.g. CPU / Vulkan / CUDA, or model A / model B). Each row downloads on
  // its own — you can start several at once and they run in parallel. For items
  // that set a config field (config_target != 'none'), an installed-but-inactive
  // variant gets a "Use" button to make it the active one.

  type Variant = { id: string; label: string; note: string; url: string; size: string };
  type Item = { id: string; name: string; description: string; config_target: string; variants: Variant[] };
  type Status = { installed_variants: string[]; active_variant: string | null } | undefined;

  let {
    item,
    status,
    progress,
    onInstall,
    onCancel,
    onActivate,
    onPause,
  }: {
    item: Item;
    status: Status;
    // The full progress map, keyed `${itemId}::${variantId}`.
    progress: Record<string, any>;
    onInstall: (id: string, variant: string) => void;
    onCancel: (id: string, variant: string) => void;
    onActivate: (id: string, variant: string) => void;
    onPause: (id: string, variant: string) => void;
  } = $props();

  const installed = (v: string) => (status?.installed_variants ?? []).includes(v);
  const isActive = (v: string) => status?.active_variant === v;
  const usesConfig = item.config_target !== 'none';
  const prog = (v: string) => progress[`${item.id}::${v}`];
  const pct = (p: any) => (p && p.total > 0 ? Math.round((p.received / p.total) * 100) : 0);
</script>

<div class="item">
  <div class="variants">
    {#each item.variants as v (v.id)}
      {@const p = prog(v.id)}
      {@const busy = p?.phase === 'download' || p?.phase === 'extract'}
      <div class="variant" class:active={isActive(v.id)}>
        <div class="vtop">
          <div class="vmeta">
            <span class="vlabel">{v.label}</span>
            <span class="vnote">{v.note}</span>
          </div>
          <span class="vsize">{v.size}</span>
          <div class="vaction">
            {#if busy}
              <span class="pl">{p.phase === 'extract' ? 'Extracting…' : pct(p) + '%'}</span>
              {#if p.phase === 'download'}
                <button type="button" class="ghost xs" onclick={() => onPause(item.id, v.id)}>Pause</button>
              {/if}
              <button type="button" class="ghost xs" onclick={() => onCancel(item.id, v.id)}>Cancel</button>
            {:else if p?.phase === 'paused'}
              <span class="pl">Paused · {pct(p)}%</span>
              <button type="button" class="primary xs" onclick={() => onInstall(item.id, v.id)}>Resume</button>
              <button type="button" class="ghost xs" onclick={() => onCancel(item.id, v.id)}>Cancel</button>
            {:else if !v.url}
              <span class="soon">Coming soon</span>
            {:else if isActive(v.id)}
              <span class="badge ok">Active</span>
            {:else if installed(v.id)}
              {#if usesConfig}
                <button type="button" class="primary xs" onclick={() => onActivate(item.id, v.id)}>Use</button>
              {:else}
                <span class="badge ok">Installed</span>
              {/if}
            {:else}
              <button type="button" class="primary xs" onclick={() => onInstall(item.id, v.id)}>Download</button>
            {/if}
          </div>
        </div>
        {#if busy || p?.phase === 'paused'}
          <div class="bar" class:indeterminate={p.phase === 'extract'} class:paused={p.phase === 'paused'}>
            <div class="fill" style="width:{p.phase === 'extract' ? 100 : pct(p)}%"></div>
          </div>
        {/if}
        {#if p?.phase === 'error'}
          <span class="err">{p.message ?? 'Download failed'}</span>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .item { display: flex; flex-direction: column; }
  .variants { display: flex; flex-direction: column; gap: 8px; }
  .variant {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding: 10px 12px;
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    background: var(--surface-2);
  }
  .variant.active { border-color: var(--hairline-accent); background: var(--accent-tint); }
  .vtop { display: flex; align-items: center; gap: 12px; }
  .vmeta { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .vlabel { font-size: 13px; font-weight: 600; color: var(--text); }
  .vnote { font-size: 11px; color: var(--faint); }
  .vsize { margin-left: auto; font-family: var(--font-mono); font-size: 11px; color: var(--faint); white-space: nowrap; }
  .vaction { display: flex; align-items: center; gap: 8px; flex: none; }
  .pl { font-family: var(--font-mono); font-size: 11.5px; color: var(--dim); }
  .soon { font-size: 11.5px; color: var(--faint-2); font-style: italic; }
  .err { font-size: 11.5px; color: var(--rec); }

  .badge {
    font-size: 10.5px;
    font-weight: 600;
    border-radius: var(--r-xs);
    padding: 2px 7px;
    letter-spacing: 0.02em;
  }
  .badge.ok { color: var(--ok); border: 1px solid color-mix(in srgb, var(--ok) 35%, transparent); }

  .bar { height: 4px; border-radius: 999px; background: var(--surface-3); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); border-radius: 999px; transition: width 180ms var(--ease-out); }
  .bar.indeterminate .fill { animation: pulse 1.1s ease-in-out infinite; }
  .bar.paused .fill { background: var(--faint); }
  @keyframes pulse { 0%, 100% { opacity: 0.45; } 50% { opacity: 1; } }

  .ghost {
    background: transparent;
    color: var(--dim);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    font-family: inherit;
    font-weight: 500;
    cursor: pointer;
    transition: color 200ms var(--ease-out), border-color 200ms var(--ease-out);
  }
  .ghost:hover { color: var(--text); border-color: var(--hairline-strong); }
  .primary {
    background: var(--accent);
    color: #18200f;
    border: 0;
    border-radius: var(--r-md);
    font-family: inherit;
    font-weight: 600;
    cursor: pointer;
    transition: background 200ms var(--ease-out);
  }
  .primary:hover { background: var(--accent-soft); }
  .xs { font-size: 11.5px; padding: 5px 11px; }
</style>
