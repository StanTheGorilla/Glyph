<script lang="ts">
  // "Find more" block for a model section: search Hugging Face, paste a direct
  // link, or point at a file already on disk. Shared by the cleanup and
  // transcription sections — `kind` only tweaks copy + the progress key; the
  // parent supplies handlers already bound to the right kind.
  import ModelSearch from './ModelSearch.svelte';

  let {
    kind,
    progress,
    onPick,
    onPasteUrl,
    onChooseLocal,
    onCancel,
    onPause,
    onResume,
  }: {
    kind: 'cleanup' | 'transcription';
    progress: Record<string, any>;
    onPick: (repo: string, path: string) => void;
    onPasteUrl: (url: string) => void;
    onChooseLocal: () => void;
    onCancel: () => void;
    onPause: () => void;
    onResume: () => void;
  } = $props();

  let urlInput = $state('');

  const key = $derived(`custom-model::${kind}`);
  const p = $derived(progress[key]);
  const downloading = $derived(p?.phase === 'download');
  const paused = $derived(p?.phase === 'paused');

  const sub = $derived(
    kind === 'transcription'
      ? 'Search Hugging Face, paste a direct link, or point to a model you’ve already downloaded — handy for gated models like NeMo or parakeet.'
      : 'Search Hugging Face, paste a direct GGUF link, or use a file you already have on disk.',
  );
  const pastePlaceholder = $derived(
    kind === 'transcription' ? 'Paste a direct .gguf or .bin link' : 'Paste a direct .gguf link',
  );
  const localLabel = $derived(
    kind === 'transcription' ? 'Use a local .gguf / .bin…' : 'Use a local .gguf…',
  );

  function pct(p: any): number {
    return p && p.total > 0 ? Math.round((p.received / p.total) * 100) : 0;
  }
  function humanMB(n: number): string {
    if (!n) return '0 MB';
    const mb = n / 1048576;
    return mb >= 1024 ? (mb / 1024).toFixed(2) + ' GB' : Math.round(mb) + ' MB';
  }
  function submitUrl() {
    const u = urlInput.trim();
    if (u) {
      onPasteUrl(u);
      urlInput = '';
    }
  }
</script>

<div class="custom">
  <div class="custom-head">
    <span class="custom-title">Find more</span>
    <span class="custom-sub">{sub}</span>
  </div>

  {#if downloading || paused}
    <div class="prog">
      <div class="bar" class:paused><div class="fill" style="width:{pct(p)}%"></div></div>
      <div class="prog-row">
        <span class="prog-label">
          {paused ? 'Paused · ' : ''}{pct(p)}% · {humanMB(p?.received)} / {humanMB(p?.total)}
        </span>
        <div class="prog-actions">
          {#if paused}
            <button type="button" class="ghost xs" onclick={onResume}>Resume</button>
          {:else}
            <button type="button" class="ghost xs" onclick={onPause}>Pause</button>
          {/if}
          <button type="button" class="ghost xs" onclick={onCancel}>Cancel</button>
        </div>
      </div>
    </div>
  {:else}
    <ModelSearch {kind} {onPick} />

    <div class="byo">
      <div class="paste">
        <input
          class="mono"
          placeholder={pastePlaceholder}
          bind:value={urlInput}
          spellcheck="false"
          onkeydown={(e) => e.key === 'Enter' && submitUrl()}
        />
        <button type="button" class="ghost" onclick={submitUrl} disabled={!urlInput.trim()}>
          Download
        </button>
      </div>
      <button type="button" class="local" onclick={onChooseLocal}>{localLabel}</button>
    </div>

    {#if p?.phase === 'error'}
      <span class="err">{p.message ?? 'Download failed'}</span>
    {/if}
  {/if}
</div>

<style>
  .custom {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-top: 6px;
    padding: 18px 0 2px;
    border-top: 1px solid var(--hairline);
  }
  .custom-head { display: flex; flex-direction: column; gap: 3px; }
  .custom-title { font-size: 13.5px; font-weight: 600; color: var(--text); }
  .custom-sub { font-size: 12px; color: var(--faint); line-height: 1.5; max-width: 56ch; }

  /* bring-your-own: paste a link, or open a local file */
  .byo { display: flex; flex-direction: column; gap: 10px; }
  .paste { display: flex; gap: 8px; align-items: stretch; }
  .paste input { flex: 1; min-width: 0; }
  .paste .ghost { flex: none; }

  .local {
    align-self: flex-start;
    background: transparent;
    border: 1px dashed var(--hairline-strong);
    border-radius: var(--r-md);
    color: var(--dim);
    font-family: inherit;
    font-size: 12.5px;
    font-weight: 500;
    padding: 8px 14px;
    cursor: pointer;
    transition: color 200ms var(--ease-out), border-color 200ms var(--ease-out);
  }
  .local:hover { color: var(--text); border-color: var(--accent); }

  .ghost {
    background: transparent;
    color: var(--dim);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    padding: 8px 14px;
    font-family: inherit;
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    transition: color 200ms var(--ease-out), border-color 200ms var(--ease-out);
  }
  .ghost:hover { color: var(--text); border-color: var(--hairline-strong); }
  .ghost:disabled { opacity: 0.4; cursor: not-allowed; }
  .ghost.xs { font-size: 11.5px; padding: 5px 11px; }

  .prog { display: flex; flex-direction: column; gap: 7px; }
  .bar { height: 5px; border-radius: 999px; background: var(--surface-3); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); border-radius: 999px; transition: width 180ms var(--ease-out); }
  .bar.paused .fill { background: var(--faint); }
  .prog-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .prog-actions { display: flex; gap: 8px; flex: none; }
  .prog-label { font-family: var(--font-mono); font-size: 11.5px; color: var(--dim); }

  .err { font-size: 11.5px; color: var(--rec); }
</style>
