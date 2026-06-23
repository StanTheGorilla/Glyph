<script lang="ts">
  // Glyph — three calm screens, split by what you're doing:
  //   Home    everyday controls (pick model, mic, hotkey, mode, cleanup on/off)
  //   History transcripts
  //   Setup   rarely-opened: download/manage models + runtimes, behavior, words, files
  //
  // Home *picks*; Setup *manages*. The model you talk through is a dropdown of
  // what's installed; downloading/removing/importing all lives once in Setup.
  //
  // All invoke() calls, the engine-event state machine, bind targets, the
  // cfg.* shape, the hotkey-capture logic, and the save/apply flows are
  // preserved exactly — this is presentational only.
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';

  // Smooth crossfade between tabs: new panel fades/slides in slightly.
  const tabIn = { duration: 240, delay: 60, easing: cubicOut };

  import StatusBadge from '$lib/components/StatusBadge.svelte';
  import Field from '$lib/components/Field.svelte';
  import Switch from '$lib/components/Switch.svelte';
  import HotkeyCapture from '$lib/components/HotkeyCapture.svelte';
  import HistoryEntry from '$lib/components/HistoryEntry.svelte';
  import BackendItem from '$lib/components/BackendItem.svelte';
  import ModelSection from '$lib/components/ModelSection.svelte';
  import ModelCustom from '$lib/components/ModelCustom.svelte';
  import { open } from '@tauri-apps/plugin-dialog';

  type Tab = 'home' | 'history' | 'settings';
  type WhisperModel = 'small' | 'medium' | 'large' | 'turbo';

  // The one-click Nemotron install: the q8_0 streaming build (parakeet.cpp can't
  // load the f16). Lives in the same HF repo parakeet ships its GGUFs from.
  const NEMOTRON_URL =
    'https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/nemotron-3.5-asr-streaming-0.6b-q8_0.gguf';

  let tab = $state<Tab>('home');
  let cfg = $state<any>(null);
  let mics = $state<string[]>([]);
  let dictText = $state('');
  let snippetList = $state<{ cue: string; exp: string }[]>([]);
  let status = $state('');
  let statusKind = $state<'ok' | 'err'>('ok');
  let engineState = $state('Starting…');
  let engineReady = $state(false);
  let history = $state<any[]>([]);
  let advancedOpen = $state(false);
  // Which Settings sections are expanded. All collapsed by default so Settings
  // opens as a calm, scannable menu rather than a wall. txMore/clMore reveal the
  // power-user bring-your-own + engine bits inside each model section.
  let setupOpen = $state({
    transcription: false, cleanup: false, behavior: false, words: false, files: false,
    txMore: false, clMore: false,
  });
  // The built-in cleanup prompt, fetched from the backend so it's the single
  // source of truth for both the prefill and the "Reset to default" button.
  let defaultPrompt = $state('');

  // Auto-save: edits persist automatically (debounced) — no Save button. `ready`
  // gates out the initial load; `lastSnapshot` stops save()'s own writes from
  // retriggering the effect (which would loop).
  let ready = false;
  let lastSnapshot = '';
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let statusTimer: ReturnType<typeof setTimeout> | undefined;
  const snapshot = () => JSON.stringify({ c: cfg, d: dictText, s: snippetList });
  function scheduleSave() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 800);
  }
  function flashStatus(msg: string, kind: 'ok' | 'err' = 'ok') {
    status = msg;
    statusKind = kind;
    clearTimeout(statusTimer);
    statusTimer = setTimeout(() => (status = ''), kind === 'err' ? 5000 : 1800);
  }

  // ── Backends / model state ──
  let catalog = $state<any[]>([]);
  let statuses = $state<Record<string, any>>({});
  let progressMap = $state<Record<string, any>>({});
  let installedModels = $state<any[]>([]);
  let backendsDir = $state('');
  let backendMsg = $state('');
  let currentModel = $state('');

  const llamaItem = $derived(catalog.find((i) => i.id === 'llama-server'));
  const asrEngineItem = $derived(catalog.find((i) => i.id === 'asr-engine'));
  const cleanupPresets = $derived(catalog.find((i) => i.id === 'cleanup-model')?.variants ?? []);
  const whisperPresets = $derived(catalog.find((i) => i.id === 'whisper-model')?.variants ?? []);
  const cleanupInstalled = $derived(installedModels.filter((m) => m.kind === 'cleanup'));
  const transcriptionInstalled = $derived(installedModels.filter((m) => m.kind === 'transcription'));

  const WHISPER_MODELS: { id: WhisperModel; label: string; note: string }[] = [
    { id: 'small', label: 'Small', note: 'lightest' },
    { id: 'medium', label: 'Medium', note: 'balanced' },
    { id: 'large', label: 'Large', note: 'most accurate' },
    { id: 'turbo', label: 'Turbo', note: 'fast + accurate' },
  ];

  // The picker is driven by what's actually downloaded (single source of truth): a
  // size is only selectable once its .bin is on disk — otherwise it offers Download.
  const installedWhisperIds = $derived(
    whisperPresets
      .filter((v: any) => transcriptionInstalled.some((m) => m.name === v.primary))
      .map((v: any) => v.id as string),
  );
  // What the engine is currently set to use — drives the "Active" highlight.
  const activeWhisper = $derived(cfg && cfg.asr.kind === 'whisper' ? (cfg.asr.whisper.model as string) : null);
  const activeNemotronPath = $derived(cfg && cfg.asr.kind === 'nemotron' ? (cfg.asr.model as string) : null);
  // Nemotron models are arbitrary .gguf files (no fixed presets), so the picker
  // lists whatever the user has downloaded into the asr folder.
  const nemotronInstalled = $derived(transcriptionInstalled.filter((m: any) => m.name.toLowerCase().endsWith('.gguf')));

  // ── Home model dropdown ──
  // One flat list of *installed* models (Whisper sizes on disk + downloaded
  // Nemotron GGUFs). The value encodes kind so onchange routes to the right
  // activate call. Picking = switching the live engine; downloading lives in Setup.
  const homeModels = $derived([
    ...WHISPER_MODELS.filter((m) => installedWhisperIds.includes(m.id)).map((m) => ({
      value: `w:${m.id}`,
      label: `Whisper ${m.label}`,
    })),
    ...nemotronInstalled.map((m: any) => ({ value: `n:${m.path}`, label: m.name.replace(/\.gguf$/i, '') })),
  ]);
  const activeModelValue = $derived(
    activeWhisper ? `w:${activeWhisper}` : activeNemotronPath ? `n:${activeNemotronPath}` : '',
  );
  function onHomeModelChange(value: string) {
    if (value.startsWith('w:')) selectWhisper(value.slice(2));
    else if (value.startsWith('n:')) selectNemotron(value.slice(2));
  }

  // Home cleanup-model dropdown: switch among installed cleanup models live.
  const prettyModel = (name: string) => name.replace(/\.(gguf|bin)$/i, '').replace(/^ggml-/, '');
  const activeCleanupPath = $derived(cleanupInstalled.find((m: any) => m.active)?.path ?? '');

  onMount(async () => {
    await loadConfig();
    try { mics = await invoke<string[]>('list_mics'); reconcileMic(); } catch {}
    // Materialize the cleanup prompt so it's visible/editable (blank = first run).
    try { defaultPrompt = await invoke<string>('default_cleanup_prompt'); } catch {}
    if (cfg && !cfg.cleanup.prompt) cfg.cleanup.prompt = defaultPrompt;
    // Load catalog + installed models up front so Home's model dropdown knows
    // what's downloaded (single source of truth with Setup → Models).
    loadBackends();
    // An engine restart is in flight (model/engine/mic change applied with no
    // app relaunch) — show "Applying…" until the new engine reports ready.
    listen('engine-reload', () => { engineReady = false; engineState = 'Applying…'; });
    listen('engine-event', (e: any) => {
      const ev = e.payload;
      switch (ev.kind) {
        case 'ready':
          engineReady = true;
          engineState = ev.cleanup ? 'Cleanup on' : 'Cleanup off';
          break;
        case 'recordingStarted':
          engineState = 'Listening';
          break;
        case 'partial':
          engineState = 'Listening';
          break;
        case 'stopped':
          engineState = 'Transcribing';
          break;
        case 'finalized':
          engineState = ev.cleanup ? 'Cleanup on' : 'Cleanup off';
          if (tab === 'history') refreshHistory();
          break;
        case 'error':
          engineReady = true;
          engineState = 'Error';
          break;
      }
    });
    try {
      const st = await invoke<{ ready: boolean; cleanup: boolean; error: boolean }>('engine_status');
      if (st.ready) {
        engineReady = true;
        engineState = st.error ? 'Error' : st.cleanup ? 'Cleanup on' : 'Cleanup off';
      }
    } catch {}

    listen('download-progress', async (e: any) => {
      const p = e.payload;
      const key = `${p.id}::${p.variant}`;
      progressMap = { ...progressMap, [key]: p };
      if (p.phase === 'done' || p.phase === 'error') {
        refreshStatuses();
        if (p.phase === 'done') {
          backendMsg = 'Installed.';
          refreshModel();
          await refreshInstalled();
          // Picking lives on Home now: auto-select what the user just grabbed so a
          // download is one click, not download-then-pick. (Custom Nemotron
          // downloads already activate themselves; just resync to match.)
          if (p.id === 'whisper-model') selectWhisper(p.variant);
          else if (p.id === 'custom-model' && p.variant === 'transcription') resyncAsr();
        }
        // Let the bar/message linger briefly, then clear so the row resets.
        setTimeout(() => {
          const { [key]: _drop, ...rest } = progressMap;
          progressMap = rest;
        }, p.phase === 'done' ? 1800 : 6000);
      }
    });

    // Everything loaded — capture the baseline and arm auto-save.
    lastSnapshot = snapshot();
    ready = true;
  });

  async function loadConfig() {
    cfg = await invoke('get_config');
    dictText = (cfg.dictionary.terms || []).join('\n');
    snippetList = Object.entries(cfg.snippets || {}).map(([cue, exp]) => ({ cue, exp: exp as string }));
    if (!cfg.asr.kind) cfg.asr.kind = 'nemotron';
    if (!cfg.asr.whisper) {
      cfg.asr.whisper = { model: 'turbo', binary: '', device: cfg.asr.device || 'auto' };
    }
  }

  async function save() {
    cfg.dictionary.terms = dictText.split('\n').map((s: string) => s.trim()).filter(Boolean);
    const snip: Record<string, string> = {};
    for (const { cue, exp } of snippetList) if (cue.trim()) snip[cue.trim()] = exp;
    cfg.snippets = snip;
    // Capture the just-mapped state so the auto-save effect doesn't re-fire on
    // our own mutations above.
    lastSnapshot = snapshot();
    try {
      const restarted = await invoke<boolean>('save_config', { config: cfg });
      flashStatus(restarted ? 'Applying changes…' : 'Saved');
    } catch (e) {
      flashStatus('' + e, 'err');
    }
  }

  // ── Transcription picker (applies live, no relaunch) ──
  // Pick/download both write config + restart the engine via the same commands the
  // Setup "Use"/download flow uses, then we pull the new asr.* back into `cfg`
  // so Home stays in sync (without clobbering other unsaved edits).
  async function resyncAsr() {
    try {
      const c: any = await invoke('get_config');
      cfg.asr.kind = c.asr.kind;
      cfg.asr.model = c.asr.model;
      cfg.asr.whisper.model = c.asr.whisper.model;
    } catch {}
  }
  async function selectWhisper(id: string) {
    const preset = whisperPresets.find((v: any) => v.id === id);
    const m = preset && transcriptionInstalled.find((x) => x.name === preset.primary);
    if (!m) return;
    try {
      await invoke('activate_model', { path: m.path, kind: 'transcription' });
      await resyncAsr();
      await refreshInstalled();
    } catch (e) { status = '' + e; statusKind = 'err'; }
  }
  async function selectNemotron(path: string) {
    try {
      await invoke('activate_model', { path, kind: 'transcription' });
      await resyncAsr();
      await refreshInstalled();
    } catch (e) { status = '' + e; statusKind = 'err'; }
  }
  function installNemotron() { startCustom('transcription', NEMOTRON_URL); }

  async function applyMode() {
    try {
      await invoke('set_hotkey_mode', { mode: cfg.hotkey.mode });
      flashStatus(cfg.hotkey.mode === 'toggle' ? 'Click to start / stop' : 'Hold to talk');
    } catch (e) {
      flashStatus('' + e, 'err');
    }
  }

  const addSnippet = () => (snippetList = [...snippetList, { cue: '', exp: '' }]);
  const removeSnippet = (i: number) => (snippetList = snippetList.filter((_, j) => j !== i));

  async function refreshHistory() { try { history = await invoke('get_history', { limit: 100 }); } catch {} }
  async function copyEntry(t: string) { await invoke('copy_text', { text: t }); flashStatus('Copied to clipboard'); }
  async function clearHist() { await invoke('clear_history'); await refreshHistory(); }

  $effect(() => { if (tab === 'history') refreshHistory(); });

  // ── Backends ──
  async function loadBackends() {
    try {
      backendsDir = await invoke('backends_dir');
      await refreshModel();
      await refreshStatuses();
      await refreshInstalled();
      catalog = await invoke('backend_catalog');
    } catch (e) { backendMsg = '' + e; }
  }
  async function refreshModel() { try { currentModel = await invoke('cleanup_model_path'); } catch {} }
  async function refreshInstalled() { try { installedModels = await invoke('installed_models'); } catch {} }
  async function activateModel(kind: string, path: string) {
    try {
      await invoke('activate_model', { path, kind });
      await refreshInstalled();
      await refreshModel();
      // Keep Home's transcription dropdown in sync when switched from Settings.
      if (kind === 'transcription') await resyncAsr();
      backendMsg = 'Switched — applying…';
    } catch (e) { backendMsg = '' + e; }
  }
  async function deleteModel(path: string) {
    try {
      await invoke('delete_model', { path });
      await refreshInstalled();
      backendMsg = 'Model deleted.';
    } catch (e) { backendMsg = '' + e; }
  }

  // The saved mic is a name *substring* (capture matches by `contains`), but the
  // dropdown matches full names exactly — so resolve it to the full device name,
  // else the select renders blank even though a mic is selected.
  function reconcileMic() {
    const saved = (cfg?.audio?.device || '').trim();
    if (!saved || mics.includes(saved)) return;
    const full = mics.find((m) => m.toLowerCase().includes(saved.toLowerCase()));
    if (full) cfg.audio.device = full;
  }

  // ── Custom model (bring-your-own), shared by cleanup + transcription ──
  // Remember the URL per kind so a paused custom download can be resumed.
  let customUrls = $state<Record<string, string>>({});
  function startCustom(kind: string, url: string) {
    backendMsg = '';
    customUrls[kind] = url;
    const key = `custom-model::${kind}`;
    progressMap = { ...progressMap, [key]: { id: 'custom-model', variant: kind, phase: 'download', received: 0, total: 0 } };
    invoke('download_custom_model', { url, kind }).catch((e) => {
      progressMap = { ...progressMap, [key]: { id: 'custom-model', variant: kind, phase: 'error', received: 0, total: 0, message: '' + e } };
    });
  }
  function resumeCustom(kind: string) {
    if (customUrls[kind]) startCustom(kind, customUrls[kind]);
    else backendMsg = 'Start the download again to resume.';
  }
  function pickSearchModel(kind: string, repo: string, path: string) {
    startCustom(kind, `https://huggingface.co/${repo}/resolve/main/${path}`);
  }
  function cancelCustom(kind: string) {
    invoke('cancel_download', { id: 'custom-model', variant: kind }).catch(() => {});
    clearProgress(`custom-model::${kind}`);
  }
  async function chooseLocalModel(kind: string) {
    try {
      const extensions = kind === 'transcription' ? ['gguf', 'bin'] : ['gguf'];
      const f = await open({ multiple: false, filters: [{ name: 'Model', extensions }] });
      if (typeof f === 'string') {
        await invoke('use_local_model', { path: f, kind });
        await refreshModel();
        await refreshInstalled();
        backendMsg = 'Using local model — restart Glyph to apply.';
      }
    } catch (e) { backendMsg = '' + e; }
  }
  async function refreshStatuses() {
    try {
      const list = await invoke<any[]>('backend_status');
      const m: Record<string, any> = {};
      for (const s of list) m[s.id] = s;
      statuses = m;
    } catch {}
  }
  function installBackend(id: string, variant: string) {
    backendMsg = '';
    const key = `${id}::${variant}`;
    // Optimistically show a starting bar before the first progress event lands.
    progressMap = { ...progressMap, [key]: { id, variant, phase: 'download', received: 0, total: 0 } };
    invoke('download_backend', { id, variant }).catch((e) => {
      progressMap = { ...progressMap, [key]: { id, variant, phase: 'error', received: 0, total: 0, message: '' + e } };
    });
  }
  function clearProgress(key: string) {
    const { [key]: _drop, ...rest } = progressMap;
    progressMap = rest;
  }
  function pauseDownload(id: string, variant: string) {
    invoke('pause_download', { id, variant }).catch(() => {});
  }
  function cancelBackend(id: string, variant: string) {
    invoke('cancel_download', { id, variant }).catch(() => {});
    clearProgress(`${id}::${variant}`);
  }
  async function activateBackend(id: string, variant: string) {
    try {
      await invoke('activate_backend', { id, variant });
      await refreshStatuses();
      await refreshModel();
      backendMsg = 'Switched — restart Glyph to apply.';
    } catch (e) { backendMsg = '' + e; }
  }
  async function changeFolder() {
    const dir = await open({ directory: true, multiple: false, title: 'Choose download folder' });
    if (typeof dir === 'string') {
      await invoke('set_backends_dir', { path: dir });
      backendsDir = dir;
      await refreshStatuses();
    }
  }
  function openFolder() { invoke('reveal_backends_dir').catch(() => {}); }

  $effect(() => { if (tab === 'settings' && catalog.length === 0) loadBackends(); });

  // Auto-save: persist whenever cfg / dictionary / snippets change (debounced).
  $effect(() => {
    const snap = snapshot();
    if (!ready) { lastSnapshot = snap; return; }
    if (snap === lastSnapshot) return;
    scheduleSave();
  });

  const badgeState = $derived(
    engineState === 'Listening' ? 'recording'
    : engineState === 'Transcribing' ? 'processing'
    : engineState === 'Error' ? 'err'
    : engineReady ? 'ok' : 'idle'
  );
</script>

<main class="page">  <!-- ───────────── Header ───────────── -->
  <header class="head">
    <h1 class="word">Glyph</h1>
    <StatusBadge state={badgeState} label={engineState} />
  </header>

  <!-- ───────────── Tabs ───────────── -->
  <nav class="tabs">
    <button class:on={tab === 'home'} onclick={() => (tab = 'home')}>Home</button>
    <button class:on={tab === 'history'} onclick={() => (tab = 'history')}>History</button>
    <button class:on={tab === 'settings'} onclick={() => (tab = 'settings')}>Settings</button>
  </nav>

  {#key tab}
  <div class="panel-wrap" in:fly={{ ...tabIn, y: 6 }}>

  <!-- ───────────── Home ───────────── -->
  {#if tab === 'home' && cfg}
    <div class="sections">
      <!-- Dictation — the controls you change between sessions -->
      <section class="sec">
        <h2 class="sec-title">Dictation</h2>
        <p class="sec-sub">What you talk through, and how you trigger it. Changes apply instantly.</p>
        <div class="grid two">
          <Field label="Transcription model">
            {#if homeModels.length}
              <select value={activeModelValue} onchange={(e) => onHomeModelChange(e.currentTarget.value)}>
                {#each homeModels as m}<option value={m.value}>{m.label}</option>{/each}
              </select>
            {:else}
              <button type="button" class="nudge" onclick={() => (tab = 'settings')}>None yet — get one →</button>
            {/if}
          </Field>
          <Field label="Microphone">
            <select bind:value={cfg.audio.device}>
              <option value="">System default</option>
              {#each mics as m}<option value={m}>{m}</option>{/each}
              {#if cfg.audio.device && !mics.includes(cfg.audio.device)}
                <option value={cfg.audio.device}>{cfg.audio.device} (not connected)</option>
              {/if}
            </select>
          </Field>
        </div>
        <div class="grid two">
          <Field label="Hotkey">
            <HotkeyCapture bind:combo={cfg.hotkey.combo} />
          </Field>
          <Field label="Mode" hint="applies instantly">
            <select bind:value={cfg.hotkey.mode} onchange={applyMode}>
              <option value="hold">Hold to talk</option>
              <option value="toggle">Toggle · click / click</option>
            </select>
          </Field>
        </div>
      </section>

      <!-- Cleanup -->
      <section class="sec">
        <h2 class="sec-title">Cleanup</h2>
        <p class="sec-sub">An LLM pass that turns messy speech into clean, written text.</p>
        <div class="switchrow">
          <Switch bind:checked={cfg.cleanup.enabled} title="LLM polish"
            sub="Remove filler words, fix grammar, tighten phrasing." />
        </div>
        {#if cfg.cleanup.enabled}
          <div class="grid two">
            <Field label="Cleanup model" hint="applies live">
              {#if cleanupInstalled.length}
                <select value={activeCleanupPath} onchange={(e) => activateModel('cleanup', e.currentTarget.value)}>
                  {#each cleanupInstalled as m}<option value={m.path}>{prettyModel(m.name)}</option>{/each}
                </select>
              {:else}
                <button type="button" class="nudge" onclick={() => (tab = 'settings')}>None installed — get one →</button>
              {/if}
            </Field>
          </div>
        {/if}
      </section>

      <!-- Dictionary -->
      <section class="sec">
        <h2 class="sec-title">Dictionary</h2>
        <p class="sec-sub">Words Glyph should always spell correctly — one per line.</p>
        <textarea class="mono" bind:value={dictText} spellcheck="false"
          placeholder={"Glyph\nNemotron\nVulkan"}></textarea>
      </section>

      <!-- Jump to Settings -->
      <section class="sec">
        <button type="button" class="ghost" onclick={() => (tab = 'settings')}>Settings &amp; models →</button>
      </section>

      <div class="pad"></div>
    </div>
  {/if}

  <!-- ───────────── History ───────────── -->
  {#if tab === 'history'}
    <div class="sections">
      <div class="hhead">
        <span class="hcount">{history.length} {history.length === 1 ? 'entry' : 'entries'}</span>
        <div class="hhead-actions">
          <button class="ghost" onclick={refreshHistory}>Refresh</button>
          <button class="ghost danger" onclick={clearHist} disabled={history.length === 0}>Clear all</button>
        </div>
      </div>

      {#if history.length === 0}
        <div class="empty">
          <h3>No dictations yet</h3>
          <p>Hold your hotkey and start speaking — your transcripts show up here.</p>
        </div>
      {:else}
        <div class="hlist">
          {#each history as h}
            <HistoryEntry entry={h} onCopy={copyEntry} />
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <!-- ───────────── Settings ───────────── -->
  {#if tab === 'settings' && cfg}
    <div class="sections">
      <!-- Transcription -->
      <section class="sec">
        <button type="button" class="sec-head" aria-expanded={setupOpen.transcription} onclick={() => (setupOpen.transcription = !setupOpen.transcription)}>
          <h2 class="sec-title">Transcription</h2>
          <span class="chev" class:open={setupOpen.transcription} aria-hidden="true">⌄</span>
        </button>
        <p class="sec-sub">Speech-to-text models. Download a size here, then pick it on Home.</p>
        {#if setupOpen.transcription}
          <div class="setup-body" transition:fade={{ duration: 140 }}>
            <ModelSection
              kind="transcription"
              itemId="whisper-model"
              installed={transcriptionInstalled}
              presets={whisperPresets}
              progress={progressMap}
              onDownload={installBackend}
              onCancel={cancelBackend}
              onActivate={activateModel}
              onDelete={deleteModel}
              onPause={pauseDownload}
            />
            <button type="button" class="disclosure" class:open={setupOpen.txMore} onclick={() => (setupOpen.txMore = !setupOpen.txMore)}>
              {setupOpen.txMore ? 'Hide' : 'Show'} bring-your-own &amp; engine
            </button>
            {#if setupOpen.txMore}
              <div class="setup-body" transition:fade={{ duration: 140 }}>
                <ModelCustom
                  kind="transcription"
                  progress={progressMap}
                  onPick={(repo, path) => pickSearchModel('transcription', repo, path)}
                  onPasteUrl={(url) => startCustom('transcription', url)}
                  onChooseLocal={() => chooseLocalModel('transcription')}
                  onCancel={() => cancelCustom('transcription')}
                  onPause={() => pauseDownload('custom-model', 'transcription')}
                  onResume={() => resumeCustom('transcription')}
                />
                {#if nemotronInstalled.length === 0}
                  <button type="button" class="ghost" onclick={installNemotron}>Install Nemotron · streaming q8_0 (~940 MB)</button>
                {/if}
                {#if asrEngineItem}
                  <span class="run-h">Runtime · Nemotron (parakeet)</span>
                  <p class="sec-sub">Whisper's engine ships with the app — this is only needed if you use Nemotron. Pick the build that matches your hardware.</p>
                  <BackendItem
                    item={asrEngineItem}
                    status={statuses['asr-engine']}
                    progress={progressMap}
                    onInstall={installBackend}
                    onCancel={cancelBackend}
                    onActivate={activateBackend}
                    onPause={pauseDownload}
                  />
                {/if}
              </div>
            {/if}
            {#if backendMsg}<span class="status" data-kind="ok">{backendMsg}</span>{/if}
          </div>
        {/if}
      </section>

      <!-- Cleanup -->
      <section class="sec">
        <button type="button" class="sec-head" aria-expanded={setupOpen.cleanup} onclick={() => (setupOpen.cleanup = !setupOpen.cleanup)}>
          <h2 class="sec-title">Cleanup</h2>
          <span class="chev" class:open={setupOpen.cleanup} aria-hidden="true">⌄</span>
        </button>
        <p class="sec-sub">The instruct model that polishes your dictation. Pick it on Home.</p>
        {#if setupOpen.cleanup}
          <div class="setup-body" transition:fade={{ duration: 140 }}>
            {#if currentModel}<code class="active-model" title={currentModel}>Active: {currentModel}</code>{/if}
            <ModelSection
              kind="cleanup"
              itemId="cleanup-model"
              installed={cleanupInstalled}
              presets={cleanupPresets}
              progress={progressMap}
              onDownload={installBackend}
              onCancel={cancelBackend}
              onActivate={activateModel}
              onDelete={deleteModel}
              onPause={pauseDownload}
            />
            <button type="button" class="disclosure" class:open={setupOpen.clMore} onclick={() => (setupOpen.clMore = !setupOpen.clMore)}>
              {setupOpen.clMore ? 'Hide' : 'Show'} bring-your-own &amp; engine
            </button>
            {#if setupOpen.clMore}
              <div class="setup-body" transition:fade={{ duration: 140 }}>
                <ModelCustom
                  kind="cleanup"
                  progress={progressMap}
                  onPick={(repo, path) => pickSearchModel('cleanup', repo, path)}
                  onPasteUrl={(url) => startCustom('cleanup', url)}
                  onChooseLocal={() => chooseLocalModel('cleanup')}
                  onCancel={() => cancelCustom('cleanup')}
                  onPause={() => pauseDownload('custom-model', 'cleanup')}
                  onResume={() => resumeCustom('cleanup')}
                />
                {#if llamaItem}
                  <span class="run-h">Runtime · llama.cpp</span>
                  <p class="sec-sub">The llama.cpp server that runs the cleanup model. Install the build for your hardware.</p>
                  <BackendItem
                    item={llamaItem}
                    status={statuses['llama-server']}
                    progress={progressMap}
                    onInstall={installBackend}
                    onCancel={cancelBackend}
                    onActivate={activateBackend}
                    onPause={pauseDownload}
                  />
                {/if}
              </div>
            {/if}
            {#if backendMsg}<span class="status" data-kind="ok">{backendMsg}</span>{/if}
          </div>
        {/if}
      </section>

      <!-- Behavior -->
      <section class="sec">
        <button type="button" class="sec-head" aria-expanded={setupOpen.behavior} onclick={() => (setupOpen.behavior = !setupOpen.behavior)}>
          <h2 class="sec-title">Behavior</h2>
          <span class="chev" class:open={setupOpen.behavior} aria-hidden="true">⌄</span>
        </button>
        <p class="sec-sub">How Glyph runs and where text goes. Set once, mostly forget.</p>
        {#if setupOpen.behavior}
          <div class="setup-body" transition:fade={{ duration: 140 }}>
            <div class="grid two">
              <Field label="Transcription device" hint="GPU or CPU">
                <select bind:value={cfg.asr.device}>
                  <option value="auto">Auto · GPU</option>
                  <option value="Vulkan0">Vulkan · GPU</option>
                  <option value="CUDA0">CUDA · GPU</option>
                  <option value="cpu">CPU</option>
                </select>
              </Field>
              <Field label="Cleanup device">
                <select bind:value={cfg.cleanup.device}>
                  <option value="cpu">CPU · frees GPU</option>
                  <option value="gpu">GPU</option>
                </select>
              </Field>
            </div>
            <div class="grid two">
              <Field label="Injection method">
                <select bind:value={cfg.inject.method}>
                  <option value="paste">Clipboard paste</option>
                  <option value="unicode">Unicode typing</option>
                </select>
              </Field>
              <Field label="Clipboard">
                <div class="clip-toggle">
                  <Switch bind:checked={cfg.inject.keep_on_clipboard}
                    title="Keep on clipboard" sub="Leave dictated text on the clipboard." />
                </div>
              </Field>
            </div>
            <div class="grid one">
              <Field label="Cleanup instructions" hint="the system prompt sent to the cleanup model" full>
                <textarea class="mono prompt" bind:value={cfg.cleanup.prompt} spellcheck="false"></textarea>
              </Field>
            </div>
            <button type="button" class="ghost" onclick={() => (cfg.cleanup.prompt = defaultPrompt)}>Reset to default prompt</button>
          </div>
        {/if}
      </section>

      <!-- Snippets -->
      <section class="sec">
        <button type="button" class="sec-head" aria-expanded={setupOpen.words} onclick={() => (setupOpen.words = !setupOpen.words)}>
          <h2 class="sec-title">Snippets</h2>
          <span class="chev" class:open={setupOpen.words} aria-hidden="true">⌄</span>
        </button>
        <p class="sec-sub">Speak a cue; Glyph types the full expansion.</p>
        {#if setupOpen.words}
          <div class="setup-body" transition:fade={{ duration: 140 }}>
            <div class="snippets">
              {#each snippetList as s, i}
                <div class="snippet">
                  <input class="mono cue" bind:value={s.cue} placeholder="my email" />
                  <span class="arrow">→</span>
                  <input class="mono" bind:value={s.exp} placeholder="you@example.com" />
                  <button type="button" class="rm" onclick={() => removeSnippet(i)} aria-label="Remove">Remove</button>
                </div>
              {/each}
              <button type="button" class="add" onclick={addSnippet}>+ Add snippet</button>
            </div>
          </div>
        {/if}
      </section>

      <!-- Files -->
      <section class="sec">
        <button type="button" class="sec-head" aria-expanded={setupOpen.files} onclick={() => (setupOpen.files = !setupOpen.files)}>
          <h2 class="sec-title">Files</h2>
          <span class="chev" class:open={setupOpen.files} aria-hidden="true">⌄</span>
        </button>
        <p class="sec-sub">Where models live on disk, and the raw paths behind them.</p>
        {#if setupOpen.files}
          <div class="setup-body" transition:fade={{ duration: 140 }}>
            <span class="run-h">Download folder</span>
            <p class="sec-sub">Where backends and models are saved. Pick a drive with room — models are large.</p>
            <div class="folder-row">
              <code class="folder">{backendsDir}</code>
              <div class="folder-actions">
                <button class="ghost" onclick={changeFolder}>Change…</button>
                <button class="ghost" onclick={openFolder}>Open</button>
              </div>
            </div>
            <p class="tip">Install as many builds/models as you like — each downloads independently and they run in parallel.</p>

            <span class="run-h">Advanced</span>
            <p class="sec-sub">Paths and ports. Edit with care.</p>
            <button class="disclosure" class:open={advancedOpen} onclick={() => (advancedOpen = !advancedOpen)}>
              {advancedOpen ? 'Hide' : 'Show'} raw configuration
            </button>
            {#if advancedOpen}
              <div class="grid one" transition:fade={{ duration: 140 }}>
                <Field label="ASR language"><input bind:value={cfg.asr.lang} /></Field>
                <Field label="Cleanup port"><input type="number" bind:value={cfg.cleanup.port} /></Field>
                <Field label="Nemotron model (GGUF)" full><input class="mono" bind:value={cfg.asr.model} /></Field>
                <Field label="parakeet.dll" full><input class="mono" bind:value={cfg.asr.dll} /></Field>
                <Field label="Whisper sidecar binary" full><input class="mono" bind:value={cfg.asr.whisper.binary} /></Field>
                <Field label="Cleanup model (GGUF)" full><input class="mono" bind:value={cfg.cleanup.model} /></Field>
                <Field label="llama-server.exe" full><input class="mono" bind:value={cfg.cleanup.server} /></Field>
                <Field label="History DB" full><input class="mono" bind:value={cfg.history.path} /></Field>
              </div>
            {/if}
          </div>
        {/if}
      </section>

      <div class="pad"></div>
    </div>
  {/if}

  </div>
  {/key}

  <!-- Auto-save confirmation toast (replaces the old Save button) -->
  {#if status}
    <div class="toast" data-kind={statusKind} transition:fade={{ duration: 150 }}>{status}</div>
  {/if}
</main>

<style>
  .page {
    max-width: var(--shell-w);
    margin: 0 auto;
    min-height: 100vh;
    padding: 48px 56px 40px;
    display: flex;
    flex-direction: column;
  }

  /* Header */
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
  }

  /* Tab panel wrapper — flex child so savebar can stick to bottom */
  .panel-wrap { display: flex; flex-direction: column; flex: 1; min-height: 0; }

  .word {
    margin: 0;
    font-family: var(--font-serif);
    font-size: 30px;
    font-weight: 500;
    letter-spacing: -0.02em;
    color: var(--text);
    line-height: 1;
  }

  /* Tabs */
  .tabs {
    display: flex;
    gap: 4px;
    margin-top: 28px;
    border-bottom: 1px solid var(--hairline);
  }
  .tabs button {
    background: none;
    border: 0;
    cursor: pointer;
    color: var(--faint);
    font-family: inherit;
    font-size: 13.5px;
    font-weight: 500;
    padding: 10px 2px;
    margin-right: 28px;
    position: relative;
    transition: color 220ms var(--ease-out);
  }
  .tabs button:hover { color: var(--dim); }
  .tabs button.on { color: var(--text); }
  .tabs button::after {
    content: '';
    position: absolute;
    left: 0; right: 0; bottom: -1px;
    height: 2px;
    background: var(--accent);
    transform: scaleX(0);
    transform-origin: center;
    opacity: 0;
    transition: transform 260ms var(--ease-out), opacity 260ms var(--ease-out);
  }
  .tabs button.on::after { transform: scaleX(1); opacity: 1; }

  /* Sections — open on the background, separated by whitespace + hairline */
  .sections { flex: 1; padding-top: 8px; display: flex; flex-direction: column; gap: 0; }
  .sec {
    padding: 36px 0;
    border-bottom: 1px solid var(--hairline);
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .sec:first-child { padding-top: 28px; }
  .sec:last-of-type { border-bottom: 0; }

  .sec-title {
    margin: 0;
    font-family: var(--font-serif);
    font-size: 19px;
    font-weight: 500;
    letter-spacing: -0.015em;
    color: var(--text);
    line-height: 1.2;
  }
  .sec-sub {
    margin: -10px 0 0;
    font-size: 13px;
    color: var(--faint);
    line-height: 1.5;
    max-width: 54ch;
  }

  /* Collapsible Setup section header */
  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
  }
  .chev {
    color: var(--faint);
    font-size: 16px;
    line-height: 1;
    transition: transform 220ms var(--ease-out), color 220ms var(--ease-out);
  }
  .sec-head:hover .chev { color: var(--text); }
  .chev.open { transform: rotate(180deg); }

  .setup-body { display: flex; flex-direction: column; gap: 18px; }
  .run-h {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--faint-2);
    margin-top: 6px;
  }

  /* Nudge — empty-state link into Setup */
  .nudge {
    align-self: flex-start;
    background: transparent;
    border: 1px dashed var(--hairline-strong);
    border-radius: var(--r-md);
    color: var(--dim);
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    padding: 10px 14px;
    cursor: pointer;
    transition: color 200ms var(--ease-out), border-color 200ms var(--ease-out);
  }
  .nudge:hover { color: var(--text); border-color: var(--accent); }

  .switchrow { padding: 2px 0; }
  /* Center this toggle's pill vertically so it lines up with the Method dropdown. */
  .clip-toggle { display: flex; min-height: 38px; align-items: center; }
  .clip-toggle :global(.switch) { align-items: center; }

  .grid { display: grid; gap: 16px 22px; }
  .grid.two { grid-template-columns: 1fr 1fr; }
  .grid.one { grid-template-columns: 1fr; }

  /* shared inputs */
  :global(.page input:not([type='checkbox'])),
  :global(.page select),
  :global(.page textarea) {
    width: 100%;
    background: transparent;
    color: var(--text);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    padding: 10px 12px;
    font-size: 13.5px;
    font-family: inherit;
    transition: border-color 220ms var(--ease-out), background 220ms var(--ease-out);
  }
  :global(.page input:hover),
  :global(.page select:hover),
  :global(.page textarea:hover) { border-color: var(--hairline-strong); }
  :global(.page input:focus),
  :global(.page select:focus),
  :global(.page textarea:focus) {
    outline: none;
    border-color: var(--accent);
    background: var(--bg-soft);
  }
  :global(.page select) {
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%238e8573' stroke-width='2.5' stroke-linecap='round'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 12px center;
    padding-right: 32px;
    cursor: pointer;
  }
  /* No resize grip anywhere — it doesn't work in the webview. Textareas are a
     fixed height and scroll instead. */
  :global(.page textarea) { resize: none; min-height: 130px; line-height: 1.65; overflow-y: auto; }
  :global(.page textarea.prompt) { height: 220px; min-height: 0; }
  :global(.page .mono) { font-family: var(--font-mono); font-size: 12.5px; }

  /* snippets */
  .snippets { display: flex; flex-direction: column; gap: 10px; }
  .snippet { display: flex; align-items: center; gap: 10px; }
  .snippet input { flex: 1; }
  .snippet .cue { flex: 0 0 36%; }
  .arrow { color: var(--faint-2); font-size: 14px; flex: none; }
  .rm {
    background: transparent;
    border: 0;
    cursor: pointer;
    font-family: inherit;
    font-size: 11.5px;
    color: var(--faint);
    padding: 4px 6px;
    border-radius: var(--r-xs);
    flex: none;
    transition: color var(--dur) var(--ease);
  }
  .rm:hover { color: var(--rec); }
  .add {
    align-self: flex-start;
    background: transparent;
    border: 0;
    cursor: pointer;
    color: var(--dim);
    font-family: inherit;
    font-size: 13px;
    padding: 2px 0;
    transition: color var(--dur) var(--ease);
  }
  .add:hover { color: var(--accent); }

  /* advanced disclosure */
  .disclosure {
    background: transparent;
    border: 0;
    cursor: pointer;
    color: var(--dim);
    font-family: inherit;
    font-size: 12.5px;
    padding: 0;
    transition: color var(--dur) var(--ease);
  }
  .disclosure:hover { color: var(--text); }

  .pad { height: 8px; }

  /* files */
  .folder-row {
    display: flex;
    align-items: center;
    gap: 12px;
    justify-content: space-between;
  }
  .folder {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--dim);
    background: var(--bg-soft);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    padding: 9px 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .folder-actions { display: flex; gap: 8px; flex: none; }
  .tip { margin: 2px 0 0; font-size: 12px; color: var(--faint-2); line-height: 1.5; }

  /* active model path shown above the cleanup model list */
  .active-model {
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--dim);
    background: var(--bg-soft);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    padding: 7px 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* history */
  .hhead { display: flex; align-items: center; justify-content: space-between; padding: 28px 0 8px; }
  .hcount { font-family: var(--font-mono); font-size: 11.5px; color: var(--faint); }
  .hhead-actions { display: flex; gap: 8px; }
  .hlist { display: flex; flex-direction: column; }

  .empty {
    margin-top: 32px;
    padding: 56px 24px;
    text-align: center;
    border-bottom: 1px solid var(--hairline);
  }
  .empty h3 { margin: 0 0 6px; font-family: var(--font-serif); font-size: 17px; font-weight: 500; color: var(--text); }
  .empty p { margin: 0; font-size: 13px; color: var(--faint); max-width: 38ch; line-height: 1.5; margin: 0 auto; }

  /* buttons */
  .ghost {
    align-self: flex-start;
    background: transparent;
    color: var(--dim);
    border: 1px solid var(--hairline);
    border-radius: var(--r-md);
    padding: 7px 13px;
    font-family: inherit;
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    transition: color 200ms var(--ease-out), border-color 200ms var(--ease-out);
  }
  .ghost:hover { color: var(--text); border-color: var(--hairline-strong); }
  .ghost.danger:hover { color: var(--rec); border-color: rgba(217, 102, 81, 0.4); }
  .ghost:disabled { opacity: 0.4; cursor: not-allowed; }

  /* inline status (e.g. backend messages) */
  .status { font-size: 12.5px; color: var(--ok); min-height: 18px; transition: opacity 200ms var(--ease-out); }

  /* auto-save confirmation toast */
  .toast {
    position: fixed;
    bottom: 22px;
    right: 24px;
    background: var(--bg-soft);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--r-md);
    padding: 9px 15px;
    font-size: 12.5px;
    color: var(--ok);
    z-index: 50;
  }
  .toast[data-kind='err'] { color: var(--rec); border-color: color-mix(in srgb, var(--rec) 40%, transparent); }

  @media (prefers-reduced-motion: reduce) {
    * { transition-duration: 1ms !important; }
  }
</style>
