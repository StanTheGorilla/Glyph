<script lang="ts">
  // The single place to choose a transcription model: the four Whisper sizes plus
  // Nemotron. A size you haven't downloaded isn't selectable — it shows a Download
  // button instead (with inline progress / pause / resume). Picking or finishing a
  // download applies live (no relaunch); the parent owns the invoke() calls.

  type WModel = { id: string; label: string; note: string };
  type NModel = { name: string; path: string };

  let {
    whisperModels,
    installedIds,
    activeWhisper,
    nemotronModels,
    activeNemotron,
    progress,
    onSelectWhisper,
    onDownloadWhisper,
    onSelectNemotron,
    onInstallNemotron,
    onPause,
    onCancel,
  }: {
    whisperModels: WModel[];
    installedIds: string[];
    activeWhisper: string | null;
    nemotronModels: NModel[];
    activeNemotron: string | null;
    progress: Record<string, any>;
    onSelectWhisper: (id: string) => void;
    onDownloadWhisper: (id: string) => void;
    onSelectNemotron: (path: string) => void;
    onInstallNemotron: () => void;
    onPause: (id: string, variant: string) => void;
    onCancel: (id: string, variant: string) => void;
  } = $props();

  const pct = (p: any) => (p && p.total > 0 ? Math.round((p.received / p.total) * 100) : 0);
  const prettyNemo = (n: string) => n.replace(/\.gguf$/i, '');
  // The custom-download channel is reused for the one-click Nemotron install.
  const nemoProg = $derived(progress['custom-model::transcription']);
</script>

<div class="picker">
  <span class="grp">Whisper</span>
  <div class="cards">
    {#each whisperModels as m (m.id)}
      {@const p = progress[`whisper-model::${m.id}`]}
      {@const busy = p && (p.phase === 'download' || p.phase === 'paused' || p.phase === 'error')}
      {@const installed = installedIds.includes(m.id)}
      {@const active = activeWhisper === m.id}
      {#if busy}
        <div class="card busy">
          <span class="c-label">{m.label}</span>
          {#if p.phase === 'download'}
            <span class="c-note">{pct(p)}%</span>
            <div class="bar"><div class="fill" style="width:{pct(p)}%"></div></div>
            <div class="c-act">
              <button type="button" class="xs ghost" onclick={() => onPause('whisper-model', m.id)}>Pause</button>
              <button type="button" class="xs ghost" onclick={() => onCancel('whisper-model', m.id)}>Cancel</button>
            </div>
          {:else if p.phase === 'paused'}
            <span class="c-note">Paused · {pct(p)}%</span>
            <div class="bar paused"><div class="fill" style="width:{pct(p)}%"></div></div>
            <div class="c-act">
              <button type="button" class="xs solid" onclick={() => onDownloadWhisper(m.id)}>Resume</button>
              <button type="button" class="xs ghost" onclick={() => onCancel('whisper-model', m.id)}>Cancel</button>
            </div>
          {:else}
            <span class="c-note err">Failed</span>
            <button type="button" class="xs solid" onclick={() => onDownloadWhisper(m.id)}>Retry</button>
          {/if}
        </div>
      {:else if installed}
        <button type="button" class="card sel" class:active onclick={() => !active && onSelectWhisper(m.id)} aria-pressed={active}>
          <span class="c-label">{m.label}</span>
          <span class="c-note">{active ? 'Active' : m.note}</span>
        </button>
      {:else}
        <div class="card off">
          <span class="c-label">{m.label}</span>
          <span class="c-note">{m.note}</span>
          <button type="button" class="xs download" onclick={() => onDownloadWhisper(m.id)}>Download</button>
        </div>
      {/if}
    {/each}
  </div>

  <span class="grp">Nemotron <span class="grp-dim">· streaming</span></span>
  {#if nemotronModels.length}
    <div class="cards nemo">
      {#each nemotronModels as nm (nm.path)}
        {@const active = activeNemotron === nm.path}
        <button type="button" class="card sel wide" class:active onclick={() => !active && onSelectNemotron(nm.path)} aria-pressed={active}>
          <span class="c-label">{prettyNemo(nm.name)}</span>
          <span class="c-note">{active ? 'Active' : 'Select'}</span>
        </button>
      {/each}
    </div>
  {:else}
    <div class="card off wide">
      <span class="c-label">Nemotron 0.6B</span>
      {#if nemoProg?.phase === 'download'}
        <span class="c-note">{pct(nemoProg)}%</span>
        <div class="bar"><div class="fill" style="width:{pct(nemoProg)}%"></div></div>
        <div class="c-act">
          <button type="button" class="xs ghost" onclick={() => onPause('custom-model', 'transcription')}>Pause</button>
          <button type="button" class="xs ghost" onclick={() => onCancel('custom-model', 'transcription')}>Cancel</button>
        </div>
      {:else if nemoProg?.phase === 'paused'}
        <span class="c-note">Paused · {pct(nemoProg)}%</span>
        <div class="bar paused"><div class="fill" style="width:{pct(nemoProg)}%"></div></div>
        <div class="c-act">
          <button type="button" class="xs solid" onclick={onInstallNemotron}>Resume</button>
          <button type="button" class="xs ghost" onclick={() => onCancel('custom-model', 'transcription')}>Cancel</button>
        </div>
      {:else}
        <span class="c-note">streaming · q8_0 · ~940 MB{nemoProg?.phase === 'error' ? ' · failed' : ''}</span>
        <button type="button" class="xs download" onclick={onInstallNemotron}>Install</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .picker { display: flex; flex-direction: column; gap: 9px; }
  .grp {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--faint-2);
    margin-top: 4px;
  }
  .grp-dim { color: var(--faint-2); font-weight: 500; text-transform: none; letter-spacing: 0; }

  .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(0, 1fr)); gap: 8px; }
  .cards.nemo { grid-template-columns: 1fr; }

  /* A model card: selectable (button), downloadable (off), or busy. */
  .card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-start;
    text-align: left;
    background: var(--surface-2);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    padding: 11px 12px;
    font-family: inherit;
    color: var(--dim);
    min-width: 0;
  }
  .card.wide { gap: 4px; }
  .c-label { font-size: 13px; font-weight: 600; letter-spacing: -0.005em; color: var(--text); }
  .c-note { font-size: 11px; color: var(--faint); }
  .c-note.err { color: var(--rec); }

  /* Selectable (installed) */
  .card.sel { cursor: pointer; transition: border-color var(--dur) var(--ease), background var(--dur) var(--ease); }
  .card.sel:hover { border-color: var(--hairline-strong); background: var(--surface-3); }
  .card.sel.active { border-color: var(--hairline-accent); background: var(--accent-tint); cursor: default; }
  .card.sel.active:hover { background: var(--accent-tint); }
  .card.sel.active .c-note { color: var(--ok); }

  /* Not downloaded — dashed, can't be selected, only downloaded */
  .card.off { border-style: dashed; }

  .bar { width: 100%; height: 4px; border-radius: 999px; background: var(--surface-3); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); border-radius: 999px; transition: width 180ms var(--ease-out); }
  .bar.paused .fill { background: var(--faint); }

  .c-act { display: flex; gap: 6px; }
  .xs {
    font-size: 11px;
    padding: 4px 10px;
    border-radius: var(--r-md);
    font-family: inherit;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid var(--hairline);
    background: transparent;
    color: var(--dim);
    transition: color 180ms var(--ease-out), border-color 180ms var(--ease-out), background 180ms var(--ease-out);
  }
  .xs.ghost:hover { color: var(--text); border-color: var(--hairline-strong); }
  .xs.download { color: var(--accent); border-color: color-mix(in srgb, var(--accent) 40%, transparent); }
  .xs.download:hover { background: var(--accent-tint); border-color: var(--accent); }
  .xs.solid { background: var(--accent); color: #18200f; border-color: transparent; font-weight: 600; }
  .xs.solid:hover { background: var(--accent-soft); }
</style>
