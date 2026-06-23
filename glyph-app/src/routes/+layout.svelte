<script lang="ts">
  // App shell — design tokens + base typography.
  // All fonts self-hosted via @fontsource (no network at runtime).
  import '@fontsource-variable/geist';
  import '@fontsource-variable/geist-mono';
  import '@fontsource-variable/newsreader';
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
</script>

{@render children()}

<style>
  :global(:root) {
    /* ── Natural surfaces — coffee-tinted warm darks ── */
    --bg:        #1c1a17;   /* page: warm coffee-black           */
    --bg-soft:   #221f1b;   /* subtle elevation                  */
    --surface:   #28241f;   /* input / inset (used sparingly)    */
    --surface-2: #2f2a24;   /* hover                             */
    --surface-3: #37312a;   /* active                            */

    /* ── Hairlines (warm, low-alpha) ── */
    --hairline:        rgba(232, 223, 208, 0.07);
    --hairline-strong: rgba(232, 223, 208, 0.13);
    --hairline-accent: rgba(146, 184, 123, 0.40);

    /* ── Text — warm off-white → warm taupe ── */
    --text:     #ece4d4;   /* primary                           */
    --dim:      #b5ab98;   /* secondary                         */
    --faint:    #8e8573;   /* tertiary                          */
    --faint-2:  #645c4f;   /* quietest                          */

    /* ── Accent: sage / botanical green (the signature) ── */
    --accent:        #92b87b;   /* sage                        */
    --accent-soft:   #7fa569;   /* hover / pressed             */
    --accent-tint:   rgba(146, 184, 123, 0.13);
    --accent-tint-2: rgba(146, 184, 123, 0.22);

    /* ── Functional (calm, earthy) ── */
    --rec:    #d96651;     /* recording — terracotta            */
    --rec-tint: rgba(217, 102, 81, 0.15);
    --amber:  #d9a656;     /* processing — honey                */
    --amber-tint: rgba(217, 166, 86, 0.15);
    --ok:     #92b87b;     /* success — same as accent          */
    --ok-tint: rgba(146, 184, 123, 0.13);

    /* ── Radii ── */
    --r-xs: 4px;
    --r-sm: 6px;
    --r-md: 8px;
    --r-lg: 12px;

    /* ── Type ── */
    --font:       'Geist Variable', ui-sans-serif, system-ui, -apple-system, sans-serif;
    --font-mono:  'Geist Mono Variable', ui-monospace, 'Cascadia Code', Consolas, monospace;
    --font-serif: 'Newsreader Variable', ui-serif, Georgia, serif;

    /* ── Motion ── */
    --ease:    cubic-bezier(0.22, 0.61, 0.36, 1);
    --ease-out:cubic-bezier(0.16, 1, 0.3, 1);
    --dur:     150ms;

    /* ── Layout ── */
    --shell-w: 860px;
  }

  :global(*) { box-sizing: border-box; }

  :global(html, body) {
    margin: 0;
    background: var(--bg);
    color: var(--text);
    font-family: var(--font);
    font-feature-settings: 'cv11', 'ss01';
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
    text-rendering: optimizeLegibility;
  }

  :global(::selection) {
    background: var(--accent-tint-2);
    color: var(--text);
  }

  :global(::-webkit-scrollbar) { width: 10px; height: 10px; }
  :global(::-webkit-scrollbar-track) { background: transparent; }
  :global(::-webkit-scrollbar-thumb) {
    background: var(--hairline-strong);
    border-radius: 999px;
    border: 3px solid transparent;
    background-clip: padding-box;
  }
  :global(::-webkit-scrollbar-thumb:hover) {
    background: rgba(232, 223, 208, 0.22);
    background-clip: padding-box;
    border: 3px solid transparent;
  }
</style>
