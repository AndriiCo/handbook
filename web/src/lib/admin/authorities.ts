// Authority list for staff pickers (compose cards on /review and
// /dashboard, /calls, /email, staff-order names). The staff index
// `GET /apostille/api/v1/admin/authorities` (P358): the public
// `/authorities` route is not served on the admin host. Shared by
// every picker on the page: one request, cached until a failure
// clears it so "Try again" re-reads.
import { getJson } from './admin-api.js';

export interface AuthorityOption {
	slug: string;
	name: string;
	country: string;
	region: string;
	/** Accepted facts on the public dossier (the compose precondition). */
	fact_count: number;
	kind: string | null;
	/** Drafts waiting in /review (staff index only). */
	proposed_count: number;
	/** Email, phone and webform channels on file (staff index only). */
	contact_count: number;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Loose = any;

let cached: Promise<AuthorityOption[]> | null = null;

const count = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);

function parse(body: Loose): AuthorityOption[] {
	const rows: Loose[] = Array.isArray(body?.authorities) ? body.authorities : Array.isArray(body) ? body : [];
	return rows
		.filter((r) => r && typeof r.slug === 'string' && r.slug)
		.map((r) => ({
			slug: r.slug,
			name: typeof r.name === 'string' && r.name ? r.name : r.slug,
			country: typeof r.country === 'string' ? r.country : '',
			region: typeof r.region === 'string' ? r.region : '',
			fact_count: count(r.fact_count),
			kind: typeof r.kind === 'string' ? r.kind : null,
			proposed_count: count(r.proposed_count),
			contact_count: count(r.contact_count)
		}))
		.sort((a, b) => a.name.localeCompare(b.name));
}

export function fetchAuthorities(): Promise<AuthorityOption[]> {
	if (!cached) {
		cached = getJson<Loose>('/admin/authorities', 'Authorities')
			.then(parse)
			.catch((err) => {
				cached = null;
				throw err;
			});
	}
	return cached;
}
