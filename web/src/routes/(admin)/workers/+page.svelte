<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { resolve } from '$app/paths';
	import { fly } from 'svelte/transition';
	import { toast } from 'svelte-sonner';
	import Notice from '#lib/public-ui/Notice.svelte';
	import Heading from '#lib/components/brand/Heading.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import AuthorityPicker from '#lib/admin/AuthorityPicker.svelte';
	import TargetField from '#lib/admin/TargetField.svelte';
	import type { AuthorityOption } from '#lib/admin/authorities.js';
	import { csrfHeaders } from '#lib/admin/config-client.js';
	import { authError, readErrorBody } from '#lib/admin/http-errors.js';
	import {
		classifyEntry,
		defaultCountryFor,
		entryKey,
		extractEntries,
		formatSaved,
		planSubmit,
		type Entry
	} from '#lib/admin/contact-entry.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import LandmarkIcon from '@lucide/svelte/icons/landmark';
	import LinkIcon from '@lucide/svelte/icons/link';
	import MailIcon from '@lucide/svelte/icons/mail';
	import PhoneIcon from '@lucide/svelte/icons/phone';
	import RadarIcon from '@lucide/svelte/icons/radar';
	import InboxIcon from '@lucide/svelte/icons/inbox';

	// Workers management against the admin workers API (admin-workers.md
	// §8): status, per-authority ingest state, capture list with retry,
	// one-off fetch, manual watch sweep. Admin build only; same literal
	// /apostille root as the triage page. The sweep takes one authority
	// and one typed field: URLs go to POST /workers/sweep, office emails
	// and phones to POST /admin/first-source/{slug}/contacts.
	const api = (path: string) => `/apostille/api/v1${path}`;

	interface Status {
		mode: string;
		backend: string;
		timeoutSecs: number;
		maxBody: number;
		captures: number;
		sources: number;
		failures: number;
		lastSweep: { at: number; actor: string; summary: string } | null;
	}

	interface Authority {
		slug: string;
		name: string;
		sources: number;
		captures: number;
		lastCaptureAt: number | null;
		failures: number;
		lastAnalysisAt: number | null;
		lastPromptVersion: string | null;
	}

	interface Capture {
		id: number;
		authority: string;
		sourceId: number | null;
		uri: string | null;
		capturedAt: number;
		kind: string;
		bytes: number;
		sha: string;
		trust: string;
		busy: boolean;
		note: string | null;
	}

	interface Contact {
		id: number;
		kind: string;
		value: string;
		details: string;
	}

	type Outcome = 'created' | 'duplicate' | 'failed' | 'skipped' | 'saved' | 'on_file' | 'rejected';

	interface ResultRow {
		key: string;
		kind: 'url' | 'email' | 'phone';
		target: string;
		outcome: Outcome;
		text: string;
	}

	const SOURCE_KINDS = [
		{ value: 'website_official', label: 'Official website' },
		{ value: 'website_thirdparty', label: 'Third-party website' }
	];
	const TRUSTS = [
		{ value: 'official', label: 'Official' },
		{ value: 'staff_verified', label: 'Staff verified' },
		{ value: 'community', label: 'Community' }
	];
	const CAPTURE_KINDS = ['warc', 'file', 'scan', 'photo', 'video', 'audio', 'email', 'transcript', 'comment'];
	const CAPTURE_TRUSTS = ['official', 'staff_verified', 'community', 'rejected'];

	let status = $state<Status | null>(null);
	let authorities = $state<Authority[]>([]);
	let captures = $state<Capture[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let refreshing = $state(false);

	let filterAuthority = $state('');
	let filterKind = $state('all');
	let filterTrust = $state('all');
	let capturesState = $state<'ready' | 'loading' | 'failed'>('ready');
	let capturesError = $state<string | null>(null);

	let fetchSlug = $state('');
	let fetchUrl = $state('');
	let fetchKind = $state('website_official');
	let fetchTrust = $state('official');
	let fetchBusy = $state(false);
	let fetchNote = $state<string | null>(null);

	let pickerRows = $state<AuthorityOption[]>([]);
	let sweepSlug = $state('');
	let sweepEntries = $state<Entry[]>([]);
	let sweepBusy = $state(false);
	let sweepNote = $state<string | null>(null);
	let sweepResults = $state<ResultRow[]>([]);
	let sweepSummary = $state<string | null>(null);
	let onFile = $state<Contact[]>([]);
	let onFileState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let onFileError = $state<string | null>(null);
	let targetField = $state<{ flush: () => void; focus: () => void } | null>(null);
	let reduceMotion = $state(false);

	const motion = $derived(reduceMotion ? 0 : 150);
	const sweepRow = $derived(pickerRows.find((r) => r.slug === sweepSlug) ?? null);
	const sweepCountry = $derived(defaultCountryFor(sweepRow ?? (sweepSlug ? { slug: sweepSlug } : null)));
	const savedKeys = $derived(new Set(onFile.map((c) => formatSaved(c).key)));
	const plan = $derived(planSubmit(sweepEntries, savedKeys));
	const sweepLabel = $derived.by(() => {
		const parts: string[] = [];
		if (plan.urls.length) parts.push(`Fetch ${plan.urls.length} URL${plan.urls.length === 1 ? '' : 's'}`);
		if (plan.contacts.length)
			parts.push(`${parts.length ? 'save' : 'Save'} ${plan.contacts.length} contact${plan.contacts.length === 1 ? '' : 's'}`);
		return parts.length ? parts.join(' · ') : 'Run sweep';
	});
	const canSweep = $derived(!!sweepSlug && !sweepBusy && plan.urls.length + plan.contacts.length > 0);

	function authFailed(res: Response): boolean {
		return res.status === 401 || res.status === 403;
	}

	// The server's own reason (session expired, request token
	// rejected, IP not allowed) — a 403 is not always a lost session.
	async function authNote(res: Response): Promise<string> {
		return authError(res.status, await readErrorBody(res)) ?? 'The admin host refused the request.';
	}

	async function post(path: string, body: unknown): Promise<Response> {
		return fetch(api(path), {
			method: 'POST',
			headers: { 'content-type': 'application/json', ...csrfHeaders() },
			credentials: 'same-origin',
			body: JSON.stringify(body)
		});
	}

	function normalizeStatus(data: any): Status | null {
		if (!data || data.mode !== 'manual') return null;
		const q = data.queue ?? {};
		const f = data.fetch ?? {};
		const sweep = data.last_sweep;
		return {
			mode: 'manual',
			backend: typeof f.backend === 'string' ? f.backend : '?',
			timeoutSecs: typeof f.timeout_secs === 'number' ? f.timeout_secs : 0,
			maxBody: typeof f.max_body_bytes === 'number' ? f.max_body_bytes : 0,
			captures: typeof q.captures === 'number' ? q.captures : 0,
			sources: typeof q.sources === 'number' ? q.sources : 0,
			failures: typeof q.open_fetch_failures === 'number' ? q.open_fetch_failures : 0,
			lastSweep:
				sweep && typeof sweep.at === 'number'
					? {
							at: sweep.at,
							actor: typeof sweep.actor === 'string' ? sweep.actor : '?',
							summary: typeof sweep.summary === 'string' ? sweep.summary : ''
						}
					: null
		};
	}

	function normalizeAuthority(row: any): Authority | null {
		if (!row || typeof row.slug !== 'string') return null;
		return {
			slug: row.slug,
			name: typeof row.name === 'string' ? row.name : row.slug,
			sources: typeof row.sources === 'number' ? row.sources : 0,
			captures: typeof row.captures === 'number' ? row.captures : 0,
			lastCaptureAt: typeof row.last_capture_at === 'number' ? row.last_capture_at : null,
			failures: typeof row.open_fetch_failures === 'number' ? row.open_fetch_failures : 0,
			lastAnalysisAt: typeof row.last_analysis_at === 'number' ? row.last_analysis_at : null,
			lastPromptVersion:
				typeof row.last_prompt_version === 'string' ? row.last_prompt_version : null
		};
	}

	function normalizeCapture(row: any): Capture | null {
		if (!row || typeof row.id !== 'number') return null;
		return {
			id: row.id,
			authority: typeof row.authority_slug === 'string' ? row.authority_slug : '?',
			sourceId: typeof row.source_id === 'number' ? row.source_id : null,
			uri: typeof row.source_uri === 'string' ? row.source_uri : null,
			capturedAt: typeof row.captured_at === 'number' ? row.captured_at : 0,
			kind: typeof row.kind === 'string' ? row.kind : '?',
			bytes: typeof row.bytes_raw === 'number' ? row.bytes_raw : 0,
			sha: typeof row.sha256 === 'string' ? row.sha256 : '',
			trust: typeof row.trust === 'string' ? row.trust : '?',
			busy: false,
			note: null
		};
	}

	function normalizeContacts(list: unknown): Contact[] {
		return (Array.isArray(list) ? list : [])
			.filter((c: any) => c && typeof c.value === 'string' && typeof c.kind === 'string')
			.map((c: any) => ({
				id: typeof c.id === 'number' ? c.id : 0,
				kind: c.kind,
				value: c.value,
				details: typeof c.details === 'string' ? c.details : ''
			}));
	}

	function isCapture(value: Capture | null): value is Capture {
		return value !== null;
	}

	function isAuthority(value: Authority | null): value is Authority {
		return value !== null;
	}

	function capturesQuery(): string {
		const params = new URLSearchParams();
		if (filterAuthority) params.set('authority', filterAuthority);
		if (filterKind !== 'all') params.set('kind', filterKind);
		if (filterTrust !== 'all') params.set('trust', filterTrust);
		params.set('limit', '100');
		return `?${params}`;
	}

	async function load(soft = false) {
		if (soft) refreshing = true;
		else loadState = 'loading';
		loadError = null;
		try {
			const [statusRes, authRes, capRes] = await Promise.all([
				fetch(api('/workers/status'), { credentials: 'same-origin' }),
				fetch(api('/workers/authorities'), { credentials: 'same-origin' }),
				fetch(api(`/workers/captures${capturesQuery()}`), { credentials: 'same-origin' })
			]);
			if ([statusRes, authRes, capRes].some(authFailed)) {
				// Auth failures show the server's answer verbatim (the
				// 403 body names the detected IP) via the shared helper.
				const first = [statusRes, authRes, capRes].find((r) => authFailed(r)) ?? statusRes;
				loadError = authError(first.status, await readErrorBody(first));
			} else if (!statusRes.ok || !authRes.ok || !capRes.ok) {
				throw new Error('one section failed to load');
			} else {
				status = normalizeStatus(await statusRes.json());
				const authData = await authRes.json();
				authorities = (Array.isArray(authData?.authorities) ? authData.authorities : [])
					.map(normalizeAuthority)
					.filter(isAuthority);
				const capData = await capRes.json();
				captures = (Array.isArray(capData?.captures) ? capData.captures : [])
					.map(normalizeCapture)
					.filter(isCapture);
				if (!status) throw new Error('unrecognized status shape');
			}
			if (!soft || loadError) loadState = loadError ? 'failed' : 'ready';
		} catch {
			loadError = 'Workers service unreachable — is the API server running?';
			if (!soft) loadState = 'failed';
		} finally {
			refreshing = false;
		}
	}

	// Filter changes re-read the capture list only; the pickers can
	// only hold known slugs, so a filter never reads as an outage.
	async function loadCaptures() {
		capturesState = 'loading';
		capturesError = null;
		try {
			const res = await fetch(api(`/workers/captures${capturesQuery()}`), { credentials: 'same-origin' });
			if (authFailed(res)) {
				capturesError = await authNote(res);
				capturesState = 'failed';
				return;
			}
			if (!res.ok) {
				capturesError = `Captures failed to load: HTTP ${res.status}.`;
				capturesState = 'failed';
				return;
			}
			const data = await res.json();
			captures = (Array.isArray(data?.captures) ? data.captures : []).map(normalizeCapture).filter(isCapture);
			capturesState = 'ready';
		} catch {
			capturesError = 'Workers service unreachable — is the API server running?';
			capturesState = 'failed';
		}
	}

	async function loadOnFile(slug: string) {
		onFileState = 'loading';
		onFileError = null;
		try {
			const res = await fetch(api(`/admin/first-source/${encodeURIComponent(slug)}/questions`), {
				credentials: 'same-origin'
			});
			if (slug !== sweepSlug) return;
			if (authFailed(res)) {
				onFileError = await authNote(res);
				onFileState = 'failed';
				return;
			}
			if (!res.ok) {
				const body = await readErrorBody(res);
				onFileError = `Contacts on file could not load (HTTP ${res.status}${typeof body?.error === 'string' ? `: ${body.error}` : ''}).`;
				onFileState = 'failed';
				return;
			}
			const data = await res.json();
			onFile = normalizeContacts(data?.contacts);
			onFileState = 'ready';
		} catch {
			if (slug !== sweepSlug) return;
			onFileError = 'The admin host could not be reached for contacts on file.';
			onFileState = 'failed';
		}
	}

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		// Deep link: ?authority=<slug>&targets=<list> prefills the sweep
		// (other staff screens link here with the URLs to re-fetch).
		const params = new URLSearchParams(window.location.search);
		const slug = params.get('authority');
		const targets = params.get('targets');
		if (slug) sweepSlug = slug;
		if (targets) sweepEntries = dedupe(extractEntries(targets, defaultCountryFor({ slug: slug ?? '' })).entries);
		void load().then(async () => {
			if (window.location.hash === '#sweep') {
				await tick();
				document.getElementById('sweep')?.scrollIntoView({ block: 'start' });
			}
		});
	});

	// Contacts on file follow the chosen authority.
	$effect(() => {
		const slug = sweepSlug;
		untrack(() => {
			onFile = [];
			sweepResults = [];
			sweepSummary = null;
			sweepNote = null;
			if (slug) void loadOnFile(slug);
			else onFileState = 'idle';
		});
	});

	// The authority filter re-reads captures when it changes (the
	// first run is the page load itself).
	let filterSeen = '';
	$effect(() => {
		const slug = filterAuthority;
		untrack(() => {
			if (slug === filterSeen) return;
			filterSeen = slug;
			if (loadState === 'ready') void loadCaptures();
		});
	});

	// Numbers typed before the authority was chosen are re-read with
	// its country (a bare 360 555 0100 needs one to be valid).
	$effect(() => {
		const country = sweepCountry;
		untrack(() => {
			if (sweepEntries.length === 0) return;
			sweepEntries = dedupe(sweepEntries.map((e) => (e.kind === 'url' || e.kind === 'email' ? e : classifyEntry(e.raw, country))));
		});
	});

	function dedupe(list: Entry[]): Entry[] {
		const seen = new Set<string>();
		return list.filter((e) => {
			const key = entryKey(e);
			if (seen.has(key)) return false;
			seen.add(key);
			return true;
		});
	}

	async function retry(item: Capture) {
		item.busy = true;
		item.note = null;
		try {
			const res = await post(`/workers/captures/${item.id}/retry`, {});
			if (authFailed(res)) {
				item.note = await authNote(res);
			} else if (res.status === 409) {
				item.note = 'No source to re-fetch (unattributed capture).';
			} else if (res.status === 502) {
				const data = await res.json().catch(() => null);
				item.note = `Fetch failed — failure task recorded (${data?.error ?? 'unknown'}).`;
			} else if (!res.ok) {
				item.note = `Retry failed: HTTP ${res.status}. Nothing changed.`;
			} else {
				const data = await res.json();
				if (data?.outcome === 'skipped') {
					item.note = `Skipped: ${data?.reason ?? 'robots disallow'}.`;
				} else {
					item.note = `${data?.outcome ?? 'done'} → capture #${data?.capture_id ?? '?'}.`;
				}
			}
		} catch {
			item.note = 'Workers service unreachable — is the API server running?';
		} finally {
			item.busy = false;
		}
	}

	async function runFetch() {
		fetchNote = null;
		if (!fetchSlug) {
			fetchNote = 'Choose the authority this URL belongs to.';
			return;
		}
		const target = classifyEntry(fetchUrl);
		if (target.kind !== 'url') {
			fetchNote = 'Enter an absolute http(s) address.';
			return;
		}
		fetchBusy = true;
		try {
			const res = await post('/workers/fetch', {
				slug: fetchSlug,
				url: target.value,
				source_kind: fetchKind,
				trust: fetchTrust
			});
			if (authFailed(res)) {
				fetchNote = await authNote(res);
			} else if (res.status === 404) {
				fetchNote = 'Unknown authority slug.';
			} else if (res.status === 422) {
				fetchNote = 'Rejected (422) — URL must be absolute http(s).';
			} else if (res.status === 502) {
				const data = await res.json().catch(() => null);
				fetchNote = `Fetch failed — failure task recorded (${data?.error ?? 'unknown'}).`;
			} else if (!res.ok) {
				fetchNote = `Fetch failed: HTTP ${res.status}. Nothing changed.`;
			} else {
				const data = await res.json();
				fetchNote =
					data?.outcome === 'skipped'
						? `Skipped: ${data?.reason ?? 'robots disallow'}.`
						: `${data?.outcome ?? 'done'} → capture #${data?.capture_id ?? '?'}.`;
				fetchUrl = '';
				await load(true);
			}
		} catch {
			fetchNote = 'Workers service unreachable — is the API server running?';
		} finally {
			fetchBusy = false;
		}
	}

	const OUTCOME_TEXT: Record<string, string> = {
		created: 'Fetched',
		duplicate: 'Unchanged',
		skipped: 'Robots disallow',
		failed: 'Fetch failed'
	};

	async function sweepUrls(slug: string, urls: Entry[]): Promise<{ rows: ResultRow[]; sent: Set<string>; note: string | null; summary: string | null }> {
		const rows: ResultRow[] = [];
		const sent = new Set<string>();
		const res = await post('/workers/sweep', { items: urls.map((u) => ({ slug, url: u.value })) });
		if (authFailed(res)) return { rows, sent, note: await authNote(res), summary: null };
		if (res.status === 404 || res.status === 422) {
			const data = await res.json().catch(() => null);
			return { rows, sent, note: `Sweep rejected (${res.status}): ${data?.error ?? 'bad items'}. Nothing fetched.`, summary: null };
		}
		if (!res.ok) return { rows, sent, note: `Sweep failed: HTTP ${res.status}. Nothing fetched.`, summary: null };
		const data = await res.json();
		for (const row of Array.isArray(data?.results) ? data.results : []) {
			const url = typeof row?.url === 'string' ? row.url : '?';
			const outcome = (['created', 'duplicate', 'failed', 'skipped'].includes(row?.outcome) ? row.outcome : 'failed') as Outcome;
			const id = typeof row?.capture_id === 'number' ? row.capture_id : null;
			const detail = typeof row?.detail === 'string' ? row.detail : null;
			let text = OUTCOME_TEXT[outcome];
			if (outcome === 'created' && id !== null) text = `Fetched → capture #${id}`;
			if (outcome === 'duplicate' && id !== null) text = `Unchanged → capture #${id}`;
			if (outcome === 'failed') text = `${detail ?? 'Fetch failed'} · failure task recorded`;
			if (outcome === 'skipped' && detail) text = `Skipped: ${detail}`;
			rows.push({ key: `url:${url}`, kind: 'url', target: url, outcome, text });
			if (outcome !== 'failed') sent.add(`url:${url}`);
		}
		const s = data?.summary;
		const summary = s
			? `${s.total} URL${s.total === 1 ? "" : "s"} swept: ${s.created} new, ${s.duplicate} unchanged, ${s.failed} failed, ${s.skipped} skipped`
			: null;
		return { rows, sent, note: null, summary };
	}

	async function saveContacts(slug: string, list: Entry[]): Promise<{ rows: ResultRow[]; sent: Set<string>; note: string | null; contacts: Contact[] | null }> {
		const rows: ResultRow[] = [];
		const sent = new Set<string>();
		let contacts: Contact[] | null = null;
		const before = new Set(onFile.map((c) => formatSaved(c).key));
		for (const c of list) {
			const kind = c.kind === 'phone' ? 'phone' : 'email';
			const key = entryKey(c);
			const res = await post(`/admin/first-source/${encodeURIComponent(slug)}/contacts`, {
				kind,
				value: c.value,
				details: c.details
			});
			if (authFailed(res)) {
				// One auth answer covers the rest: stop, keep every chip.
				return { rows, sent, note: await authNote(res), contacts };
			}
			const data = await res.json().catch(() => null);
			if (!res.ok) {
				const why = typeof data?.error === 'string' ? data.error : `HTTP ${res.status}`;
				rows.push({ key, kind, target: c.display, outcome: 'rejected', text: `Not saved: ${why}` });
				continue;
			}
			if (Array.isArray(data?.contacts)) contacts = normalizeContacts(data.contacts);
			const existed = data?.created === false || (data?.created === undefined && before.has(key));
			rows.push({
				key,
				kind,
				target: c.display,
				outcome: existed ? 'on_file' : 'saved',
				text: existed ? 'Already on file' : 'Saved to this office'
			});
			sent.add(key);
		}
		return { rows, sent, note: null, contacts };
	}

	async function runSweep() {
		targetField?.flush();
		await tick();
		sweepNote = null;
		if (!sweepSlug) {
			sweepNote = 'Choose the issuing office first.';
			return;
		}
		const current = planSubmit(sweepEntries, savedKeys);
		if (current.urls.length + current.contacts.length === 0) {
			sweepNote = current.invalid.length ? 'Fix or remove the flagged entries first.' : 'Add a URL, an email or a phone number.';
			targetField?.focus();
			return;
		}
		const slug = sweepSlug;
		sweepBusy = true;
		sweepResults = [];
		sweepSummary = null;
		try {
			const [urlPart, contactPart] = await Promise.allSettled([
				current.urls.length ? sweepUrls(slug, current.urls) : Promise.resolve(null),
				current.contacts.length ? saveContacts(slug, current.contacts) : Promise.resolve(null)
			]);
			const rows: ResultRow[] = [];
			const sent = new Set<string>();
			const notes: string[] = [];
			if (urlPart.status === 'fulfilled' && urlPart.value) {
				rows.push(...urlPart.value.rows);
				urlPart.value.sent.forEach((k) => sent.add(k));
				if (urlPart.value.note) notes.push(urlPart.value.note);
				sweepSummary = urlPart.value.summary;
			} else if (urlPart.status === 'rejected') {
				notes.push('Sweep: workers service unreachable — nothing fetched.');
			}
			if (contactPart.status === 'fulfilled' && contactPart.value) {
				rows.push(...contactPart.value.rows);
				contactPart.value.sent.forEach((k) => sent.add(k));
				if (contactPart.value.note && !notes.includes(contactPart.value.note)) notes.push(contactPart.value.note);
				if (contactPart.value.contacts) onFile = contactPart.value.contacts;
			} else if (contactPart.status === 'rejected') {
				notes.push('Contacts: the admin host could not be reached — nothing saved.');
			}
			sweepResults = rows;
			sweepNote = notes.length ? notes.join(' ') : null;
			// Sent chips leave the field; failed and invalid ones stay.
			sweepEntries = sweepEntries.filter((e) => !sent.has(entryKey(e)) && !(e.kind === 'url' && sent.has(`url:${e.value}`)));
			const ok = rows.filter((r) => r.outcome !== 'failed' && r.outcome !== 'rejected').length;
			const bad = rows.length - ok;
			if (rows.length && bad === 0 && notes.length === 0) toast.success(`${ok} done for ${slug}.`);
			else if (rows.length) toast.warning(`${ok} done, ${bad + (notes.length ? 1 : 0)} need a look.`);
			else if (notes.length) toast.error(notes[0]);
			if (current.urls.length) await load(true);
		} finally {
			sweepBusy = false;
		}
	}

	function stamp(at: number | null): string {
		if (at === null || at <= 0) return '—';
		return new Date(at * 1000).toISOString().replace('T', ' ').slice(0, 16) + 'Z';
	}

	function kib(bytes: number): string {
		if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(bytes % (1024 * 1024) === 0 ? 0 : 1)} MiB`;
		if (bytes >= 1024) return `${Math.round(bytes / 1024)} KiB`;
		return `${bytes} B`;
	}

	function labelFor(list: { value: string; label: string }[], value: string): string {
		return list.find((x) => x.value === value)?.label ?? value;
	}

	function human(value: string): string {
		return value.replace(/_/g, ' ');
	}

	// Full class strings per outcome (the tailwind checker reads literals).
	function outcomeCls(outcome: Outcome): string {
		const base = 'inline-flex h-6 flex-none items-center rounded-full px-2 text-xs font-medium';
		if (outcome === 'created' || outcome === 'saved') return `${base} bg-verified-bg text-verified`;
		if (outcome === 'skipped') return `${base} bg-changed-bg text-changed`;
		if (outcome === 'failed' || outcome === 'rejected') return `${base} bg-destructive/10 text-destructive`;
		return `${base} bg-surface-2 text-ink-muted`;
	}

	const OUTCOME_BADGE: Record<Outcome, string> = {
		created: 'New capture',
		duplicate: 'Unchanged',
		skipped: 'Skipped',
		failed: 'Failed',
		saved: 'Saved',
		on_file: 'On file',
		rejected: 'Rejected'
	};
</script>

<svelte:head>
	<title>Workers — Admin — Apostille Handbook</title>
</svelte:head>

<div class="mt-6 flex flex-col gap-6 pb-12">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<Heading level={1} eyebrow="Content">Workers</Heading>
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Manual drive over the URL download and analysis ingest paths. Every fetch runs now, inside the
				request; there is no background scheduler.
			</p>
		</div>
		<Button variant="outline" size="lg" onclick={() => load(true)} disabled={refreshing || loadState === 'loading'}>
			<RefreshCwIcon class={refreshing ? 'motion-safe:animate-spin' : ''} aria-hidden="true" />
			Refresh
		</Button>
	</header>

	{#if loadState === 'loading'}
		<div class="flex flex-col gap-6" role="status" aria-label="Loading worker status">
			<div class="grid grid-cols-2 gap-3 md:grid-cols-4">
				{#each [0, 1, 2, 3] as i (i)}
					<div class="flex flex-col gap-2 rounded-lg border border-line bg-card p-4 shadow-sm">
						<Skeleton class="h-3 w-20 rounded-xs" />
						<Skeleton class="h-6 w-12 rounded-xs" />
					</div>
				{/each}
			</div>
			<div class="flex flex-col gap-3 rounded-lg border border-line bg-card p-4 shadow-sm md:p-6">
				<Skeleton class="h-6 w-40 rounded-xs" />
				<Skeleton class="h-9 w-full rounded-md" />
				<Skeleton class="h-11 w-full rounded-md" />
			</div>
		</div>
	{:else if loadState === 'failed'}
		<Notice variant="warning" heading="Workers unavailable.">
			<p class="m-0">{loadError}</p>
			<p class="mt-3 mb-0"><Button onclick={() => load()}>Try again</Button></p>
		</Notice>
	{:else if status}
		<section aria-label="Status">
			<dl class="m-0 grid grid-cols-2 gap-3 md:grid-cols-4">
				<div class="flex flex-col gap-1 rounded-lg border border-line bg-card p-4 shadow-sm">
					<dt class="text-xs font-medium text-ink-muted">Captures</dt>
					<dd class="m-0 text-h3 font-semibold tabular-nums">{status.captures}</dd>
				</div>
				<div class="flex flex-col gap-1 rounded-lg border border-line bg-card p-4 shadow-sm">
					<dt class="text-xs font-medium text-ink-muted">Sources</dt>
					<dd class="m-0 text-h3 font-semibold tabular-nums">{status.sources}</dd>
				</div>
				<div class="flex flex-col gap-1 rounded-lg border border-line bg-card p-4 shadow-sm">
					<dt class="text-xs font-medium text-ink-muted">Open fetch failures</dt>
					<dd class={`m-0 text-h3 font-semibold tabular-nums ${status.failures > 0 ? 'text-changed' : ''}`}>{status.failures}</dd>
				</div>
				<div class="flex flex-col gap-1 rounded-lg border border-line bg-card p-4 shadow-sm">
					<dt class="text-xs font-medium text-ink-muted">Fetcher</dt>
					<dd class="m-0 text-sm">
						<span class="font-mono">{status.backend}</span> · {status.mode}
						<span class="block text-xs text-ink-muted">{status.timeoutSecs} s timeout · {kib(status.maxBody)} cap</span>
					</dd>
				</div>
				<div class="col-span-2 flex flex-col gap-1 rounded-lg border border-line bg-card p-4 shadow-sm md:col-span-4">
					<dt class="text-xs font-medium text-ink-muted">Last sweep</dt>
					<dd class="m-0 text-sm">
						{#if status.lastSweep}
							<time class="font-mono text-xs" datetime={new Date(status.lastSweep.at * 1000).toISOString()}
								>{stamp(status.lastSweep.at)}</time
							>
							by {status.lastSweep.actor} · {status.lastSweep.summary}
						{:else}
							No sweep has run yet. Start one below.
						{/if}
					</dd>
				</div>
			</dl>
		</section>

		<div class="grid gap-6 lg:grid-cols-5">
			<section
				id="sweep"
				aria-labelledby="sweep-title"
				class="flex min-w-0 scroll-mt-20 flex-col gap-4 rounded-lg border border-line bg-card p-4 shadow-sm md:p-6 lg:col-span-3"
			>
				<div class="flex flex-col gap-1">
					<h2 id="sweep-title" class="m-0 flex items-center gap-2 text-h3 font-semibold">
						<RadarIcon class="size-5 text-accent-strong" aria-hidden="true" /> Watch sweep
					</h2>
					<p class="m-0 text-sm text-ink-muted">
						Pick the office, then paste what you found: up to 10 URLs are fetched right now, in order;
						emails and phone numbers are saved as the office's contacts for first-source calls and mail.
					</p>
				</div>
				<form
					class="flex flex-col gap-4"
					onsubmit={(event) => {
						event.preventDefault();
						void runSweep();
					}}
				>
					<AuthorityPicker id="sweep-authority" label="Issuing office" bind:value={sweepSlug} onLoaded={(rows) => (pickerRows = rows)} />
					<div class="flex flex-col gap-1 rounded-md bg-surface px-3 py-2 text-sm">
						<p class="m-0 flex flex-wrap items-center gap-x-2 gap-y-1">
							<LandmarkIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
							{#if sweepSlug}
								<span class="font-medium">{sweepRow?.name ?? sweepSlug}</span>
								<code class="rounded-xs bg-surface-2 px-1 font-mono text-xs text-ink-muted">{sweepSlug}</code>
							{:else}
								<span class="text-ink-muted">No office chosen yet</span>
							{/if}
						</p>
						<p class="m-0 text-xs text-ink-muted">
							A slug names the office that issues apostilles, not the state or country:
							<code class="font-mono">usa-wa-sos</code> is Washington's Secretary of State, so there is no
							<code class="font-mono">usa-wa</code>. Dossier links and live updates key on the slug, so it
							never changes.
						</p>
					</div>

					{#if sweepSlug}
						<div class="flex flex-col gap-1.5" aria-live="polite">
							<p class="m-0 text-xs font-medium text-ink-muted">On file for this office</p>
							{#if onFileState === 'loading'}
								<div class="flex flex-wrap gap-1.5" role="status" aria-label="Loading contacts on file">
									<Skeleton class="h-7 w-44 rounded-full" />
									<Skeleton class="h-7 w-36 rounded-full" />
								</div>
							{:else if onFileState === 'failed'}
								<p class="m-0 flex flex-wrap items-center gap-2 text-sm text-ink-muted">
									{onFileError}
									<Button variant="outline" size="sm" onclick={() => loadOnFile(sweepSlug)}>Try again</Button>
								</p>
							{:else if onFile.length === 0}
								<p class="m-0 text-sm text-ink-muted">
									No office email or phone yet. Add the ones from its public contact page below.
								</p>
							{:else}
								<ul class="m-0 flex list-none flex-wrap gap-1.5 p-0" aria-label="Contacts on file">
									{#each onFile as c (c.id || c.value)}
										{@const f = formatSaved(c)}
										<li
											class="inline-flex min-h-7 max-w-full items-center gap-1 rounded-full bg-surface-2 px-2 text-sm text-ink-muted"
											title={f.title}
										>
											{#if f.kind === 'phone'}<PhoneIcon class="size-3.5 flex-none" aria-hidden="true" />{:else if f.kind === 'email'}<MailIcon
													class="size-3.5 flex-none"
													aria-hidden="true"
												/>{:else}<LinkIcon class="size-3.5 flex-none" aria-hidden="true" />{/if}
											<span class="min-w-0 wrap-anywhere">{f.display}</span>
										</li>
									{/each}
								</ul>
							{/if}
						</div>
					{/if}

					<TargetField
						bind:this={targetField}
						id="sweep-targets"
						bind:entries={sweepEntries}
						defaultCountry={sweepCountry}
						{savedKeys}
						disabled={sweepBusy}
					/>

					<div class="flex flex-wrap items-center gap-3">
						<Button type="submit" size="lg" disabled={!canSweep}>
							{#if sweepBusy}
								<RefreshCwIcon class="motion-safe:animate-spin" aria-hidden="true" /> Working…
							{:else}
								{sweepLabel}
							{/if}
						</Button>
						{#if !sweepSlug}
							<span class="text-sm text-ink-muted">Choose the issuing office to enable the sweep.</span>
						{/if}
					</div>
				</form>

				{#if sweepNote}
					<Notice variant="warning" heading="Not everything went through.">
						<p class="m-0 text-sm">{sweepNote}</p>
					</Notice>
				{/if}
				{#if sweepResults.length > 0}
					<div class="flex flex-col gap-2">
						{#if sweepSummary}<p class="m-0 text-sm font-medium" role="status">{sweepSummary}</p>{/if}
						<ul class="m-0 flex list-none flex-col divide-y divide-line rounded-md border border-line p-0" aria-label="Sweep results">
							{#each sweepResults as row, index (`${row.key}-${index}`)}
								<li class="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-sm" in:fly={{ y: -4, duration: motion }}>
									{#if row.kind === 'url'}<LinkIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />{:else if row.kind === 'email'}<MailIcon
											class="size-4 flex-none text-ink-muted"
											aria-hidden="true"
										/>{:else}<PhoneIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />{/if}
									<span class="min-w-0 flex-1 font-mono text-xs wrap-anywhere">{row.target}</span>
									<span class={outcomeCls(row.outcome)}>{OUTCOME_BADGE[row.outcome]}</span>
									<span class="w-full text-ink-muted sm:w-auto">{row.text}</span>
								</li>
							{/each}
						</ul>
					</div>
				{/if}
			</section>

			<section
				aria-labelledby="fetch-title"
				class="flex min-w-0 flex-col gap-4 self-start rounded-lg border border-line bg-card p-4 shadow-sm md:p-6 lg:col-span-2"
			>
				<div class="flex flex-col gap-1">
					<h2 id="fetch-title" class="m-0 text-h3 font-semibold">Fetch one URL</h2>
					<p class="m-0 text-sm text-ink-muted">One page, with the source kind and trust you choose.</p>
				</div>
				<form
					class="flex flex-col gap-4"
					onsubmit={(event) => {
						event.preventDefault();
						void runFetch();
					}}
				>
					<AuthorityPicker id="fetch-authority" label="Authority" bind:value={fetchSlug} />
					<div class="flex flex-col gap-1">
						<label for="fetch-url" class="text-xs font-medium text-ink-muted">URL</label>
						<Input
							id="fetch-url"
							type="url"
							class="h-10"
							bind:value={fetchUrl}
							placeholder="https://…"
							autocomplete="off"
							required
						/>
					</div>
					<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
						<div class="flex min-w-0 flex-col gap-1">
							<span id="fetch-kind-label" class="text-xs font-medium text-ink-muted">Source kind</span>
							<Select.Root type="single" bind:value={fetchKind}>
								<Select.Trigger id="fetch-kind" class="h-10 w-full" aria-labelledby="fetch-kind-label fetch-kind">
									{labelFor(SOURCE_KINDS, fetchKind)}
								</Select.Trigger>
								<Select.Content>
									{#each SOURCE_KINDS as k (k.value)}
										<Select.Item value={k.value} label={k.label}>{k.label}</Select.Item>
									{/each}
								</Select.Content>
							</Select.Root>
						</div>
						<div class="flex min-w-0 flex-col gap-1">
							<span id="fetch-trust-label" class="text-xs font-medium text-ink-muted">Trust</span>
							<Select.Root type="single" bind:value={fetchTrust}>
								<Select.Trigger id="fetch-trust" class="h-10 w-full" aria-labelledby="fetch-trust-label fetch-trust">
									{labelFor(TRUSTS, fetchTrust)}
								</Select.Trigger>
								<Select.Content>
									{#each TRUSTS as t (t.value)}
										<Select.Item value={t.value} label={t.label}>{t.label}</Select.Item>
									{/each}
								</Select.Content>
							</Select.Root>
						</div>
					</div>
					<div>
						<Button type="submit" size="lg" disabled={fetchBusy}>{fetchBusy ? 'Fetching…' : 'Fetch now'}</Button>
					</div>
				</form>
				{#if fetchNote}
					<p class="m-0 rounded-md border border-line bg-surface px-3 py-2 text-sm" role="status">{fetchNote}</p>
				{/if}
			</section>
		</div>

		<section aria-labelledby="authorities-title" class="flex flex-col gap-3">
			<h2 id="authorities-title" class="m-0 text-h3 font-semibold">Authorities</h2>
			{#if authorities.length === 0}
				<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line bg-card px-6 py-10 text-center">
					<InboxIcon class="size-8 text-ink-muted" aria-hidden="true" />
					<p class="m-0 text-sm text-ink-muted">No authority has ingest state yet. A sweep creates the first.</p>
					<Button variant="outline" href="#sweep">Start a sweep</Button>
				</div>
			{:else}
				<div class="overflow-x-auto rounded-lg border border-line bg-card shadow-sm">
					<Table.Root aria-label="Authorities">
						<Table.Header class="bg-surface">
							<tr class="border-b border-line">
								<Table.Head class="w-1/2 px-3 text-xs text-ink-muted sm:px-4 lg:w-1/3">Authority</Table.Head>
								<Table.Head class="text-right text-xs text-ink-muted">Sources</Table.Head>
								<Table.Head class="text-right text-xs text-ink-muted">Captures</Table.Head>
								<Table.Head class="hidden text-xs text-ink-muted md:table-cell">Last capture</Table.Head>
								<Table.Head class="text-right text-xs text-ink-muted">Failures</Table.Head>
								<Table.Head class="hidden pr-4 text-xs text-ink-muted lg:table-cell">Last analysis</Table.Head>
							</tr>
						</Table.Header>
						<Table.Body>
							{#each authorities as row (row.slug)}
								<tr class="border-b border-line last:border-b-0">
									<th scope="row" class="max-w-0 px-3 py-2.5 text-left font-normal sm:px-4">
										<span class="block truncate text-sm font-medium" title={row.name}>{row.name}</span>
										<span class="block truncate font-mono text-xs text-ink-muted">{row.slug}</span>
									</th>
									<Table.Cell class="text-right font-mono text-sm tabular-nums">{row.sources}</Table.Cell>
									<Table.Cell class="text-right font-mono text-sm tabular-nums">{row.captures}</Table.Cell>
									<Table.Cell class="hidden font-mono text-xs text-ink-muted md:table-cell">{stamp(row.lastCaptureAt)}</Table.Cell>
									<Table.Cell class={`text-right font-mono text-sm tabular-nums ${row.failures > 0 ? 'text-changed' : ''}`}>{row.failures}</Table.Cell>
									<Table.Cell class="hidden pr-4 font-mono text-xs text-ink-muted lg:table-cell">
										{#if row.lastAnalysisAt}{stamp(row.lastAnalysisAt)} · {row.lastPromptVersion ?? '?'}{:else}—{/if}
									</Table.Cell>
								</tr>
							{/each}
						</Table.Body>
					</Table.Root>
				</div>
			{/if}
		</section>

		<section aria-labelledby="captures-title" class="flex flex-col gap-3">
			<h2 id="captures-title" class="m-0 text-h3 font-semibold">Captures</h2>
			<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-4 lg:items-end">
				<div class="flex min-w-0 flex-col gap-1 sm:col-span-2">
					<AuthorityPicker id="filter-authority" label="Authority" bind:value={filterAuthority} />
				</div>
				<div class="flex min-w-0 flex-col gap-1">
					<span id="filter-kind-label" class="text-xs font-medium text-ink-muted">Kind</span>
					<Select.Root type="single" bind:value={filterKind} onValueChange={() => loadCaptures()}>
						<Select.Trigger id="filter-kind" class="h-9 w-full" aria-labelledby="filter-kind-label filter-kind">
							{filterKind === 'all' ? 'Any kind' : human(filterKind)}
						</Select.Trigger>
						<Select.Content>
							<Select.Item value="all" label="Any kind">Any kind</Select.Item>
							{#each CAPTURE_KINDS as k (k)}
								<Select.Item value={k} label={human(k)}>{human(k)}</Select.Item>
							{/each}
						</Select.Content>
					</Select.Root>
				</div>
				<div class="flex min-w-0 flex-col gap-1">
					<span id="filter-trust-label" class="text-xs font-medium text-ink-muted">Trust</span>
					<Select.Root type="single" bind:value={filterTrust} onValueChange={() => loadCaptures()}>
						<Select.Trigger id="filter-trust" class="h-9 w-full" aria-labelledby="filter-trust-label filter-trust">
							{filterTrust === 'all' ? 'Any trust' : human(filterTrust)}
						</Select.Trigger>
						<Select.Content>
							<Select.Item value="all" label="Any trust">Any trust</Select.Item>
							{#each CAPTURE_TRUSTS as t (t)}
								<Select.Item value={t} label={human(t)}>{human(t)}</Select.Item>
							{/each}
						</Select.Content>
					</Select.Root>
				</div>
			</div>
			{#if filterAuthority}
				<p class="m-0 flex flex-wrap items-center gap-2 text-sm text-ink-muted">
					Showing <code class="font-mono text-xs">{filterAuthority}</code>
					<Button
						variant="ghost"
						size="sm"
						onclick={() => {
							filterAuthority = '';
						}}>All authorities</Button
					>
				</p>
			{/if}

			{#if capturesState === 'loading'}
				<div class="flex flex-col gap-3" role="status" aria-label="Loading captures">
					{#each [0, 1] as i (i)}
						<Skeleton class="h-24 w-full rounded-lg" />
					{/each}
				</div>
			{:else if capturesState === 'failed'}
				<Notice variant="warning" heading="Captures could not load.">
					<p class="m-0">{capturesError}</p>
					<p class="mt-3 mb-0"><Button onclick={loadCaptures}>Try again</Button></p>
				</Notice>
			{:else if captures.length === 0}
				<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line bg-card px-6 py-10 text-center">
					<InboxIcon class="size-8 text-ink-muted" aria-hidden="true" />
					<p class="m-0 text-sm text-ink-muted">No capture matches these filters.</p>
					<Button
						variant="outline"
						onclick={() => {
							filterAuthority = '';
							filterKind = 'all';
							filterTrust = 'all';
						}}>Clear filters</Button
					>
				</div>
			{:else}
				<ul class="m-0 flex list-none flex-col gap-3 p-0">
					{#each captures as item (item.id)}
						<li class="flex flex-col gap-2 rounded-lg border border-line bg-card p-4 shadow-sm">
							<div class="flex flex-wrap items-center gap-x-3 gap-y-1">
								<span class="text-eyebrow font-semibold tracking-wide text-ink-muted uppercase">
									{item.authority} · {human(item.kind)} · {human(item.trust)}
								</span>
								<span class="ml-auto font-mono text-sm">#{item.id}</span>
							</div>
							<p class="m-0 font-mono text-sm wrap-anywhere">
								{item.uri ?? 'Unattributed capture: nothing to re-fetch'}
							</p>
							<p class="m-0 text-sm text-ink-muted">
								{stamp(item.capturedAt)} · {kib(item.bytes)} · sha <span class="font-mono">{item.sha.slice(0, 12)}</span>
							</p>
							{#if item.note}
								<p class="m-0 rounded-md border border-line bg-surface px-3 py-2 text-sm" role="status">{item.note}</p>
							{/if}
							<div class="flex flex-wrap gap-2">
								<Button variant="outline" onclick={() => retry(item)} disabled={item.busy || !item.uri}>
									{item.busy ? 'Fetching…' : 'Retry'}
								</Button>
								{#if item.sourceId !== null}
									<Button
										variant="outline"
										href={`${resolve('/(admin)/sources/[id]', { id: String(item.sourceId) })}?capture=${item.id}`}
									>
										History &amp; diff
									</Button>
								{/if}
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</section>
	{/if}
</div>
