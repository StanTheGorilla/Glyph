<script lang="ts">
  // Shows the models actually downloaded into the managed folder (scanned, so it
  // includes presets AND custom downloads), with the active one marked and the
  // rest switchable via "Use". Below that, the preset models you don't have yet,
  // each downloadable. Scrolls when there are many.

  type Installed = { kind: string; name: string; path: string; size: number; active: boolean };
  type Preset = { id: string; label: string; note: string; size: string; primary: string };

  let {
    kind,
    itemId,
    installed,
    presets,
    progress,
    onDownload,
    onCancel,
    onActivate,
    onDelete,
    onPause,
  }: {
    kind: 'cleanup' | 'transcription';
    itemId: string;
    installed: Installed[];
    presets: Preset[];
    progress: Record<string, any>;
    onDownload: (itemId: string, variant: string) => void;
    onCancel: (itemId: string, variant: string) => void;
    onActivate: (kind: string, path: string) => void;
    onDelete: (path: string) => void;
    onPause: (itemId: string, variant: string) => void;
  } = $props();

  // Which installed model is awaiting delete confirmation (inline, no OS dialog).
  let confirming = $state<string | null>(null);

  // Presets you don't already have (matched by their on-disk filename).
  const available = $derived(
    presets.filter((v) => !installed.some((m) => m.name === v.primary)),
  );

  function human(n: number): string {
    if (!n) return '';
    const mb = n / 1048576;
    return mb >= 1024 ? (mb / 1024).toFixed(1) + ' GB' : Math.round(mb) + ' MB';
  }
  function pretty(name: string): string {
    return name.replace(/\.(gguf|bin)$/i, '').replace(/^ggml-/, '');
  }
  const pct = (p: any) => (p && p.total > 0 ? Math.round((p.received / p.total) * 100) : 0);
</script>

<div class="msec">
  <span class="msub">Installed</span>
  {#if installed.length === 0}
    <p class="empty">Nothing downloaded yet — grab one below.</p>
  {:else}
    <div class="mlist scroll">
      {#each installed as m (m.path)}
        <div class="mrow" class:active={m.active}>
          <span class="mname" title={m.path}>{pretty(m.name)}</span>
          <span class="msize">{human(m.size)}</span>
          {#if m.active}
            <span class="badge ok">Active</span>
          {:else if confirming === m.path}
            <span class="confirm-q">Delete?</span>
            <button type="button" class="ghost xs danger" onclick={() => { onDelete(m.path); confirming = null; }}>Delete</button>
            <button type="button" class="ghost xs" onclick={() => (confirming = null)}>Cancel</button>
          {:else}
            <button type="button" class="primary xs" onclick={() => onActivate(kind, m.path)}>Use</button>
            <button type="button" class="trash" title="Delete model" aria-label="Delete model" onclick={() => (confirming = m.path)}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M3 6h18"/>
                <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>
                <line x1="10" y1="11" x2="10" y2="17"/>
                <line x1="14" y1="11" x2="14" y2="17"/>
              </svg>
            </button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  {#if available.length}
    <span class="msub">Download</span>
    <div class="mlist">
      {#each available as v (v.id)}
        {@const p = progress[`${itemId}::${v.id}`]}
        {@const busy = p?.phase === 'download' || p?.phase === 'extract'}
        <div class="mrow">
          <div class="mmeta">
            <span class="mname">{v.label}</span>
            <span class="mnote">{v.note}</span>
          </div>
          <span class="msize">{v.size}</span>
          {#if busy}
            <span class="pl">{p.phase === 'extract' ? 'Extracting…' : pct(p) + '%'}</span>
            {#if p.phase === 'download'}
              <button type="button" class="ghost xs" onclick={() => onPause(itemId, v.id)}>Pause</button>
            {/if}
            <button type="button" class="ghost xs" onclick={() => onCancel(itemId, v.id)}>Cancel</button>
          {:else if p?.phase === 'paused'}
            <span class="pl">Paused · {pct(p)}%</span>
            <button type="button" class="primary xs" onclick={() => onDownload(itemId, v.id)}>Resume</button>
            <button type="button" class="ghost xs" onclick={() => onCancel(itemId, v.id)}>Cancel</button>
          {:else}
            <button type="button" class="ghost xs" onclick={() => onDownload(itemId, v.id)}>Download</button>
          {/if}
        </div>
        {#if busy || p?.phase === 'paused'}
          <div class="bar" class:paused={p?.phase === 'paused'}><div class="fill" style="width:{pct(p)}%"></div></div>
        {/if}
        {#if p?.phase === 'error'}
          <span class="err">{p.message ?? 'Download failed'}</span>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .msec { display: flex; flex-direction: column; gap: 8px; }
  .msub {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--faint-2);
    margin-top: 6px;
  }
  .empty { margin: 0; font-size: 12.5px; color: var(--faint); }

  .mlist { display: flex; flex-direction: column; gap: 6px; }
  .mlist.scroll { max-height: 240px; overflow-y: auto; padding-right: 2px; }

  .mrow {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 12px;
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    background: var(--surface-2);
  }
  .mrow.active { border-color: var(--hairline-accent); background: var(--accent-tint); }
  .mmeta { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .mname {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mnote { font-size: 11px; color: var(--faint); }
  .msize { margin-left: auto; font-family: var(--font-mono); font-size: 11px; color: var(--faint); white-space: nowrap; }
  .pl { font-family: var(--font-mono); font-size: 11.5px; color: var(--dim); }
  .err { font-size: 11.5px; color: var(--rec); }

  .badge { font-size: 10.5px; font-weight: 600; border-radius: var(--r-xs); padding: 2px 8px; letter-spacing: 0.02em; }
  .badge.ok { color: var(--ok); border: 1px solid color-mix(in srgb, var(--ok) 35%, transparent); }

  .bar { height: 4px; border-radius: 999px; background: var(--surface-3); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); border-radius: 999px; transition: width 180ms var(--ease-out); }
  .bar.paused .fill { background: var(--faint); }

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
  .ghost.danger { color: var(--rec); border-color: color-mix(in srgb, var(--rec) 35%, transparent); }
  .ghost.danger:hover { color: var(--rec); border-color: var(--rec); }
  .confirm-q { font-size: 11.5px; color: var(--faint); flex: none; }

  /* trash / delete a downloaded model */
  .trash {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    color: var(--faint-2);
    cursor: pointer;
    padding: 5px 7px;
    flex: none;
    transition: color 180ms var(--ease-out), border-color 180ms var(--ease-out);
  }
  .trash:hover { color: var(--rec); border-color: color-mix(in srgb, var(--rec) 40%, transparent); }
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
  .xs { font-size: 11.5px; padding: 5px 11px; flex: none; }
</style>
