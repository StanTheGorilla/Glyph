<script lang="ts">
  // Hotkey capture field. Click → press a key/combo → it's captured and shown
  // as <kbd> chips. Esc cancels. Logic preserved verbatim from the original.
  let { combo = $bindable() }: { combo: string } = $props();

  let capturing = $state(false);
  let heldMods = $state<{ token: string; code: string }[]>([]);
  let heldKeys = $state<string[]>([]);

  const MOD_GENERIC: Record<string, string> = {
    ControlLeft: 'ctrl', ControlRight: 'ctrl', ShiftLeft: 'shift', ShiftRight: 'shift',
    AltLeft: 'alt', AltRight: 'alt', MetaLeft: 'win', MetaRight: 'win',
  };
  const MOD_SIDED: Record<string, string> = {
    ControlLeft: 'lctrl', ControlRight: 'rctrl', ShiftLeft: 'lshift', ShiftRight: 'rshift',
    AltLeft: 'lalt', AltRight: 'ralt', MetaLeft: 'win', MetaRight: 'win',
  };
  const MOD_ORDER = ['ctrl', 'shift', 'alt', 'win'];
  const isMod = (code: string) => code in MOD_GENERIC;

  const TOKEN_LABEL: Record<string, string> = {
    ctrl: 'Ctrl', shift: 'Shift', alt: 'Alt', win: 'Win',
    lctrl: 'LCtrl', rctrl: 'RCtrl', lshift: 'LShift', rshift: 'RShift',
    lalt: 'LAlt', ralt: 'RAlt',
    space: 'Space', tab: 'Tab', enter: 'Enter', caps: 'Caps',
  };

  function keyToken(e: KeyboardEvent): string | null {
    const c = e.code;
    if (c in MOD_GENERIC) return MOD_GENERIC[c];
    if (c === 'Space') return 'space';
    if (c === 'Tab') return 'tab';
    if (c === 'Enter' || c === 'NumpadEnter') return 'enter';
    if (c === 'CapsLock') return 'caps';
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(c)) return c.toLowerCase();
    if (/^Key[A-Z]$/.test(c)) return c.slice(3).toLowerCase();
    if (/^Digit[0-9]$/.test(c)) return c.slice(5);
    return null;
  }

  function comboString(): string {
    if (heldKeys.length === 0 && heldMods.length === 1) {
      return MOD_SIDED[heldMods[0].code] ?? heldMods[0].token;
    }
    const mods = [...new Set(heldMods.map((m) => m.token))].sort(
      (a, b) => MOD_ORDER.indexOf(a) - MOD_ORDER.indexOf(b),
    );
    return [...mods, ...heldKeys].join('+');
  }

  function pretty(t: string): string {
    return TOKEN_LABEL[t] ?? t.toUpperCase();
  }
  const chips = $derived((combo || '').split('+').filter(Boolean));

  function onCaptureKeyDown(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.code === 'Escape') { endCapture(); return; }
    const tok = keyToken(e);
    if (!tok) return;
    if (isMod(e.code)) {
      if (!heldMods.some((m) => m.code === e.code)) heldMods = [...heldMods, { token: tok, code: e.code }];
    } else {
      if (!heldKeys.includes(tok)) heldKeys = [...heldKeys, tok];
      combo = comboString();
      endCapture();
    }
  }
  function onCaptureKeyUp(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (capturing && heldKeys.length === 0 && heldMods.length > 0) {
      combo = comboString();
      endCapture();
    }
  }
  function startCapture() {
    capturing = true;
    heldMods = [];
    heldKeys = [];
    window.addEventListener('keydown', onCaptureKeyDown, true);
    window.addEventListener('keyup', onCaptureKeyUp, true);
  }
  function endCapture() {
    capturing = false;
    window.removeEventListener('keydown', onCaptureKeyDown, true);
    window.removeEventListener('keyup', onCaptureKeyUp, true);
  }
</script>

<button
  type="button"
  class="hk"
  class:capturing
  onclick={() => (capturing ? endCapture() : startCapture())}
>
  {#if capturing}
    <span class="hk-live">
      <span class="hk-capturing">{heldMods.length || heldKeys.length ? comboString() : 'Listening…'}</span>
    </span>
  {:else if chips.length}
    <span class="chips">
      {#each chips as t, i}
        {#if i > 0}<span class="plus">+</span>{/if}
        <kbd>{pretty(t)}</kbd>
      {/each}
    </span>
  {:else}
    <span class="hk-empty">Click to set hotkey</span>
  {/if}
</button>

<style>
  /* Match the page's other form controls: transparent fill, hairline border,
     same padding/height, accent border while capturing. */
  .hk {
    width: 100%;
    text-align: left;
    display: flex;
    align-items: center;
    background: transparent;
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    padding: 10px 12px;
    font-family: inherit;
    font-size: 13.5px;
    cursor: pointer;
    min-height: 40px;
    transition: border-color 220ms var(--ease-out), background 220ms var(--ease-out);
  }
  .hk:hover { border-color: var(--hairline-strong); }
  .hk.capturing { border-color: var(--accent); background: var(--bg-soft); }
  .chips { display: flex; align-items: center; gap: 5px; flex-wrap: wrap; }
  /* Flat tokens, not 3D keycaps — consistent with the rest of the UI. */
  kbd {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    color: var(--dim);
    background: var(--bg-soft);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--r-xs);
    padding: 2px 7px;
    line-height: 1.5;
  }
  .plus { color: var(--faint-2); font-size: 11px; }
  .hk-live { display: inline-flex; align-items: center; color: var(--accent); }
  .hk-capturing { font-family: var(--font-mono); font-size: 12.5px; }
  .hk-empty { color: var(--faint); }
</style>
