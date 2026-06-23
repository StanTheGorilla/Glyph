<script lang="ts">
  // The recording HUD — a single status orb in a 120×120 transparent window.
  //
  // States (driven by the existing `engine-event` stream):
  //   idle       grey, gentle slow breathing      (ready, not recording)
  //   recording  red, live pulse                  (mic open / listening)
  //   processing orange, calmer slow pulse         (transcribing / cleaning)
  // The window is persistent — never shown/hidden by the engine, which is
  // what keeps it from exiting fullscreen apps.
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';

  type State = 'idle' | 'recording' | 'processing';
  let state = $state<State>('idle');

  onMount(() => {
    const un = listen('engine-event', (e: any) => {
      const ev = e.payload;
      switch (ev.kind) {
        // State is driven purely by lifecycle events — partials carry only
        // transcript text and must NOT move the orb (a late/stale one would
        // otherwise flash it back to recording when nothing is being recorded).
        case 'ready': state = 'idle'; break;
        case 'recordingStarted': state = 'recording'; break;
        case 'partial': break;
        case 'stopped': state = 'processing'; break;
        // Only return to idle from processing, so a finalized from a just-ended
        // utterance can't override a new recording that started right after.
        case 'finalized': if (state === 'processing') state = 'idle'; break;
        case 'error': state = 'idle'; break;
      }
    });
    return () => { un.then((f) => f()); };
  });
</script>

<div class="orb" data-state={state} aria-label="Glyph status">
  <div class="halo"></div>
  <div class="ring"></div>
  <div class="ring-2"></div>
  <div class="core"></div>
</div>

<style>
  :global(html, body) {
    background: transparent !important;
    margin: 0;
    overflow: hidden;
    width: 100%;
    height: 100%;
  }

  .orb {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
  }

  .halo, .ring, .ring-2, .core {
    grid-area: 1 / 1;
    border-radius: 50%;
    will-change: transform, opacity;
  }

  /* Core dot — the color carrier. */
  .core {
    width: 32px;
    height: 32px;
    background: #8e939c;
    box-shadow: 0 0 0 0.5px rgba(0, 0, 0, 0.25) inset;
    transition: background 280ms cubic-bezier(0.22, 0.61, 0.36, 1),
                transform 280ms cubic-bezier(0.22, 0.61, 0.36, 1);
  }

  /* Inner ring. */
  .ring {
    width: 50px;
    height: 50px;
    border: 1px solid rgba(232, 223, 208, 0.14);
    transition: border-color 280ms cubic-bezier(0.22, 0.61, 0.36, 1),
                width 280ms cubic-bezier(0.22, 0.61, 0.36, 1),
                height 280ms cubic-bezier(0.22, 0.61, 0.36, 1);
  }
  /* Outer ring — fainter, wider. */
  .ring-2 {
    width: 68px;
    height: 68px;
    border: 1px solid rgba(232, 223, 208, 0.06);
    transition: border-color 280ms ease, width 280ms ease, height 280ms ease;
  }

  /* Soft radial glow; invisible at idle. */
  .halo {
    width: 100px;
    height: 100px;
    opacity: 0;
    transition: opacity 320ms ease, background 320ms ease;
  }

  /* ── idle: grey, slow confident breathing ── */
  .orb[data-state='idle'] .core {
    background: #8e939c;
    animation: idle-core 4s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
  .orb[data-state='idle'] .ring {
    border-color: rgba(142, 147, 156, 0.28);
    animation: idle-ring 4s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }

  /* ── recording / listening: red, live mic pulse ── */
  .orb[data-state='recording'] .core {
    background: #e5484d;
    transform: scale(1.06);
  }
  .orb[data-state='recording'] .ring {
    width: 50px; height: 50px;
    border-color: rgba(229, 72, 77, 0.55);
    animation: live-pulse 1.15s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
  .orb[data-state='recording'] .ring-2 {
    border-color: rgba(229, 72, 77, 0.16);
    animation: live-pulse-outer 1.15s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
  .orb[data-state='recording'] .halo {
    background: radial-gradient(circle, rgba(229, 72, 77, 0.5) 0%, rgba(229, 72, 77, 0) 62%);
    opacity: 1;
  }

  /* ── processing / transcribing: orange, slower, calmer ── */
  .orb[data-state='processing'] .core {
    background: #e8a23c;
    transform: scale(1.02);
  }
  .orb[data-state='processing'] .ring {
    width: 48px; height: 48px;
    border-color: rgba(232, 162, 60, 0.5);
    animation: live-pulse 1.5s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
  .orb[data-state='processing'] .ring-2 {
    border-color: rgba(232, 162, 60, 0.15);
    animation: live-pulse-outer 1.5s cubic-bezier(0.4, 0, 0.6, 1) infinite;
  }
  .orb[data-state='processing'] .halo {
    background: radial-gradient(circle, rgba(232, 162, 60, 0.44) 0%, rgba(232, 162, 60, 0) 62%);
    opacity: 1;
  }

  @keyframes idle-core {
    0%, 100% { transform: scale(1); }
    50% { transform: scale(0.92); }
  }
  @keyframes idle-ring {
    0%, 100% { opacity: 0.9; transform: scale(1); }
    50% { opacity: 0.4; transform: scale(0.95); }
  }
  @keyframes live-pulse {
    0%, 100% { transform: scale(1); opacity: 0.9; }
    50% { transform: scale(1.32); opacity: 0.3; }
  }
  @keyframes live-pulse-outer {
    0%, 100% { transform: scale(1); opacity: 0.7; }
    50% { transform: scale(1.16); opacity: 0.2; }
  }

  @media (prefers-reduced-motion: reduce) {
    .core, .ring, .ring-2, .halo { animation: none !important; }
  }
</style>
