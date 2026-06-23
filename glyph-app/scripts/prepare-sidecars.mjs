// Build the Whisper + parakeet sidecar executables and stage them where Tauri's
// `externalBin` bundler expects them (named with the host target triple). This
// runs as part of `npm run tauri build` (see beforeBuildCommand) so the produced
// installer ships the sidecars next to glyph-app.exe — otherwise the installed
// app launches but can't transcribe.
import { execSync } from 'node:child_process';
import { mkdirSync, copyFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url)); // glyph-app/scripts
const repoRoot = join(here, '..', '..'); // workspace root (crates/ live here)
const releaseDir = join(repoRoot, 'target', 'release');
const outDir = join(here, '..', 'src-tauri', 'binaries');

const exe = process.platform === 'win32' ? '.exe' : '';
const triple = execSync('rustc -vV').toString().match(/host:\s*(\S+)/)[1];

const sidecars = ['glyph-asr-sidecar', 'glyph-whisper-sidecar'];

console.log('[sidecars] building release binaries…');
execSync(`cargo build --release ${sidecars.map((s) => `-p ${s}`).join(' ')}`, {
  cwd: repoRoot,
  stdio: 'inherit',
});

mkdirSync(outDir, { recursive: true });
for (const name of sidecars) {
  const src = join(releaseDir, `${name}${exe}`);
  const dst = join(outDir, `${name}-${triple}${exe}`);
  if (!existsSync(src)) throw new Error(`[sidecars] missing build output: ${src}`);
  copyFileSync(src, dst);
  console.log(`[sidecars] staged ${dst}`);
}
