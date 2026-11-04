import test from 'node:test';
import assert from 'node:assert/strict';
import { fakeRibbon } from '../src/lib/admin/fake-client.js';

test('the FAKE ribbon names every fake integration', () => {
	const text = fakeRibbon([
		{ slug: 'mail_out', label: 'Outgoing email' },
		{ slug: 'labels', label: 'FedEx/USPS labels' }
	]);
	assert.ok(text?.startsWith('FAKE — Outgoing email, FedEx/USPS labels.'), text ?? '');
	assert.match(text ?? '', /nothing here sends mail, ships a parcel or charges a card/);
});

test('no fakes, no ribbon', () => {
	for (const fakes of [undefined, null, [], {}, 'mail_out', [{ slug: 'x' }], [{ label: ' ' }]]) {
		assert.equal(fakeRibbon(fakes), null, JSON.stringify(fakes));
	}
});
