// Route + separation smoke over the two static builds.
// Run AFTER both builds: npm run build && npm run build:admin && npm run smoke.
// Fails when: a build is missing, the public bundle leaks admin-only
// markers, or the admin bundle lacks them.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = new URL('..', import.meta.url).pathname;
const PUBLIC = join(ROOT, 'build');
const ADMIN = join(ROOT, 'build-admin');

// Strings that exist ONLY in (admin) route code.
const ADMIN_ONLY = ['/(admin)/config', 'Staff only'];
// Strings that exist in shared/public route code (both builds).
const SHARED = ['Start with your destination'];
// Route ids registered in the client manifest (entry chunk).
const PUBLIC_ROUTES = [
	'/(public)',
	'/(public)/authority/[...trail]',
	'/(customer)/orders',
	'/(customer)/uploads',
	'/(customer)/precheck'
];
const ADMIN_ROUTES = [
	'/(admin)/dashboard',
	'/(admin)/review',
	'/(admin)/trace',
	'/(admin)/config',
	'/(admin)/triage',
	'/(admin)/staff-login',
	'/(admin)/enrol',
	'/(admin)/workers',
	'/(admin)/calls',
	'/(admin)/calls/[id]'
];

function* files(dir) {
	for (const entry of readdirSync(dir)) {
		const path = join(dir, entry);
		if (statSync(path).isDirectory()) yield* files(path);
		else yield path;
	}
}

function containsHaystack(dir, needle) {
	for (const file of files(dir)) {
		if (/\.(html|js|css)$/.test(file) && readFileSync(file, 'utf8').includes(needle)) {
			return true;
		}
	}
	return false;
}

function entrySource(dir) {
	const entryDir = join(dir, '_app/immutable/entry');
	return readdirSync(entryDir)
		.filter((name) => name.startsWith('app.') && name.endsWith('.js'))
		.map((name) => readFileSync(join(entryDir, name), 'utf8'))
		.join('\n');
}

let failures = 0;
function check(ok, message) {
	console.log(`${ok ? 'ok' : 'FAIL'}: ${message}`);
	if (!ok) failures += 1;
}

check(existsSync(join(PUBLIC, 'index.html')), 'public build has index.html fallback');
check(existsSync(join(ADMIN, 'index.html')), 'admin build has index.html fallback');

for (const marker of ADMIN_ONLY) {
	check(!containsHaystack(PUBLIC, marker), `public bundle has no “${marker}”`);
	check(containsHaystack(ADMIN, marker), `admin bundle contains “${marker}”`);
}
for (const marker of SHARED) {
	check(containsHaystack(PUBLIC, marker), `public bundle contains “${marker}”`);
	check(containsHaystack(ADMIN, marker), `admin bundle contains “${marker}”`);
}

const publicBase = readFileSync(join(PUBLIC, 'index.html'), 'utf8').includes('/apostille');
check(publicBase, 'public shell references /apostille base');

const publicEntry = entrySource(PUBLIC);
const adminEntry = entrySource(ADMIN);
for (const id of PUBLIC_ROUTES) {
	check(publicEntry.includes(`"${id}"`), `public manifest registers ${id}`);
	check(adminEntry.includes(`"${id}"`), `admin manifest registers ${id}`);
}
for (const id of ADMIN_ROUTES) {
	check(!publicEntry.includes(id), `public manifest has no ${id}`);
	check(adminEntry.includes(`"${id}"`), `admin manifest registers ${id}`);
}
// Admin domain-root contract (server redirects `/` → `/dashboard`, see
// crates/web/src/admin.rs): the admin build must register the redirect
// target, and must not contain a second root page (SvelteKit would refuse
// to build two `/` routes — this pins that the admin build's `/` is still
// exactly the (public) homepage the redirect exists to bypass).
check(adminEntry.includes('"/(admin)/dashboard"'), 'admin manifest registers redirect target /(admin)/dashboard');
check(!adminEntry.includes('"/(admin)/console"'), 'retired /(admin)/console registers no route');
const adminRootPages = (adminEntry.match(/"\/\((public|admin|mock|customer)\)"/g) || []).length;
check(adminRootPages === 1, `admin manifest has exactly one root route (found ${adminRootPages})`);

if (failures > 0) {
	console.error(`smoke FAILED (${failures})`);
	process.exit(1);
}
console.log('smoke OK: both builds serve, admin split holds.');
