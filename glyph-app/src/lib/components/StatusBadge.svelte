<script lang="ts">
  // Engine status — a small static dot + label. No pulse, no animation.
  // Quiet by design: it tells you the state without competing for attention.
  type DotKind = 'idle' | 'ok' | 'recording' | 'processing' | 'err';

  let {
    state,
    label,
  }: { state: DotKind; label: string } = $props();
</script>

<span class="badge" data-state={state}>
  <span class="dot" aria-hidden="true"></span>
  <span class="label">{label}</span>
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    font-weight: 500;
    color: var(--dim);
    letter-spacing: -0.005em;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: none;
    background: var(--faint-2);
    transition: background var(--dur) var(--ease);
  }

  .badge[data-state='idle'] { color: var(--dim); }
  .badge[data-state='idle'] .dot { background: var(--faint); }

  .badge[data-state='ok'] { color: var(--text); }
  .badge[data-state='ok'] .dot { background: var(--accent); }

  .badge[data-state='recording'] { color: #f0cab9; }
  .badge[data-state='recording'] .dot { background: var(--rec); }

  .badge[data-state='processing'] { color: #ecd9b0; }
  .badge[data-state='processing'] .dot { background: var(--amber); }

  .badge[data-state='err'] { color: #f0cab9; }
  .badge[data-state='err'] .dot { background: var(--rec); }
</style>
