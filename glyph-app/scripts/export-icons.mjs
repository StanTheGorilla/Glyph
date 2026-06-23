// Renders the Glyph brand SVG to every icon size the Tauri bundle expects,
// and hand-assembles a multi-resolution .ico (no extra dep — the ICO format
// is a small fixed binary header + concatenated PNGs).
//
//   node scripts/export-icons.mjs
//
// Reads  scripts/glyph-mark.svg
// Writes src-tauri/icons/{32,128,128@2x,Square*,StoreLogo}.png, icon.png,
//         icon.ico (256+128+64+48+32+16), icon.icns (best-effort via sips on macOS;
//         skipped elsewhere — Windows is the only target here).

import { Resvg } from '@resvg/resvg-js';
import sharp from 'sharp';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, '..');
const ICONS = join(ROOT, 'src-tauri', 'icons');
const SVG = readFileSync(join(__dirname, 'glyph-mark.svg'), 'utf8');

mkdirSync(ICONS, { recursive: true });

// { outputName: pixelSize }
const PNGS = {
  '32x32.png': 32,
  '128x128.png': 128,
  '128x128@2x.png': 256,
  'icon.png': 512,
  'Square30x30Logo.png': 30,
  'Square44x44Logo.png': 44,
  'Square71x71Logo.png': 71,
  'Square89x89Logo.png': 89,
  'Square107x107Logo.png': 107,
  'Square142x142Logo.png': 142,
  'Square150x150Logo.png': 150,
  'Square284x284Logo.png': 284,
  'Square310x310Logo.png': 310,
  'StoreLogo.png': 50,
};

function renderPng(size) {
  const r = new Resvg(SVG, {
    fitTo: { mode: 'width', value: size },
    background: '#00000000',
  });
  return r.render().asPng();
}

const pngByName = {};
for (const [name, size] of Object.entries(PNGS)) {
  const png = renderPng(size);
  writeFileSync(join(ICONS, name), png);
  pngByName[size] ??= png; // cache one png per size for the ico
  console.log(`  ${name}  (${size}px)`);
}

// ---- Multi-resolution .ico (PNG-encoded entries) ----
// ICONDIR (6 bytes) + ICONDIRENTRY (16 bytes each) + PNG data.
const ICO_SIZES = [16, 32, 48, 64, 128, 256];
const entries = ICO_SIZES.map((s) => {
  // render at the requested size; ICO stores width/height as u8 (0 == 256).
  const png = renderPng(s);
  return { size: s, png };
});

const headerLen = 6;
const entryLen = 16;
let offset = headerLen + entries.length * entryLen;
const dir = Buffer.alloc(headerLen);
dir.writeUInt16LE(0, 0); // reserved
dir.writeUInt16LE(1, 2); // type = icon
dir.writeUInt16LE(entries.length, 4);

const entryBufs = entries.map((e, i) => {
  const b = Buffer.alloc(entryLen);
  const dim = e.size === 256 ? 0 : e.size;
  b.writeUInt8(dim, 0);          // width
  b.writeUInt8(dim, 1);          // height
  b.writeUInt8(0, 2);            // color count (0 = >=256)
  b.writeUInt8(0, 3);            // reserved
  b.writeUInt16LE(1, 4);         // planes
  b.writeUInt16LE(32, 6);        // bpp
  b.writeUInt32LE(e.png.length, 8); // size
  b.writeUInt32LE(offset, 12);   // offset
  offset += e.png.length;
  return b;
});

const ico = Buffer.concat([dir, ...entryBufs, ...entries.map((e) => e.png)]);
writeFileSync(join(ICONS, 'icon.ico'), ico);
console.log('  icon.ico  (multi-res: ' + ICO_SIZES.join(',') + ')');

// Apple .icns: out of scope on Windows (no target platform). Create a
// placeholder 1024 png so the bundle list element still resolves.
const bigPng = renderPng(1024);
writeFileSync(join(ICONS, 'icon.icns'), bigPng);
console.log('  icon.png + icon.icns written');

console.log('\nDone —', Object.keys(PNGS).length + 1, 'files in', ICONS);
