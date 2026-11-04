// route-budget.mjs — advisory Brotli size report for one public route
// (site-theme.md §2: per-route page ≤ 30 KB Brotli, the "additional
// per-route chunk loaded on navigation"). Reads the fresh public
// `build/` (run `npm run build` first) and prints:
//   - entry: the client runtime, the app entry and the root layout's
//     closure (what every public page loads);
//   - route: the route node's static closure beyond the entry;
//   - each lazy chunk reachable by dynamic import, with its own closure
//     beyond what loaded before it (nested lazy chunks indented);
//   - per navigation: the route plus the chunks the route requests
//     itself (first-level lazy). This is the page the budget counts: a
//     chunk the route imports as its module loads arrives on every
//     visit, so moving code into one does not take it off the page.
// Sizes are Brotli quality 11 per file, as a precompressing server
// serves them. No gate: size reporting is advisory since 2026-10-28;
// `--max <KB>` exits 1 when the per-navigation total is over, for local
// checks only.
//
// Usage: node scripts/route-budget.mjs [route id] [--max <KB>]
//   (default route: /(public)/authority/[...trail])
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, join, normalize, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { brotliCompressSync, constants } from 'node:zlib';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const maxAt = args.indexOf('--max');
const max = maxAt >= 0 ? Number(args[maxAt + 1]) : null;
const routeId =
	args.find((a, i) => !a.startsWith('--') && (maxAt < 0 || i !== maxAt + 1)) ?? '/(public)/authority/[...trail]';

const imm = join(root, 'build', '_app', 'immutable');
const entryDir = join(imm, 'entry');
if (!existsSync(entryDir)) {
	console.error('route-budget: no build/ found — run `npm run build` first');
	process.exit(2);
}
const appFile = readdirSync(entryDir).find((f) => f.startsWith('app.'));
const startFile = readdirSync(entryDir).find((f) => f.startsWith('start.'));
const app = readFileSync(join(entryDir, appFile), 'utf8');

// The app entry lists every node's dependencies (`__vite__mapDeps`) and
// the route dictionary (route id → [page node, [layout nodes]]).
const depList = JSON.parse(`[${app.match(/m\.f=\[([^\]]*)\]/)[1]}]`);
const nodeFiles = new Map(
	[...app.matchAll(/import\(`\.\.\/nodes\/(\d+)\.[^`]+`\),__vite__mapDeps\(\[([\d,]*)\]\)/g)].map((m) => [
		Number(m[1]),
		m[2]
			.split(',')
			.filter(Boolean)
			.map((i) => normalize(join(entryDir, depList[Number(i)])))
	])
);
const routes = JSON.parse(`{${app.match(/\{("\/[^}]*)\}/)[1]}}`);
const target = routes[routeId];
if (!target) {
	console.error(`route-budget: no route ${routeId} (known: ${Object.keys(routes).join(', ')})`);
	process.exit(2);
}
const [pageNode, layouts = []] = target;

const STATIC = /(?:^|[;\s}])import\s*(?:[^'"`()]*?from\s*)?["'`](\.{1,2}\/[^"'`]+)["'`]/g;
const DYNAMIC = /import\(\s*["'`](\.{1,2}\/[^"'`]+)["'`]\s*\)/g;
const source = (file) => readFileSync(file, 'utf8');

/**
 * Dynamic imports in `file`, each with the files Vite's preload helper
 * fetches alongside it (`__vite__mapDeps([...])`: its chunks and its CSS).
 * @returns {{ chunk: string, deps: string[] }[]}
 */
function dynamicImports(file) {
	const text = source(file);
	const table = text.match(/m\.f=\[([^\]]*)\]/);
	const list = table ? JSON.parse(`[${table[1]}]`) : [];
	const out = [];
	for (const m of text.matchAll(DYNAMIC)) {
		const after = text.slice(m.index + m[0].length, m.index + m[0].length + 240);
		const call = after.split('import(')[0].match(/__vite__mapDeps\(\[([\d,]*)\]\)/);
		const deps = call
			? call[1]
					.split(',')
					.filter(Boolean)
					.map((i) => normalize(join(file, '..', list[Number(i)])))
			: [];
		out.push({ chunk: normalize(join(file, '..', m[1])), deps });
	}
	return out;
}

/** Static import closure of `file` (dynamic imports are not followed). */
function closure(file, out = new Set()) {
	if (out.has(file)) return out;
	out.add(file);
	if (file.endsWith('.js')) for (const m of source(file).matchAll(STATIC)) closure(normalize(join(file, '..', m[1])), out);
	return out;
}

const brCache = new Map();
function br(file) {
	if (!brCache.has(file)) {
		brCache.set(file, brotliCompressSync(readFileSync(file), { params: { [constants.BROTLI_PARAM_QUALITY]: 11 } }).length);
	}
	return brCache.get(file);
}
const kb = (bytes) => `${(bytes / 1024).toFixed(2)} KB`;
const sum = (files) => [...files].reduce((n, f) => n + br(f), 0);
const name = (file) => relative(imm, file);

const entry = new Set();
closure(join(entryDir, startFile), entry);
closure(join(entryDir, appFile), entry);
for (const n of [0, ...layouts]) for (const f of nodeFiles.get(n) ?? []) closure(f, entry);

const pageAll = new Set();
for (const f of nodeFiles.get(pageNode) ?? []) closure(f, pageAll);
const page = [...pageAll].filter((f) => !entry.has(f));

console.log(`route ${routeId} (node ${pageNode})`);
console.log(`  entry + layout   ${kb(sum(entry)).padStart(10)}  (${entry.size} files, every public page)`);
const pageBytes = sum(page);
console.log(`  route            ${kb(pageBytes).padStart(10)}  (${page.length} files)${max ? `  budget ${max} KB` : ''}`);
for (const f of [...page].sort((a, b) => br(b) - br(a)).slice(0, 6)) console.log(`      ${kb(br(f)).padStart(10)}  ${name(f)}`);

// Lazy chunks the route's own code requests: each one's closure beyond
// what loads before it (the entry, the route, and its parent chunk).
const seen = new Set();
const firstLevel = new Set();
function lazyFrom(files, base, depth) {
	for (const f of files) {
		if (!f.endsWith('.js')) continue;
		for (const { chunk, deps } of dynamicImports(f)) {
			if (seen.has(chunk) || base.has(chunk)) continue;
			seen.add(chunk);
			const all = closure(chunk);
			for (const d of deps) closure(d, all);
			const own = [...all].filter((x) => !base.has(x));
			if (depth === 1) for (const x of own) firstLevel.add(x);
			console.log(`  ${'  '.repeat(depth)}lazy ${kb(sum(own)).padStart(10)}  ${name(chunk)} (+${own.length - 1} files)`);
			lazyFrom(own, new Set([...base, ...own]), depth + 1);
		}
	}
}
lazyFrom(page, new Set([...entry, ...pageAll]), 1);
console.log(`  first-level lazy ${kb(sum(firstLevel)).padStart(10)}  (union of the chunks the route requests itself)`);
const perNavigation = pageBytes + sum(firstLevel);
console.log(
	`  per navigation   ${kb(perNavigation).padStart(10)}  (route + first-level lazy; site-theme §2 budget 30 KB${max ? `, --max ${max} KB` : ''})`
);
if (max !== null && perNavigation / 1024 > max) {
	console.error(`route-budget: ${routeId} is ${kb(perNavigation)} per navigation, over the ${max} KB budget`);
	process.exit(1);
}
