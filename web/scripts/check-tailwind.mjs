// check-tailwind.mjs — fail-closed Tailwind audit. Unknown utilities
// silently emit no CSS, so every class token used in .svelte must
// resolve to a rule in the built CSS (public or admin) or to a class
// defined in app.css / a remaining <style> block (print hooks, :global
// resets, @page pages). Runs AFTER both builds (needs fresh CSS).
//
// Usage: node scripts/check-tailwind.mjs (also wired into `xtask ci`).
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

function walk(dir, ext, out = []) {
	for (const name of readdirSync(dir)) {
		const path = join(dir, name);
		if (statSync(path).isDirectory()) {
			if (name !== 'routes-public') walk(path, ext, out); // symlinked twin
		} else if (name.endsWith(ext)) out.push(path);
	}
	return out;
}

let css = '';
for (const build of ['build', 'build-admin']) {
	for (const file of walk(join(root, build, '_app', 'immutable', 'assets'), '.css')) {
		css += readFileSync(file, 'utf8');
	}
}
if (!css) {
	console.error('check-tailwind: no built CSS found — run `npm run build` + `build:admin` first');
	process.exit(2);
}

// Classes defined outside Tailwind: app.css + remaining <style> blocks.
let defined = new Set();
defined.add('masthead');
defined.add('menus');
{
	const app = readFileSync(join(root, 'src', 'app.css'), 'utf8');
	for (const m of app.matchAll(/\.([a-zA-Z][\w-]*)/g)) defined.add(m[1]);
}
for (const file of walk(join(root, 'src'), '.svelte')) {
	const src = readFileSync(file, 'utf8');
	for (const style of src.matchAll(/<style>([\s\S]*?)<\/style>/g)) {
		for (const m of style[1].matchAll(/\.([a-zA-Z][\w-]*)/g)) defined.add(m[1]);
	}
}

// Tokens: class="..." attributes + helper backtick literals
// (tabCls/filterCls/pillCls/liCls/badgeCls/stepCls full strings).
const tokens = new Set();
for (const file of walk(join(root, 'src'), '.svelte')) {
	const src = readFileSync(file, 'utf8');
	for (const m of src.matchAll(/class="([^"]+)"/g)) {
		for (const w of m[1].split(/\s+/)) if (w) tokens.add(w);
	}
	for (const m of src.matchAll(/`([^`]*?)`/g)) {
		const chunk = m[1];
		if (!/(flex|bg-|text-|border)/.test(chunk)) continue;
		for (let w of chunk.split(/\s+/)) {
			w = w.replace(/^[^a-z\[]+|[^a-z0-9\]()%#,._-]+$/gi, '');
			if (/^[a-z\[]/.test(w) && /[-:\[]/.test(w)) tokens.add(w);
		}
	}
}

const esc = (t) =>
	t.replaceAll(':', '\\:').replaceAll('[', '\\[').replaceAll(']', '\\]').replaceAll('#', '\\#')
		.replaceAll('.', '\\.').replaceAll('/', '\\/').replaceAll('%', '\\%').replaceAll('(', '\\(')
		.replaceAll(')', '\\)').replaceAll(',', '\\,').replaceAll('=', '\\=').replaceAll('!', '\\!')
		.replaceAll('*', '\\*');

const missing = [];
for (const t of [...tokens].sort()) {
	if (defined.has(t)) continue;
	// A class also resolves when followed by `.` (compound selector,
	// e.g. `.toaster.group` from the Sonner root) or `>` (the `*:`
	// child variant emits `.cls>*`).
	if (['{', ':', ',', '.', '>'].some((c) => css.includes('.' + esc(t) + c))) continue;
	missing.push(t);
}

if (missing.length > 0) {
	console.error(`check-tailwind: ${missing.length} used ${missing.length === 1 ? 'class' : 'classes'} missing from built CSS:`);
	for (const t of missing) console.error(`  ${t}`);
	process.exit(1);
}
console.log(`check-tailwind OK: ${tokens.size} tokens all resolve (${defined.size} style-defined).`);
