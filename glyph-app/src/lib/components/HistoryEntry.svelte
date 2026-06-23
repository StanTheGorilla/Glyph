<script lang="ts">
  // A single dictation row. The transcript leads (primary, large), with a
  // clear meta line beneath: app · time · copy. Designed for scanability.
  let {
    entry,
    onCopy,
  }: { entry: any; onCopy: (text: string) => void } = $props();

  const rel = $derived.by(() => {
    const ts = entry.ts * 1000;
    const diff = Date.now() - ts;
    const m = Math.floor(diff / 60000);
    if (m < 1) return 'just now';
    if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60);
    if (h < 24) return `${h}h ago`;
    const d = Math.floor(h / 24);
    if (d === 1) return 'Yesterday';
    if (d < 7) return `${d}d ago`;
    return new Date(ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  });
</script>

<article class="entry">
  <p class="clean">{entry.clean}</p>

  {#if entry.raw && entry.raw !== entry.clean}
    <p class="raw">{entry.raw}</p>
  {/if}

  <div class="meta">
    <span class="app">{entry.app || 'Unknown'}</span>
    <span class="sep" aria-hidden="true">·</span>
    <span class="ts">{rel}</span>
    <button class="copy" onclick={() => onCopy(entry.clean)}>Copy</button>
  </div>
</article>

<style>
  .entry {
    padding: 22px 0;
    border-bottom: 1px solid var(--hairline);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .entry:last-child { border-bottom: 0; padding-bottom: 8px; }

  /* The transcript — the thing you're here to read. Large, primary, relaxed. */
  .clean {
    margin: 0;
    color: var(--text);
    font-size: 15px;
    line-height: 1.6;
    letter-spacing: -0.003em;
  }

  /* Raw (pre-cleanup) — clearly secondary, mono, dimmed. */
  .raw {
    margin: 0;
    color: var(--faint-2);
    font-size: 12.5px;
    font-family: var(--font-mono);
    line-height: 1.5;
  }

  /* Meta line — sits below, quiet. */
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
  }
  .app {
    font-family: var(--font-mono);
    font-weight: 500;
    color: var(--dim);
    letter-spacing: 0.01em;
  }
  .sep { color: var(--faint-2); }
  .ts { color: var(--faint); }

  .copy {
    margin-left: auto;
    background: transparent;
    border: 1px solid transparent;
    cursor: pointer;
    font-family: inherit;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--faint);
    padding: 3px 10px;
    border-radius: var(--r-sm);
    transition: color 200ms var(--ease-out), border-color 200ms var(--ease-out), background 200ms var(--ease-out);
  }
  .copy:hover {
    color: var(--accent);
    border-color: var(--hairline-accent);
    background: var(--accent-tint);
  }
</style>
