<script lang="ts">
  // Search bar for finding GGUF models on Hugging Face. Type a name → see the
  // most-downloaded matches → expand one to pick a quant file → onPick downloads it.
  import { invoke } from '@tauri-apps/api/core';

  let {
    kind = 'cleanup',
    onPick,
  }: { kind?: 'cleanup' | 'transcription'; onPick: (repo: string, path: string) => void } = $props();

  let query = $state('');
  let results = $state<any[]>([]);
  let loading = $state(false);
  let err = $state('');
  let expanded = $state<string | null>(null);
  let files = $state<any[]>([]);
  let filesLoading = $state(false);
  let timer: any;

  function onInput() {
    clearTimeout(timer);
    const q = query.trim();
    if (!q) { results = []; err = ''; return; }
    timer = setTimeout(() => search(q), 350);
  }
  async function search(q: string) {
    loading = true; err = ''; expanded = null;
    try { results = await invoke('hf_search', { query: q, kind }); }
    catch (e) { err = '' + e; results = []; }
    loading = false;
  }
  async function toggle(repo: string) {
    if (expanded === repo) { expanded = null; return; }
    expanded = repo; files = []; filesLoading = true; err = '';
    try { files = await invoke('hf_gguf_files', { repo, kind }); }
    catch (e) { err = '' + e; }
    filesLoading = false;
  }
  function human(n: number): string {
    if (!n) return '';
    const mb = n / 1048576;
    return mb >= 1024 ? (mb / 1024).toFixed(1) + ' GB' : Math.round(mb) + ' MB';
  }
  function quant(path: string): string {
    return (path.split('/').pop() || path).replace(/\.(gguf|bin)$/i, '');
  }
  function compact(n: number): string {
    if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M';
    if (n >= 1e3) return (n / 1e3).toFixed(1) + 'k';
    return '' + (n ?? 0);
  }
</script>

<div class="search">
  <input
    class="sinput mono"
    placeholder={kind === 'transcription'
      ? 'Search Hugging Face — e.g. “parakeet”, “nemotron”, “whisper”…'
      : 'Search Hugging Face — e.g. “qwen3”, “gemma”, “llama”…'}
    bind:value={query}
    oninput={onInput}
    spellcheck="false"
  />

  {#if loading}<p class="hint">Searching…</p>{/if}
  {#if err}<p class="hint err">{err}</p>{/if}

  {#if results.length}
    <div class="results">
      {#each results as r (r.id)}
        <div class="result" class:open={expanded === r.id}>
          <button type="button" class="rrow" onclick={() => toggle(r.id)}>
            <span class="rid">{r.id}</span>
            <span class="rstats">↓ {compact(r.downloads)} · ♥ {compact(r.likes)}</span>
          </button>
          {#if expanded === r.id}
            <div class="files">
              {#if filesLoading}
                <span class="hint">Loading files…</span>
              {:else if files.length === 0}
                <span class="hint">No .gguf files in this repo.</span>
              {:else}
                {#each files as f (f.path)}
                  <button type="button" class="file" onclick={() => onPick(r.id, f.path)}>
                    <span class="fq">{quant(f.path)}</span>
                    <span class="fs">{human(f.size)}</span>
                  </button>
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .search { display: flex; flex-direction: column; gap: 10px; }
  .sinput { width: 100%; }
  .hint { margin: 0; font-size: 12px; color: var(--faint); }
  .hint.err { color: var(--rec); }

  .results {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    overflow: hidden;
    background: var(--surface-2);
  }
  .result { border-bottom: 1px solid var(--hairline); }
  .result:last-child { border-bottom: 0; }
  .result.open { background: var(--surface-3); }

  .rrow {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    background: transparent;
    border: 0;
    cursor: pointer;
    padding: 11px 13px;
    text-align: left;
    font-family: inherit;
    transition: background 160ms var(--ease-out);
  }
  .rrow:hover { background: var(--surface-3); }
  .rid {
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rstats { margin-left: auto; font-size: 11px; color: var(--faint); white-space: nowrap; flex: none; }

  .files { display: flex; flex-wrap: wrap; gap: 7px; padding: 4px 13px 13px; }
  .file {
    display: inline-flex;
    align-items: baseline;
    gap: 7px;
    background: var(--bg-soft);
    border: 1px solid var(--hairline);
    border-radius: var(--r-sm, 8px);
    padding: 6px 10px;
    cursor: pointer;
    font-family: inherit;
    transition: border-color 160ms var(--ease-out), color 160ms var(--ease-out);
  }
  .file:hover { border-color: var(--hairline-accent); color: var(--text); }
  .fq { font-family: var(--font-mono); font-size: 11.5px; color: var(--text); }
  .fs { font-size: 10.5px; color: var(--faint); }
</style>
