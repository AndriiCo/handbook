import test from 'node:test';
import assert from 'node:assert/strict';
import {
	UPLOAD_TTL_MS,
	clearDraft,
	draftKey,
	loadDraft,
	parseDraft,
	photoKind,
	quoteLine,
	saveDraft
} from '../src/lib/public-ui/report-draft.js';

const mem = () => {
	const m = new Map();
	return {
		getItem: (k) => (m.has(k) ? m.get(k) : null),
		setItem: (k, v) => m.set(k, String(v)),
		removeItem: (k) => m.delete(k),
		m
	};
};
const ID = 'a'.repeat(32);

test('draftKey is one draft per office', () => {
	assert.equal(draftKey('USA-WA-SOS '), 'hb_report_draft:usa-wa-sos');
	assert.equal(draftKey(''), 'hb_report_draft:any');
	assert.equal(draftKey(null), 'hb_report_draft:any');
});

test('a draft round-trips and an empty one clears the key', () => {
	const s = mem();
	const key = draftKey('usa-wa-sos');
	const at = saveDraft(s, key, { authority: 'usa-wa-sos', body: 'Fee is $20 now', uploads: [] }, 5);
	assert.equal(at, 5);
	assert.deepEqual(loadDraft(s, key, 6), {
		authority: 'usa-wa-sos',
		body: 'Fee is $20 now',
		uploads: [],
		savedAt: 5
	});
	assert.equal(saveDraft(s, key, { authority: 'usa-wa-sos', body: '  ', uploads: [] }), null);
	assert.equal(s.m.has(key), false);
});

test('expired or malformed uploads drop out of a restored draft', () => {
	const raw = JSON.stringify({
		body: 'x',
		uploads: [
			{ id: ID, name: 'a.jpg', size: 10, at: 1000 },
			{ id: 'b'.repeat(32), name: 'old.jpg', size: 10, at: 0 },
			{ id: '../etc', name: 'bad', size: 1, at: 1000 }
		]
	});
	const d = parseDraft(raw, UPLOAD_TTL_MS + 500);
	assert.deepEqual(
		d?.uploads.map((u) => u.id),
		[ID]
	);
	assert.equal(parseDraft('{not json'), null);
	assert.equal(parseDraft(null), null);
});

test('blocked storage never throws', () => {
	const boom = {
		getItem() {
			throw new Error('blocked');
		},
		setItem() {
			throw new Error('blocked');
		},
		removeItem() {
			throw new Error('blocked');
		}
	};
	assert.equal(loadDraft(boom, 'k'), null);
	assert.equal(saveDraft(boom, 'k', { authority: '', body: 'x', uploads: [] }), null);
	clearDraft(boom, 'k');
	assert.equal(saveDraft(null, 'k', { authority: '', body: 'x', uploads: [] }), null);
});

test('quoteLine folds whitespace and caps length', () => {
	assert.equal(quoteLine('  Fee:\n  $15 per\tdocument '), '> "Fee: $15 per document"\n');
	assert.equal(quoteLine(''), '');
	assert.equal(quoteLine('x'.repeat(400)).length, '> ""\n'.length + 300);
});

test('photoKind reads magic bytes, not names', () => {
	assert.equal(photoKind(new Uint8Array([0xff, 0xd8, 0xff, 0xe0])), 'jpeg');
	assert.equal(photoKind(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])), 'png');
	assert.equal(photoKind(new TextEncoder().encode('RIFF\0\0\0\0WEBP')), 'webp');
	assert.equal(photoKind(new TextEncoder().encode('GIF89a')), 'gif');
	assert.equal(photoKind(new TextEncoder().encode('%PDF-1.7')), null);
});
