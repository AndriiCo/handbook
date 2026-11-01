// brand-fonts.mjs — stage the proprietary Vogun brand face for release
// builds. Copies Vogun-Medium.woff2 + Vogun-MediumItalic.woff2 from
// $BRAND_FONT_DIR into static/fonts/brand/ (git-ignored; the files never
// enter the repo because it will be public). Without the directory the
// build continues on the 'Vogun Fallback' face — one line, no failure.
//
// Usage: node scripts/brand-fonts.mjs (also wired as prebuild hooks).
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'static', 'fonts', 'brand');
const files = ['Vogun-Medium.woff2', 'Vogun-MediumItalic.woff2'];

const srcDir = process.env.BRAND_FONT_DIR;
if (!srcDir || !existsSync(srcDir)) {
	// One line on the fallback path; the build must continue.
	console.log('brand-fonts: no BRAND_FONT_DIR — building with the Vogun Fallback face');
	process.exit(0);
}

mkdirSync(outDir, { recursive: true });
let staged = 0;
for (const file of files) {
	const src = join(srcDir, file);
	if (!existsSync(src)) {
		console.log(`brand-fonts: missing ${file} in BRAND_FONT_DIR — building with the fallback face`);
		continue;
	}
	// Served unmodified: the licence forbids subsetting or conversion.
	copyFileSync(src, join(outDir, file));
	staged += 1;
}
console.log(`brand-fonts: staged ${staged}/${files.length} Vogun files`);
