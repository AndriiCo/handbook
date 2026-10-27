// Synthetic fixtures for the clickable UI mock (route group `(mock)`).
// NEVER real data: every value below is invented for layout review.
// Zero backend calls — pages import only this module plus existing
// display components from $lib/public-ui.

export interface MockAuthority {
	slug: string;
	name: string;
	kind: string;
	tier: string;
	verified: string | null;
	requirements: string[];
	forms: { name: string; revision: string; hash: string }[];
	fees: { label: string; amount: number }[];
	visit: string[];
	mail: string[];
}

export const AUTHORITIES: MockAuthority[] = [
	{
		slug: 'demo-secretary-north',
		name: 'Demo Secretary of State — North',
		kind: 'Secretary of State · Apostille',
		tier: 'Official',
		verified: '2026-10-20',
		requirements: [
			'Original document signed by a commissioned notary of this state.',
			'Notary commission must be active on the signing date.',
			'Corporate documents need a good-standing certificate dated within 90 days.'
		],
		forms: [
			{ name: 'APO-100 Request for Apostille', revision: 'rev. 2026-04', hash: 'sha256:demo11…' },
			{ name: 'APO-102 Cover Sheet (mail-in)', revision: 'rev. 2025-11', hash: 'sha256:demo22…' }
		],
		fees: [
			{ label: 'Authority fee (cited passthrough)', amount: 8 },
			{ label: 'Per certified copy', amount: 2 }
		],
		visit: [
			'Walk-in counter, 8:30–16:00 Mon–Fri. Same-day before 14:00.',
			'Photo ID required. Parking garage one block east.'
		],
		mail: [
			'Mail to PO Box 9000, Capital City. Include a self-addressed prepaid envelope.',
			'Current turnaround: 5–7 business days after receipt.'
		]
	},
	{
		slug: 'demo-consulate-harbor',
		name: 'Demo Consulate — Harbor City',
		kind: 'Consular office · Authentication',
		tier: 'Official',
		verified: '2026-09-30',
		requirements: [
			'Document must already carry a state apostille or federal authentication.',
			'One photocopy of every page, single-sided.',
			'Appointment booked online; walk-ins are not served.'
		],
		forms: [{ name: 'LEG-40 Legalization Application', revision: 'rev. 2026-01', hash: 'sha256:demo33…' }],
		fees: [
			{ label: 'Authority fee (cited passthrough)', amount: 25 },
			{ label: 'Per certified copy', amount: 25 }
		],
		visit: ['By appointment only. Arrive 15 minutes early; phones stay at reception.'],
		mail: ['Mail-in is not offered for first-time applicants — in person only.']
	},
	{
		slug: 'demo-county-clerk-east',
		name: 'Demo County Clerk — East',
		kind: 'County clerk · Certification',
		tier: 'Community',
		verified: null,
		requirements: ['Call ahead: certification hours change monthly (unverified report).'],
		forms: [],
		fees: [{ label: 'Authority fee (cited passthrough)', amount: 5 }],
		visit: ['Counter hours unverified — confirm by phone before visiting.'],
		mail: []
	}
];

export function searchMock(query: string): MockAuthority[] {
	const q = query.trim().toLowerCase();
	if (!q) return [];
	return AUTHORITIES.filter(
		(a) =>
			a.name.toLowerCase().includes(q) ||
			a.kind.toLowerCase().includes(q) ||
			a.slug.includes(q.replace(/\s+/g, '-'))
	);
}

export function findMock(slug: string): MockAuthority | null {
	return AUTHORITIES.find((a) => a.slug === slug) ?? null;
}

export interface MockOrder {
	id: string;
	authority: string;
	docType: string;
	status: 'Draft' | 'Submitted' | 'In review' | 'Ready to file' | 'Completed';
	updated: string;
}

export const ORDERS: MockOrder[] = [
	{ id: 'ORD-1042', authority: 'Demo Secretary of State — North', docType: 'Birth certificate', status: 'In review', updated: '2026-10-26' },
	{ id: 'ORD-1041', authority: 'Demo Consulate — Harbor City', docType: 'Corporate document', status: 'Ready to file', updated: '2026-10-24' },
	{ id: 'ORD-1040', authority: 'Demo Secretary of State — North', docType: 'Marriage certificate', status: 'Completed', updated: '2026-10-18' },
	{ id: 'ORD-1039', authority: 'Demo County Clerk — East', docType: 'Other', status: 'Draft', updated: '2026-10-15' }
];

export interface MockUpload {
	name: string;
	size: string;
	state: 'ready' | 'scanning' | 'flagged';
	note: string;
}

export const UPLOADS: MockUpload[] = [
	{ name: 'birth-cert-scan.pdf', size: '2.4 MB', state: 'ready', note: 'Legible, all corners visible.' },
	{ name: 'notary-page-photo.jpg', size: '3.1 MB', state: 'scanning', note: 'Precheck running…' },
	{ name: 'old-copy-blurry.jpg', size: '1.1 MB', state: 'flagged', note: 'Blurry seal — retake suggested.' }
];

export type PrecheckState = 'pass' | 'warn' | 'fail';

export interface PrecheckItem {
	label: string;
	state: PrecheckState;
	detail: string;
}

export const PRECHECK: PrecheckItem[] = [
	{ label: 'Resolution ≥ 300 DPI', state: 'pass', detail: '412 DPI measured.' },
	{ label: 'All four corners visible', state: 'pass', detail: 'Edges detected.' },
	{ label: 'Seal legible', state: 'warn', detail: 'Low contrast — retake in daylight.' },
	{ label: 'No glare over signature', state: 'fail', detail: 'Glare covers the signature line.' }
];

export interface TriageItem {
	id: number;
	authority: string;
	topic: string;
	statement: string;
	conflict: boolean;
	conflicts: { id: number; status: string; statement: string }[];
}

export const TRIAGE: TriageItem[] = [
	{
		id: 501,
		authority: 'demo-secretary-north',
		topic: 'fee',
		statement: 'Apostille fee is USD $8.00 per document — cited from clerk phone call 2026-10-27.',
		conflict: false,
		conflicts: []
	},
	{
		id: 502,
		authority: 'demo-secretary-north',
		topic: 'mail',
		statement: 'Mail-in address is PO Box 9000 — cited from office notice 2026-10-20.',
		conflict: true,
		conflicts: [{ id: 411, status: 'accepted', statement: 'Mail-in address is PO Box 4500 (older notice).' }]
	},
	{
		id: 503,
		authority: 'demo-consulate-harbor',
		topic: 'hours',
		statement: 'Counter hours are 9:00–12:00 Mon–Thu — cited from portal screenshot 2026-10-22.',
		conflict: false,
		conflicts: []
	}
];

export const CONSOLE_TRANSCRIPT: { speaker: string; line: string }[] = [
	{ speaker: 'Staff', line: 'Good morning, Demo Secretary office — how can I help?' },
	{ speaker: 'Caller', line: 'Hi — what is the current apostille fee per document?' },
	{ speaker: 'Staff', line: 'Eight dollars per document. Same-day if we receive it before 2 PM.' },
	{ speaker: 'Caller', line: 'And the mail-in box — is it still PO Box 9000?' },
	{ speaker: 'Staff', line: 'Yes, Box 9000. Please include a prepaid return envelope.' }
];

export interface MockConfigRow {
	key: string;
	type: 'bool' | 'text' | 'secret';
	value?: boolean | string;
	set?: boolean;
	hint: string;
}

export const CONFIG: MockConfigRow[] = [
	{ key: 'handbook.public_search', type: 'bool', value: true, hint: 'Enable the public search index.' },
	{ key: 'handbook.support_email', type: 'text', value: 'help@example.invalid', hint: 'Shown on error pages.' },
	{ key: 'billing.webhook_secret', type: 'secret', set: true, hint: 'Write-only: entering a value replaces it.' },
	{ key: 'models.api_key', type: 'secret', set: false, hint: 'Write-only: never displayed after save.' }
];

export interface MockInvoice {
	id: string;
	date: string;
	amount: string;
	status: 'Paid' | 'Open';
}

export const INVOICES: MockInvoice[] = [
	{ id: 'INV-2026-118', date: '2026-10-20', amount: 'USD $33.00', status: 'Paid' },
	{ id: 'INV-2026-121', date: '2026-10-26', amount: 'USD $8.00', status: 'Open' }
];

export const DEMO_REFS = ['demo-customer-001', 'demo-customer-002'];

export function money(amount: number): string {
	return `USD $${amount.toFixed(2)}`;
}
