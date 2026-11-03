// Authority list for staff pickers (compose cards on /review and
// /dashboard). The public `GET /apostille/api/v1/authorities` read,
// shared by every picker on the page: one request, cached until a
// failure clears it so "Try again" re-reads.
import { getJson } from './admin-api.js';

export interface AuthorityOption {
	slug: string;
	name: string;
	country: string;
	region: string;
	/** Accepted facts on the public dossier (the compose precondition). */
	fact_count: number;
	kind: string | null;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Loose = any;

let cached: Promise<AuthorityOption[]> | null = null;

function parse(body: Loose): AuthorityOption[] {
	const rows: Loose[] = Array.isArray(body?.authorities) ? body.authorities : Array.isArray(body) ? body : [];
	return rows
		.filter((r) => r && typeof r.slug === 'string' && r.slug)
		.map((r) => ({
			slug: r.slug,
			name: typeof r.name === 'string' && r.name ? r.name : r.slug,
			country: typeof r.country === 'string' ? r.country : '',
			region: typeof r.region === 'string' ? r.region : '',
			fact_count: typeof r.fact_count === 'number' ? r.fact_count : 0,
			kind: typeof r.kind === 'string' ? r.kind : null
		}))
		.sort((a, b) => a.name.localeCompare(b.name));
}

export function fetchAuthorities(): Promise<AuthorityOption[]> {
	if (!cached) {
		cached = getJson<Loose>('/authorities', 'Authorities')
			.then(parse)
			.catch((err) => {
				cached = null;
				throw err;
			});
	}
	return cached;
}
