// Import-separation gate (spa-separation.md §1.2): (public) routes must
// never import customer/admin libs. Usage: npm run check:imports.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = new URL('..', import.meta.url).pathname;
const PUBLIC = join(ROOT, 'src/routes/(public)');
const ROUTES_PUBLIC = join(ROOT, 'src/routes-public');

// Static + dynamic imports of customer/admin-only modules.
const BANNED = [
	/lib\/customer\//,
	/lib\/admin\//,
	/\$lib\/customer\//,
	/\$lib\/admin\//,
	/#lib\/customer\//,
	/#lib\/admin\//
];

function* walk(dir) {
	for (const entry of readdirSync(dir)) {
		const path = join(dir, entry);
		if (statSync(path).isDirectory()) yield* walk(path);
		else if (/\.(svelte|js|ts)$/.test(entry)) yield path;
	}
}

let failures = 0;
for (const file of walk(PUBLIC)) {
	const text = readFileSync(file, 'utf8');
	for (const pattern of BANNED) {
		if (pattern.test(text)) {
			console.error(`BANNED IMPORT in ${file}: matches ${pattern}`);
			failures += 1;
		}
	}
}

// The public build's routes dir must not resolve an (admin) group.
for (const entry of readdirSync(ROUTES_PUBLIC)) {
	if (entry === '(admin)') {
		console.error('BANNED: src/routes-public contains (admin)');
		failures += 1;
	}
}

if (failures > 0) {
	console.error(`check:imports FAILED (${failures})`);
	process.exit(1);
}
console.log('check:imports OK: no customer/admin imports from (public).');
