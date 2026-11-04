import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

// Staff secrets (config keys, setup keys) go through SecretInput: a
// `type="password"` field makes browsers and password managers offer
// to save a config secret as a login.
const SRC = join(dirname(fileURLToPath(import.meta.url)), '..', 'src');

function* svelteFiles(dir) {
	for (const entry of readdirSync(dir)) {
		const path = join(dir, entry);
		if (statSync(path).isDirectory()) yield* svelteFiles(path);
		else if (path.endsWith('.svelte')) yield path;
	}
}

test('no route renders a password-type field', () => {
	const offenders = [];
	for (const file of svelteFiles(join(SRC, 'routes'))) {
		const text = readFileSync(file, 'utf8');
		if (/type=["']password["']|autocomplete=["'](new|current)-password["']/.test(text)) {
			offenders.push(file.slice(SRC.length + 1));
		}
	}
	assert.deepEqual(offenders, []);
});

test('SecretInput opts out of autofill and password managers', () => {
	const text = readFileSync(join(SRC, 'lib', 'admin', 'SecretInput.svelte'), 'utf8');
	for (const attr of [
		'type="text"',
		'autocomplete="off"',
		'data-1p-ignore',
		'data-lpignore="true"',
		'data-bwignore="true"',
		'data-form-type="other"',
		'data-protonpass-ignore="true"',
		'aria-describedby={hintId}',
		'aria-pressed={shown}'
	]) {
		assert.ok(text.includes(attr), attr);
	}
	assert.ok(!/type=["']password["']/.test(text.replace(/\/\/.*$/gm, '')));
});
