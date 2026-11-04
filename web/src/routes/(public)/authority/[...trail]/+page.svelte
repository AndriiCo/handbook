<script module lang="ts">
	// The guide's code (the presenter and every guide component) is a chunk
	// of its own, requested as this route's module loads: it downloads
	// alongside the first API reads rather than ahead of them. It loads
	// on every visit, so it counts toward the page budget
	// (doc/guide-code.js; web/README.md, JS budgets).
	const guideCode = import('../../../../lib/public-ui/doc/guide-code.js');
	guideCode.catch(() => {});
</script>

<script lang="ts">
	import { tick } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../../lib/components/ui/skeleton/index.js';
	import {
		evidenceRows,
		factRows,
		formRows,
		hasSendSlot,
		indexRow,
		officeRecords,
		planAuthority
	} from '../../../../lib/public-ui/doc/load.js';
	import { modelLabel, parsePlan, type Office } from '../../../../lib/public-ui/plan.js';
	import type { Page as GuidePage } from '../../../../lib/public-ui/doc/contract.js';
	import { subscribe } from '../../../../lib/live.svelte.js';
	import { listAuthorities, publicError, readDossier } from '../../../../lib/public-ui/handbook.js';
	import { PAGE_COPY, indexTranslations, recordsLang } from '../../../../lib/public-ui/translate.js';
	import { site } from '../../../../lib/locale.svelte.js';

	// The address is a readable path (`usa/washington`, `us/wa`, a county
	// `usa/new_york/chemung`) or the internal slug. A one-segment address
	// IS the slug, so its fetches start at once; a path resolves first.
	// The slug alone keys every fetch and the WS topic.
	const trail = $derived(page.params.trail ?? '');
	let resolvedSlugs = $state<Record<string, string>>({});
	let unknownPath = $state(false);
	const slug = $derived(trail.includes('/') ? (resolvedSlugs[trail] ?? '') : trail);
	// Canonical address and breadcrumbs, from the resolver or the
	// authority row, each tagged with the slug it belongs to (a
	// client-side jump to another office must never reuse the previous
	// one's).
	let canonical = $state<{ slug: string; path: string } | null>(null);
	type Crumb = { path: string; label: string; slug: string | null; location?: string | null };
	let crumbsOf = $state<{ slug: string; list: Crumb[] } | null>(null);
	const crumbs = $derived(crumbsOf && crumbsOf.slug === slug ? crumbsOf.list : []);

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	// The guide draws once its code and its reads are both in.
	let code = $state.raw<Awaited<typeof guideCode> | null>(null);
	let codeFailed = $state(false);
	guideCode.then(
		(m) => (code = m),
		() => (codeFailed = true)
	);

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	let facts = $state<any[] | null>(null);
	let loadError = $state<string | null>(null);
	let notFound = $state(false);
	// Machine-translation view (SA-20): in Spanish or French (the site
	// language, or an older `?lang=es|fr` link) stored guard-passing rows
	// show next to the records; anything missing stays in English. Never
	// a model call from this page.
	const target = $derived(recordsLang(page.url.search, site.locale));
	const copy = $derived(PAGE_COPY[site.locale]);
	let translatedById = $state<Record<number, { statement: string; model: string }>>({});

	// `GET /authorities/{slug}/plan` always answers 200: the authority
	// row (R1), accepted office records, forms, published staff notes
	// and the accepted plan's provenance. The page waits for it, and for
	// the index fallback when it names no authority, before it draws, so
	// the header never flashes the slug.
	type AuthorityRow = {
		slug: string;
		name: string;
		region: string;
		country: string;
		kind: string;
		path: string;
		official_url: string | null;
		iso2?: string;
	};
	let planBody = $state<any>(null);
	let planSettled = $state(false);
	let authority = $state<AuthorityRow | null>(null);
	// `/p2/orders/options` (always 200): read after first paint, only
	// when the page has an address to send to; failures are ignored.
	let options = $state<any>(null);
	let optionsFor = '';
	let nowTick = $state(Date.now());
	let flash = $state<string[]>([]);
	let announce = $state('');

	async function loadPlan(current: string) {
		let body: any = null;
		try {
			const res = await get(`/authorities/${encodeURIComponent(current)}/plan`);
			if (current !== slug) return;
			body = res.ok ? res.body : null;
		} catch {
			if (current !== slug) return;
		}
		let row: AuthorityRow | null = planAuthority(body);
		if (!row) {
			// The 118 KB index only when /plan names no authority (J1
			// fixtures, a store before R1).
			try {
				const res = await listAuthorities({ get });
				if (current !== slug) return;
				row = res.ok ? indexRow(res.body, current) : null;
			} catch {
				row = null;
			}
		}
		if (current !== slug) return;
		planBody = body;
		if (row) authority = row;
		if (row?.path && !(canonical && canonical.slug === current)) {
			canonical = { slug: current, path: row.path };
		}
		planSettled = true;
	}

	// A place (`/authority/usa`) is not an office: show its place page.
	async function placeOr404(address: string) {
		try {
			const res = await get(`/authorities/resolve?path=${encodeURIComponent(address)}`);
			const place = res.ok ? res.body?.location?.path : null;
			if (typeof place === 'string' && place) {
				// A full load: the place page is another route with its
				// own crawler head, and this page never finished loading.
				const href = resolve('/(public)/in/[...path]', { path: place });
				window.location.replace(`${href}${page.url.search}`);
				return;
			}
		} catch {
			// Falls through to the unknown-address notice.
		}
		if (address === trail) notFound = true;
	}

	async function reload(current: string) {
		const res = await readDossier(current, { get });
		if (current !== slug) return; // stale slug, ignore
		if (!res.ok) {
			if (res.status === 404) await placeOr404(current);
			else loadError = publicError(res.status, 'dossier');
			return;
		}
		facts = factRows(res.body);
	}

	const isCrumb = (c: any): c is Crumb =>
		typeof c?.path === 'string' && typeof c?.label === 'string' && c.label !== '';

	// A readable path resolves to its slug once (public, model-free read).
	$effect(() => {
		const t = trail;
		unknownPath = t === '';
		if (!t.includes('/') || resolvedSlugs[t]) return;
		let cancelled = false;
		get(`/authorities/resolve?path=${encodeURIComponent(t)}`)
			.then((res) => {
				if (cancelled || t !== trail) return;
				if (res.ok && typeof res.body?.location?.path === 'string') {
					void placeOr404(t);
					return;
				}
				if (!res.ok || typeof res.body?.slug !== 'string') {
					unknownPath = true;
					return;
				}
				if (typeof res.body.canonical === 'string') {
					canonical = { slug: res.body.slug, path: res.body.canonical };
				}
				if (Array.isArray(res.body.crumbs)) {
					crumbsOf = {
						slug: res.body.slug,
						list: res.body.crumbs.slice(0, -1).filter(isCrumb)
					};
				}
				resolvedSlugs = { ...resolvedSlugs, [t]: res.body.slug };
			})
			.catch(() => {
				if (!cancelled && t === trail) unknownPath = true;
			});
		return () => {
			cancelled = true;
		};
	});

	// A slug address learns its breadcrumbs from its canonical path's
	// resolve, once (a one-segment path has none).
	$effect(() => {
		const current = slug;
		const canon = canonical && canonical.slug === current ? canonical.path : null;
		if (!current || !canon || !canon.includes('/') || crumbsOf?.slug === current) return;
		let cancelled = false;
		get(`/authorities/resolve?path=${encodeURIComponent(canon)}`)
			.then((res) => {
				if (cancelled || current !== slug || !res.ok || !Array.isArray(res.body?.crumbs)) return;
				crumbsOf = { slug: current, list: res.body.crumbs.slice(0, -1).filter(isCrumb) };
			})
			.catch(() => {});
		return () => {
			cancelled = true;
		};
	});

	// Show the canonical address in place (no navigation, no refetch):
	// /authority/usa-wa-sos and /authority/us/wa read /authority/usa/washington.
	let swappedFor = '';
	$effect(() => {
		const current = slug;
		const canon = canonical && canonical.slug === current ? canonical.path : null;
		if (!canon || !current || canon === trail || swappedFor === current) return;
		swappedFor = current;
		const href = resolve('/(public)/authority/[...trail]', { trail: canon });
		void goto(`${href}${page.url.search}${page.url.hash}`, { shallow: true, replace: true });
	});

	// Live updates over WS `authority:{slug}` (ids only): re-read, re-present,
	// light the changed rows and say what changed. No polling.
	const LIVE_KINDS = new Set(['fact_accepted', 'plan_accepted', 'offices_accepted', 'fact_retired', 'evidence_updated']);

	async function refresh(current: string) {
		const before = view;
		await Promise.all([reload(current).catch(() => {}), loadPlan(current)]);
		if (current !== slug) return;
		nowTick = Date.now();
		const after = view;
		if (!after || !code) return;
		const changed = code.changedItems(before, after);
		flash = changed.ids;
		announce = '';
		if (changed.labels.length > 0) {
			await tick();
			announce = `${copy.updated}: ${changed.labels.join(', ')}`;
		}
	}

	$effect(() => {
		const current = slug;
		facts = null;
		loadError = null;
		notFound = false;
		authority = null;
		planBody = null;
		planSettled = false;
		options = null;
		optionsFor = '';
		flash = [];
		announce = '';
		if (!current) return;
		reload(current).catch(() => {
			if (current !== slug) return;
			loadError = publicError(500, 'dossier');
		});
		void loadPlan(current);
		return subscribe(`authority:${current}`, (msg) => {
			if (current !== slug || !LIVE_KINDS.has(String(msg?.type ?? ''))) return;
			void refresh(current);
		});
	});

	// Office status and closures move with the clock.
	$effect(() => {
		const timer = setInterval(() => {
			nowTick = Date.now();
		}, 60_000);
		return () => clearInterval(timer);
	});

	// Machine translations follow the same reload cycle: refetch
	// whenever the slug or the records language changes; failures fall back to
	// English silently (the endpoint 404s unknown slugs the same way).
	$effect(() => {
		const current = slug;
		const want = target;
		if (!want || !current) {
			translatedById = {};
			return;
		}
		let cancelled = false;
		get(`/authorities/${encodeURIComponent(current)}/translations?lang=${want}`)
			.then((res) => {
				if (cancelled || current !== slug) return;
				translatedById = res.ok ? indexTranslations(res.body?.translations) : {};
			})
			.catch(() => {
				if (!cancelled) translatedById = {};
			});
		return () => {
			cancelled = true;
		};
	});

	const crumbLinks = $derived(
		crumbs.map((c) => ({
			label: c.label,
			href: c.slug
				? resolve('/(public)/authority/[...trail]', { trail: c.path })
				: c.location
					? resolve('/(public)/in/[...path]', { path: c.location })
					: null
		}))
	);
	const officeSet = $derived(officeRecords(planBody) as Office[]);
	const forms = $derived(formRows(planBody));
	const evidence = $derived(evidenceRows(planBody));
	const planView = $derived(planBody ? parsePlan(planBody) : null);
	const models = $derived([
		...new Set(
			(facts ?? [])
				.map((f) => String(f?.drafted_by ?? f?.created_by ?? '').split(' ')[0])
				.filter((m) => m && m !== 'unknown')
				.map(modelLabel)
		)
	]);

	// The guide: every value from the accepted facts (present.js).
	const view = $derived.by((): GuidePage | null => {
		if (facts === null || !planSettled || !code) return null;
		return code.present({
			authority: { ...(authority ?? { slug }), crumbs: crumbLinks },
			facts,
			plan: planBody?.plan ?? null,
			offices: officeSet,
			forms,
			evidence,
			now: nowTick
		});
	});
	const nowDate = $derived(new Date(nowTick));

	$effect(() => {
		const current = slug;
		const v = view;
		if (!v || !current || optionsFor === current || !hasSendSlot(v)) return;
		optionsFor = current;
		// After first paint: the offers never hold up the page.
		requestAnimationFrame(() => {
			setTimeout(() => {
				get(`/p2/orders/options?slug=${encodeURIComponent(current)}`)
					.then((res) => {
						if (current === slug && res.ok && Array.isArray(res.body?.options)) options = res.body;
					})
					.catch(() => {});
			}, 0);
		});
	});

	// Every header value is runtime data: the authority row and the
	// accepted facts; the slug only until the row arrives.
	const displayName = $derived(authority?.name || slug);
</script>

<svelte:head>
	<title>{displayName} — Apostille Handbook</title>
</svelte:head>

{#if notFound || unknownPath}
	<Notice variant="warning" heading={copy.unknownHeading(slug || trail)}>
		<p>
			{copy.unknownBody}
			<a href={resolve('/(public)')}>{copy.backToIndex}</a>.
		</p>
	</Notice>
{:else if loadError || codeFailed}
	<Notice variant="error" heading={copy.unavailableHeading}>
		<p>{loadError ?? copy.unavailableBody}</p>
	</Notice>
{:else if !view || !code}
	<div role="status" aria-label={copy.loading} class="flex flex-col gap-2 pt-6">
		<span class="sr-only">{copy.loading}</span>
		<Skeleton class="h-9 w-2/3" />
		<Skeleton class="h-4 w-1/3" />
		<Skeleton class="h-24 w-full" />
	</div>
{:else}
	{#if copy.pending}
		<div class="mb-6 print:hidden">
			<Notice variant="info" heading={copy.pending.heading}>
				<p>{copy.pending.body}</p>
			</Notice>
		</div>
	{/if}
	<!-- The guide's own words are English until its copy is translated. -->
	<div lang="en">
	<code.DossierGuide
		page={view}
		name={displayName}
		{slug}
		apiBase={apiRoot}
		region={authority?.region || null}
		country={authority?.country || null}
		{officeSet}
		{options}
		{evidence}
		{forms}
		provenance={planView?.provenance ?? null}
		planVersion={planView?.plan.version ?? 0}
		planAcceptedAt={planView?.plan.accepted_at ?? null}
		{models}
		lang={target}
		translations={translatedById}
		pathname={page.url.pathname}
		now={nowDate}
		{flash}
		{announce}
	/>
	</div>
{/if}
