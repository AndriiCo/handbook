// brand-fonts.mjs — stage licensed brand files for release builds.
// Copies the Vogun webfonts from $BRAND_FONT_DIR into static/fonts/brand/
// and the provider logos (google, apple, meta, passkey .svg) from
// $BRAND_DIR into src/lib/assets/brand/. Both targets are git-ignored:
// the files are operator-licensed and never enter the repo.
// `cargo xtask setup` passes only files whose licence acceptance is
// recorded in the setup file. Without the directories the build
// continues on the 'Vogun Fallback' face and text-only provider
// buttons — one line each, no failure.
//
// Usage: node scripts/brand-fonts.mjs (also wired as prebuild hooks).
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

function stage(envName, files, outDir, label) {
	const srcDir = process.env[envName];
	if (!srcDir || !existsSync(srcDir)) {
		console.log(`brand-fonts: no ${envName} — building without the licensed ${label}`);
		return;
	}
	let staged = 0;
	for (const file of files) {
		const src = join(srcDir, file);
		if (!existsSync(src)) continue;
		mkdirSync(outDir, { recursive: true });
		// Served unmodified: the licences forbid altering the files.
		copyFileSync(src, join(outDir, file));
		staged += 1;
	}
	console.log(`brand-fonts: staged ${staged}/${files.length} ${label}`);
}

stage(
	'BRAND_FONT_DIR',
	['Vogun-Medium.woff2', 'Vogun-MediumItalic.woff2'],
	join(root, 'static', 'fonts', 'brand'),
	'Vogun files'
);
stage(
	'BRAND_DIR',
	['google.svg', 'apple.svg', 'meta.svg', 'passkey.svg'],
	join(root, 'src', 'lib', 'assets', 'brand'),
	'provider logos'
);
