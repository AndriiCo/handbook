import { test } from 'node:test';
import assert from 'node:assert/strict';
import { REPORT_COPY } from '../src/lib/public-ui/report-copy.js';

test('REPORT_COPY: every language has the same keys', () => {
	const keys = Object.keys(REPORT_COPY.en).sort();
	for (const code of ['en', 'es', 'fr']) assert.deepEqual(Object.keys(REPORT_COPY[code]).sort(), keys, code);
});

test('REPORT_COPY: Spanish and French are not English', () => {
	// Same word on purpose: "Photos" is French too.
	const same = new Set(['fr.photos']);
	for (const code of ['es', 'fr']) {
		for (const [key, value] of Object.entries(REPORT_COPY[code])) {
			if (same.has(`${code}.${key}`)) continue;
			const en = REPORT_COPY.en[key];
			const a = typeof value === 'function' ? value('X') : value;
			const b = typeof en === 'function' ? en('X') : en;
			assert.notEqual(a, b, `${code}.${key}`);
		}
	}
});

test('REPORT_COPY: the received heading names the report when it has an id', () => {
	assert.equal(REPORT_COPY.en.receivedHeading(12), 'Report #12 received.');
	assert.equal(REPORT_COPY.en.receivedHeading(null), 'Report received.');
	assert.match(REPORT_COPY.es.receivedHeading(12), /n\.º 12/);
	assert.match(REPORT_COPY.fr.receivedHeading(null), /^Signalement reçu/);
});
