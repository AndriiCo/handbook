<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { fly } from 'svelte/transition';
	import { toast } from 'svelte-sonner';
	import { resolve } from '$app/paths';
	import Heading from '#lib/components/brand/Heading.svelte';
	import Notice from '#lib/public-ui/Notice.svelte';
	import * as Tabs from '#lib/components/ui/tabs/index.js';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Kbd } from '#lib/components/ui/kbd/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import PlanReview from '#lib/admin/PlanReview.svelte';
	import OfficeReview from '#lib/admin/OfficeReview.svelte';
	import FormMapReview from '#lib/admin/FormMapReview.svelte';
	import AuthorityGroups from '#lib/admin/AuthorityGroups.svelte';
	import {
		acceptBody,
		acceptFact,
		bulkBody,
		bulkDecide,
		createRule,
		fetchQueue,
		fetchQueueItem,
		fetchRules,
		fetchSuppressions,
		rejectBody,
		rejectFact,
		revokeRule,
		ruleBody
	} from '#lib/admin/review-client.js';
	import { isVerified, markChanged, plural, riskCls, riskiestFirst, riskOf, splitBlocked, WONDER } from '#lib/admin/review-signals.js';
	import { LOCKED_TOPIC, canMoveTopic, chipStatement, editAcceptBody, keyNote, statementKey, topicChoices } from '#lib/admin/review-keys.js';
	import { canonicalKeys, splitKey } from '#lib/public-ui/slots.js';
	import { getJson, postJson, errorText } from '#lib/admin/admin-api.js';
	import { fetchFactChecks, type FactCheck } from '#lib/admin/proposals.js';
	import { ago, modelName } from '#lib/admin/format.js';
	import { subscribe } from '#lib/live.svelte.js';
	import CheckIcon from '@lucide/svelte/icons/check';
	import CheckCheckIcon from '@lucide/svelte/icons/check-check';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import ListOrderedIcon from '@lucide/svelte/icons/list-ordered';
	import LandmarkIcon from '@lucide/svelte/icons/landmark';

	interface QueueItem {
		id: number;
		authority_slug: string;
		topic: string;
		statement: string;
		status: string;
		created_by: string;
		created_at: number;
		diff: { base_statement: string; base_fact_id: number; base_status: string } | null;
		citations: { capture_id: number; start_byte: number; end_byte: number; quote: string }[];
		reasoning_untrusted: string | null;
		blast_radius: {
			conflict: boolean;
			conflicts: { id: number; statement: string; status: string }[];
			open_tasks: number;
		};
		provenance: {
			capture_id: number;
			capture_kind: string;
			trust: string;
			authenticity: string;
			sidecar: string;
		};
	}

	interface SuppressRule {
		id: number;
		authority_id: number | null;
		authority_slug: string | null;
		topic: string | null;
		statement: string;
		reason: string;
		created_by: string;
		created_at: number;
		revoked_at: number | null;
	}

	interface Suppression {
		id: number;
		capture_id: number;
		authority_slug: string;
		topic: string;
		statement: string;
		path: string;
		rule_id: number | null;
		created_at: number;
	}

	interface CardUi {
		quoteOpen: boolean;
		editing: boolean;
		editText: string;
		// Accept-with-edit topic: the draft's own until the select moves it.
		topic: string;
		rejectOpen: boolean;
		reason: string;
		suppressOn: boolean;
		scope: string;
		lockTopic: boolean;
		acting: boolean;
		error: string | null;
	}

	// Review console (analysis-agent.md §11.4 + MISSION WIN W1b): a data
	// table of proposed facts with one detail pane, j/k/a/r keys, and
	// tabs for page plans, offices, form maps and translations. Every
	// decision is the server's: optimistic here, rolled back on error.
	const api = () => `/apostille/api/v1`;

	type Tab = 'facts' | 'plans' | 'offices' | 'forms' | 'translations';
	const TABS: Tab[] = ['facts', 'plans', 'offices', 'forms', 'translations'];

	// Compose deep links, read before the tabs mount: `compose=<slug>`
	// preselects the authority (never starts a run); `run=<id>` reopens
	// that compose run in the tab named by `tab`.
	const boot = typeof location === 'undefined' ? new URLSearchParams() : new URLSearchParams(location.search);
	const bootTab = boot.get('tab');
	const composeSlug = /^[a-z0-9-]{1,80}$/.test(boot.get('compose') ?? '') ? (boot.get('compose') ?? '') : '';
	const bootRunRaw = Number(boot.get('run'));
	const bootRun = Number.isInteger(bootRunRaw) && bootRunRaw > 0 ? bootRunRaw : null;
	let tab = $state<Tab>('facts');
	let counts = $state<Record<'plans' | 'offices' | 'forms', number | null>>({
		plans: null,
		offices: null,
		forms: null
	});

	let authority = $state('');
	let topic = $state('');
	let limit = $state(50);
	let items = $state<QueueItem[]>([]);
	let queueCount = $state(0);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let loadingMore = $state(false);
	let decided = $state<Record<number, string>>({});
	let refreshNote = $state<string | null>(null);
	let ui = $state<Record<number, CardUi>>({});
	let selectedId = $state<number | null>(null);
	let checks = $state<Record<number, FactCheck>>({});
	let batchActing = $state(false);
	let batchNote = $state<{ text: string; pass: number[] } | null>(null);
	let reduceMotion = $state(false);
	// Facts tab layout: the risk-sorted table + detail pane (default,
	// the demo's take), or proposals grouped per authority with bulk
	// decisions (`?view=authority`). Mounted with {#if}, never both.
	type View = 'risk' | 'authority';
	let view = $state<View>('risk');
	let focusFact = $state<number | null>(null);

	let rules = $state<SuppressRule[]>([]);
	let rulesState = $state<'loading' | 'ready' | 'failed'>('loading');
	let rulesError = $state<string | null>(null);
	let showRevoked = $state(false);
	let ruleStatement = $state('');
	let ruleAuthority = $state('');
	let ruleTopic = $state('');
	let ruleReason = $state('');
	let ruleError = $state<string | null>(null);
	let ruleSaving = $state(false);

	let suppressions = $state<Suppression[]>([]);
	let skipsState = $state<'loading' | 'ready' | 'failed'>('loading');
	let skipsError = $state<string | null>(null);

	// Translations tab (SA-20): Muse Spark renders accepted facts into
	// es/fr after approval. The guard (digits/fees/addresses) holds
	// failures back as `guard_ok = 0` — staff-visible, never public.
	interface AcceptedFact {
		id: number;
		topic: string;
		statement: string;
		status: string;
	}
	interface TranslationRow {
		fact_id: number;
		lang: string;
		statement: string;
		model: string;
		guard_ok: boolean;
		created_at: number;
	}
	let acceptedFacts = $state<AcceptedFact[]>([]);
	let acceptedState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let acceptedError = $state<string | null>(null);
	let trRows = $state<TranslationRow[]>([]);
	let trState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let trError = $state<string | null>(null);
	let trNote = $state<string | null>(null);
	let trActing = $state<Record<string, boolean>>({});

	function trKey(factId: number, lang: string): string {
		return `${factId}:${lang}`;
	}

	function trFor(factId: number, lang: string): TranslationRow | null {
		return trRows.find((r) => r.fact_id === factId && r.lang === lang) ?? null;
	}

	function trLabel(row: TranslationRow | null): string {
		if (!row) return 'Missing';
		if (!row.statement) return 'Pending';
		return row.guard_ok ? 'Done' : 'Held by guard';
	}

	async function loadAccepted() {
		acceptedState = 'loading';
		acceptedError = null;
		try {
			const body = await getJson<{ facts?: AcceptedFact[] }>('/facts?status=accepted&limit=100', 'Accepted facts');
			acceptedFacts = Array.isArray(body?.facts) ? body.facts : [];
			acceptedState = 'ready';
		} catch (err) {
			acceptedError = errorText(err, 'Accepted facts failed.');
			acceptedState = 'failed';
		}
	}

	async function loadTranslations() {
		trState = 'loading';
		trError = null;
		try {
			const body = await getJson<{ translations?: TranslationRow[] }>('/translations/status', 'Translations');
			trRows = Array.isArray(body?.translations) ? body.translations : [];
			trState = 'ready';
		} catch (err) {
			trError = errorText(err, 'Translations failed.');
			trState = 'failed';
		}
	}

	async function translateOne(factId: number, lang: string) {
		const key = trKey(factId, lang);
		if (trActing[key]) return;
		trActing[key] = true;
		trNote = null;
		try {
			const body = await postJson<{ guard_ok?: boolean }>(`/facts/${factId}/translate`, { lang }, `Translate #${factId}`);
			trNote =
				body?.guard_ok === false
					? `Fact #${factId} (${lang}): held by the number/fee/address guard — English stays live.`
					: `Fact #${factId} (${lang}): translated.`;
			await loadTranslations();
		} catch (err) {
			trNote = errorText(err, `Translate #${factId} failed.`);
		} finally {
			trActing[key] = false;
		}
	}

	async function backfill() {
		trNote = null;
		try {
			const body = await postJson<{ queued?: number; rendered?: number; guard_failed?: number }>(
				'/translations/backfill',
				{ limit: 20 },
				'Backfill'
			);
			trNote = `Backfill: ${body?.queued ?? 0} queued, ${body?.rendered ?? 0} rendered, ${body?.guard_failed ?? 0} held by guard.`;
			await loadTranslations();
		} catch (err) {
			trNote = errorText(err, 'Backfill failed.');
		}
	}

	function cardUi(id: number): CardUi {
		let card = ui[id];
		if (!card) {
			card = {
				quoteOpen: false,
				editing: false,
				editText: '',
				topic: '',
				rejectOpen: false,
				reason: '',
				suppressOn: false,
				scope: 'authority',
				lockTopic: true,
				acting: false,
				error: null
			};
			ui[id] = card;
		}
		return card;
	}

	const nowSec = () => Math.max(Date.now() / 1000, ...items.map((i) => i.created_at));

	let sorted = $derived(riskiestFirst(items));
	const selected = $derived(sorted.find((i) => i.id === selectedId) ?? null);
	const selectedCard = $derived(selected ? (ui[selected.id] ?? null) : null);
	const open = $derived(sorted.filter((i) => !decided[i.id]));
	const batch = $derived(
		selected
			? open.filter(
					(i) =>
						i.authority_slug === selected.authority_slug &&
						i.provenance.capture_id === selected.provenance.capture_id &&
						isVerified(i, checks[i.id])
				)
			: []
	);

	interface BlockedItem {
		id: number;
		at: number;
		authority_slug: string;
		attempt: number;
		kind: string;
		topic: string;
		field: string;
		value: string;
		span_start: number;
		span_end: number;
	}

	let blocked = $state<BlockedItem[]>([]);
	let blockedCount = $state(0);
	let blockedState = $state<'loading' | 'ready' | 'failed'>('loading');
	let blockedError = $state<string | null>(null);

	// Records the verifier refused (the staff `/draft-runs/recent`
	// endpoint reads the `analyze.rejected` audit rows). Failed reads
	// keep the queue usable — this section alone reports the error.
	async function loadBlocked() {
		blockedState = 'loading';
		blockedError = null;
		try {
			const body = await getJson<{ rejections?: BlockedItem[]; count?: number }>(
				'/draft-runs/recent?limit=20',
				'Blocked list'
			);
			blocked = Array.isArray(body?.rejections) ? body.rejections : [];
			blockedCount = typeof body?.count === 'number' ? body.count : blocked.length;
			blockedState = 'ready';
		} catch (err) {
			blockedError = errorText(err, 'Blocked list failed.');
			blockedState = 'failed';
		}
	}

	function syncUrl() {
		const params = new URLSearchParams(location.search);
		for (const [key, value] of [
			['authority', authority.trim()],
			['topic', topic.trim()],
			['limit', limit !== 50 ? String(limit) : ''],
			['tab', tab !== 'facts' ? tab : ''],
			['view', view === 'authority' ? 'authority' : '']
		] as const) {
			if (value) params.set(key, value);
			else params.delete(key);
		}
		const query = params.toString();
		history.replaceState(history.state, '', query ? `?${query}` : location.pathname);
	}

	function ensureSelection() {
		if (selectedId !== null && sorted.some((i) => i.id === selectedId)) return;
		selectedId = open[0]?.id ?? null;
	}

	async function loadChecks() {
		const slugs = [...new Set(items.map((i) => i.authority_slug))];
		const found = await Promise.allSettled(slugs.map((slug) => fetchFactChecks(slug)));
		const next: Record<number, FactCheck> = {};
		for (const r of found) {
			if (r.status !== 'fulfilled') continue;
			for (const c of r.value) next[c.fact_id] = c;
		}
		checks = next;
	}

	async function loadQueue(afterId?: number, append = false, quiet = false) {
		if (append) {
			loadingMore = true;
		} else if (!quiet) {
			loadState = 'loading';
			loadError = null;
		}
		try {
			const out = await fetchQueue(api(), {
				authority: authority.trim() || undefined,
				topic: topic.trim() || undefined,
				limit,
				after_id: afterId
			});
			// Card UI state is created here (event/task context), never
			// during render: assigning ui[id] inside the template
			// throws state_unsafe_mutation on the first non-empty queue.
			for (const row of out.items) cardUi(row.id);
			items = append ? [...items, ...out.items] : out.items;
			queueCount = out.count;
			loadState = 'ready';
			ensureSelection();
			void loadChecks();
			void openDeepLink();
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Review queue failed.';
			if (quiet) toast.error(loadError);
			else loadState = append ? loadState : 'failed';
		} finally {
			loadingMore = false;
		}
	}

	// `?fact=<id>` (dashboard activity links): select that row, reading
	// it on its own when it sits beyond the loaded page.
	let deepLinked = false;
	async function openDeepLink() {
		if (deepLinked || focusFact === null) return;
		deepLinked = true;
		const id = focusFact;
		if (!items.some((i) => i.id === id)) {
			try {
				const item = await fetchQueueItem(api(), id);
				cardUi(item.id);
				items = [item, ...items];
				queueCount += 1;
			} catch {
				refreshNote = `Fact #${id} is not waiting for review any more.`;
				focusFact = null;
				return;
			}
		}
		select(id);
	}

	// By-authority view: every proposal of one authority (the queue
	// pages 25-200 rows across all of them), merged by id.
	async function loadAllFor(slug: string) {
		const out = await fetchQueue(api(), { authority: slug, limit: 200 });
		for (const row of out.items) cardUi(row.id);
		const known = new Set(items.map((i) => i.id));
		items = [...items, ...out.items.filter((i) => !known.has(i.id))];
		void loadChecks();
	}

	function setView(next: View) {
		if (view === next) return;
		view = next;
		syncUrl();
		if (next === 'risk') ensureSelection();
	}

	function openInQueue(slug: string) {
		authority = slug;
		view = 'risk';
		selectedId = null;
		syncUrl();
		void loadQueue();
	}

	function applyFilters(event: SubmitEvent) {
		event.preventDefault();
		syncUrl();
		selectedId = null;
		void loadQueue();
	}

	async function loadRules() {
		rulesState = 'loading';
		rulesError = null;
		try {
			const out = await fetchRules(api(), { include_revoked: showRevoked || undefined });
			rules = out.rules;
			rulesState = 'ready';
		} catch (err) {
			rulesError = err instanceof Error ? err.message : 'Suppress rules failed.';
			rulesState = 'failed';
		}
	}

	async function loadSuppressions() {
		skipsState = 'loading';
		skipsError = null;
		try {
			const out = await fetchSuppressions(api(), { limit: 50 });
			suppressions = out.suppressions;
			skipsState = 'ready';
		} catch (err) {
			skipsError = err instanceof Error ? err.message : 'Suppression log failed.';
			skipsState = 'failed';
		}
	}

	// Staff `proposals` feed: a draft run or voice quote that inserts
	// proposals publishes `proposal.created` per fact — insert the row
	// live (deduped by id) instead of waiting for the next refresh.
	async function onProposal(msg: { type?: unknown; fact_id?: unknown }) {
		if (msg?.type !== 'proposal.created') return;
		const id = msg?.fact_id;
		if (typeof id !== 'number') return;
		if (decided[id] || items.some((item) => item.id === id)) return;
		try {
			const item = await fetchQueueItem(api(), id);
			cardUi(item.id);
			items = [item, ...items];
			queueCount += 1;
			refreshNote = `New proposal #${item.id} arrived live.`;
			ensureSelection();
		} catch {
			// The next queue refresh picks it up; never break the feed.
		}
	}

	function setTab(value: string) {
		if (!TABS.includes(value as Tab)) return;
		tab = value as Tab;
		syncUrl();
		if (tab === 'translations') {
			if (acceptedState === 'idle') void loadAccepted();
			void loadTranslations();
		}
	}

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		const params = new URLSearchParams(location.search);
		authority = params.get('authority') ?? '';
		topic = params.get('topic') ?? '';
		const raw = Number(params.get('limit'));
		if ([25, 50, 100, 200].includes(raw)) limit = raw;
		const wanted = params.get('tab');
		if (wanted && TABS.includes(wanted as Tab) && wanted !== 'facts') setTab(wanted);
		if (params.get('view') === 'authority') view = 'authority';
		const fact = Number(params.get('fact'));
		if (Number.isInteger(fact) && fact > 0) focusFact = fact;
		void loadQueue();
		void loadRules();
		void loadSuppressions();
		void loadBlocked();
		const unsub = subscribe('proposals', (msg) => {
			void onProposal(msg);
		});
		return () => unsub();
	});

	// Next undecided row after `id` in table order (wraps to the top).
	function nextOpen(id: number): number | null {
		const at = sorted.findIndex((i) => i.id === id);
		const after = sorted.slice(at + 1).find((i) => !decided[i.id] && i.id !== id);
		return after?.id ?? sorted.find((i) => !decided[i.id] && i.id !== id)?.id ?? null;
	}

	function select(id: number | null) {
		selectedId = id;
		if (id === null) return;
		void tick().then(() => {
			document.getElementById(`fact-row-${id}`)?.scrollIntoView({ block: 'nearest' });
		});
	}

	function handleDecisionError(id: number, err: unknown, fallback: string) {
		const message = err instanceof Error ? err.message : fallback;
		if (/409|not proposed/.test(message)) {
			refreshNote = `Fact #${id}: already decided — refreshing.`;
			void loadQueue();
		} else {
			cardUi(id).error = message;
			toast.error(`Fact #${id}: ${message}`);
		}
	}

	async function accept(id: number, quiet = false): Promise<boolean> {
		const card = cardUi(id);
		if (decided[id] || card.acting) return false;
		const item = items.find((i) => i.id === id);
		card.acting = true;
		card.error = null;
		// Optimistic: the row reads decided the moment staff clicks and
		// the pane moves on; a failed POST rolls both back below.
		decided[id] = 'accepted';
		if (selectedId === id) select(nextOpen(id));
		try {
			const out = await acceptFact(api(), id, acceptBody(undefined, undefined) ?? {});
			decided[id] = out.status;
			if (!quiet && item) toast.success(`Accepted — the ${item.authority_slug} dossier updates live.`);
			return true;
		} catch (err) {
			delete decided[id];
			select(id);
			handleDecisionError(id, err, 'Accept failed.');
			return false;
		} finally {
			card.acting = false;
		}
	}

	// Per-capture batch: ONE all-or-nothing POST /facts/bulk (one
	// audit header, one dossier event). A blocked batch changed
	// nothing: every optimistic row rolls back, the refused rows say
	// why, and the rest can go again as a fresh batch.
	async function acceptBatch(only?: number[]) {
		if (batchActing) return;
		const rows = only ? items.filter((i) => only.includes(i.id) && !decided[i.id]) : batch;
		if (rows.length === 0) return;
		const slug = rows[0].authority_slug;
		const capture = rows[0].provenance.capture_id;
		const ids = rows.map((i) => i.id).slice(0, 100);
		const body = bulkBody('accept', slug, ids, { requireVerified: true });
		if (!body) return;
		batchActing = true;
		batchNote = null;
		for (const id of ids) {
			cardUi(id).error = null;
			decided[id] = 'accepted';
		}
		if (selectedId !== null && ids.includes(selectedId)) select(nextOpen(selectedId));
		try {
			const out = await bulkDecide(api(), body);
			if (out.ok) {
				for (const r of out.results) decided[r.id] = r.status ?? 'accepted';
				toast.success(`Accepted ${plural(out.count, 'verified fact')} from capture #${capture} — the ${slug} dossier updates live.`);
				return;
			}
			for (const id of ids) delete decided[id];
			const { refused, passing } = splitBlocked(out.results);
			for (const r of refused) cardUi(r.id).error = `Held back from the batch: ${r.tag}.`;
			const pass = passing.filter((id) => ids.includes(id));
			const text = `Nothing accepted: ${refused.length} of ${ids.length} blocked.`;
			batchNote = { text, pass };
			toast.error(
				text,
				pass.length > 0 ? { action: { label: `Accept the ${pass.length} that pass`, onClick: () => void acceptBatch(pass) } } : undefined
			);
			select(refused[0]?.id ?? ids[0]);
			if (refused.some((r) => r.code === 409)) {
				refreshNote = 'Some of the batch was decided elsewhere — refreshing.';
				void loadQueue(undefined, false, true);
			}
		} catch (err) {
			for (const id of ids) delete decided[id];
			const message = err instanceof Error ? err.message : 'Batch accept failed.';
			batchNote = { text: `Nothing accepted: ${message}`, pass: [] };
			toast.error(`Nothing accepted: ${message}`);
			select(ids[0]);
		} finally {
			batchActing = false;
		}
	}

	function startEdit(item: QueueItem) {
		const card = cardUi(item.id);
		card.editing = true;
		card.rejectOpen = false;
		card.editText = item.statement;
		card.topic = item.topic;
		card.error = null;
	}

	async function acceptEdit(item: QueueItem) {
		const card = cardUi(item.id);
		const body = editAcceptBody(item, card.editText, card.topic);
		if (!body) {
			card.error = 'Edited statement must be 1–2000 chars.';
			return;
		}
		card.acting = true;
		card.error = null;
		try {
			const out = await acceptFact(api(), item.id, body);
			decided[item.id] = out.status;
			select(nextOpen(item.id));
			const moved = body.topic ? ` as ${body.topic}` : '';
			toast.success(`Accepted with edit${moved} — the ${item.authority_slug} dossier updates live.`);
		} catch (err) {
			handleDecisionError(item.id, err, 'Accept failed.');
		} finally {
			card.acting = false;
		}
	}

	async function openReject(item: QueueItem) {
		const card = cardUi(item.id);
		card.rejectOpen = true;
		card.editing = false;
		await tick();
		document.getElementById(`reason-${item.id}`)?.focus();
	}

	async function reject(item: QueueItem) {
		const card = cardUi(item.id);
		const body = rejectBody(
			card.reason,
			card.suppressOn ? { scope: card.scope, lock_topic: card.lockTopic } : undefined
		);
		if (!body) {
			card.error = 'A reason (1–1024 chars) is required to reject.';
			return;
		}
		card.acting = true;
		card.error = null;
		try {
			const out = await rejectFact(api(), item.id, body);
			decided[item.id] = out.status;
			select(nextOpen(item.id));
			toast.success(`Fact #${item.id} rejected${card.suppressOn ? ' and suppressed' : ''}.`);
			void loadSuppressions();
		} catch (err) {
			handleDecisionError(item.id, err, 'Reject failed.');
		} finally {
			card.acting = false;
		}
	}

	// j/k move, a accept, r reject — facts tab, risk view only, never
	// while typing or picking a topic (the select's typeahead takes
	// letters). Shift+key belongs to the by-authority view (Shift+A
	// accepts a whole group there), which handles its own keys.
	function onKey(e: KeyboardEvent) {
		if (tab !== 'facts' || view !== 'risk') return;
		if (e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || e.repeat) return;
		const t = e.target as HTMLElement | null;
		if (t && (t.closest('input, textarea, select, [contenteditable="true"], [role="combobox"], [role="listbox"]') || t.isContentEditable)) return;
		if (document.querySelector('[role="dialog"]')) return;
		const key = e.key.toLowerCase();
		if (key !== 'j' && key !== 'k' && key !== 'a' && key !== 'r') return;
		e.preventDefault();
		if (key === 'j' || key === 'k') {
			if (sorted.length === 0) return;
			const at = sorted.findIndex((i) => i.id === selectedId);
			const step = key === 'j' ? 1 : -1;
			const next = at < 0 ? 0 : Math.min(sorted.length - 1, Math.max(0, at + step));
			select(sorted[next].id);
			return;
		}
		if (!selected || decided[selected.id]) return;
		// An open edit keeps its text: `a` would accept the draft
		// unedited and `r` would close the edit.
		if (ui[selected.id]?.editing) return;
		if (key === 'a') void accept(selected.id);
		else void openReject(selected);
	}

	async function saveRule(event: SubmitEvent) {
		event.preventDefault();
		const body = ruleBody(ruleStatement, ruleAuthority || undefined, ruleTopic || undefined, ruleReason || undefined);
		if (!body) {
			ruleError = 'Statement must be 1–2000 chars; reason ≤1024 chars.';
			return;
		}
		ruleSaving = true;
		ruleError = null;
		try {
			await createRule(api(), body);
			ruleStatement = '';
			ruleAuthority = '';
			ruleTopic = '';
			ruleReason = '';
			await loadRules();
		} catch (err) {
			ruleError = err instanceof Error ? err.message : 'Create rule failed.';
		} finally {
			ruleSaving = false;
		}
	}

	async function revoke(id: number) {
		ruleError = null;
		try {
			await revokeRule(api(), id);
			await loadRules();
			void loadSuppressions();
		} catch (err) {
			ruleError = err instanceof Error ? err.message : 'Revoke failed.';
		}
	}

	function toggleRevoked() {
		showRevoked = !showRevoked;
		void loadRules();
	}

	const TOPIC_CHOICES = topicChoices();
	const tabLabel = (n: number | null) => (n === null ? '' : ` ${n}`);
	// Inactive triggers in full muted ink (the primitive's 60% ink
	// misses 4.5:1 on the muted track).
	const TRIGGER = 'h-8 px-3 text-ink-muted data-active:text-foreground';
	const decidedCount = $derived(Object.keys(decided).length);
</script>

<svelte:window onkeydown={onKey} />

{#snippet filters()}
	<form class="flex min-w-0 flex-1 flex-wrap items-end gap-2" onsubmit={applyFilters} aria-label="Filter the queue">
		<label class="flex min-w-0 flex-1 flex-col gap-1 text-xs font-medium text-ink-muted" for="filter-authority">
			Authority
			<Input id="filter-authority" class="h-9" bind:value={authority} autocomplete="off" />
		</label>
		<label class="flex min-w-0 flex-1 flex-col gap-1 text-xs font-medium text-ink-muted" for="filter-topic">
			Topic
			<Input id="filter-topic" class="h-9" bind:value={topic} autocomplete="off" />
		</label>
		<Button type="submit" variant="outline" size="lg">Filter</Button>
	</form>
{/snippet}
{#snippet viewSwitch()}
	<div class="inline-flex w-fit flex-none gap-1 rounded-md border border-line bg-surface p-1" role="group" aria-label="Facts layout">
		{#each [['risk', 'Riskiest first'], ['authority', 'By authority']] as [value, label] (value)}
			<Button
				size="sm"
				variant="ghost"
				class={view === value ? 'bg-background text-foreground shadow-sm hover:bg-background' : 'text-ink-muted'}
				aria-pressed={view === value}
				onclick={() => setView(value as View)}
			>
				{#if value === 'risk'}<ListOrderedIcon aria-hidden="true" />{:else}<LandmarkIcon aria-hidden="true" />{/if}
				{label}
			</Button>
		{/each}
	</div>
{/snippet}

<svelte:head>
	<title>Review queue — Admin — Apostille Handbook</title>
</svelte:head>

<div class="mt-6 flex flex-col gap-6 pb-12">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<Heading level={1} eyebrow="Content">Review queue</Heading>
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Decide each Muse Spark proposal on its quoted evidence, never on the model's own summary.
				Every decision is audited and reaches the dossier live.
			</p>
		</div>
		<p class={`m-0 hidden items-center gap-1 text-xs text-ink-muted ${tab === 'facts' ? 'md:flex' : ''}`}>
			<Kbd>j</Kbd><Kbd>k</Kbd> move <span aria-hidden="true">·</span>
			{#if view === 'authority'}
				<Kbd>x</Kbd> check <span aria-hidden="true">·</span> <Kbd>a</Kbd> accept <span aria-hidden="true">·</span>
				<Kbd>Shift</Kbd><Kbd>A</Kbd> accept group <span aria-hidden="true">·</span>
			{:else}
				<Kbd>a</Kbd> accept <span aria-hidden="true">·</span>
			{/if}
			<Kbd>r</Kbd> reject
		</p>
	</header>

	{#if refreshNote}
		<p class="m-0 text-sm font-semibold" role="status">{refreshNote}</p>
	{/if}

	<Tabs.Root value={tab} onValueChange={setTab} class="flex-col gap-4">
		<div class="-mx-4 overflow-x-auto px-4 md:mx-0 md:px-0">
			<Tabs.List aria-label="Review sections">
				<Tabs.Trigger value="facts" class={TRIGGER}>Facts{loadState === 'ready' ? ` ${open.length}` : ''}</Tabs.Trigger>
				<Tabs.Trigger value="plans" class={TRIGGER}>Page plans{tabLabel(counts.plans)}</Tabs.Trigger>
				<Tabs.Trigger value="offices" class={TRIGGER}>Offices{tabLabel(counts.offices)}</Tabs.Trigger>
				<Tabs.Trigger value="forms" class={TRIGGER}>Forms{tabLabel(counts.forms)}</Tabs.Trigger>
				<Tabs.Trigger value="translations" class={TRIGGER}>Translations</Tabs.Trigger>
			</Tabs.List>
		</div>

		<Tabs.Content value="facts" class="flex flex-col gap-8">
			{#if view === 'authority'}
				<div class="flex flex-col gap-4">
					<div class="flex flex-col gap-3 md:flex-row md:items-end">
						{@render viewSwitch()}
						{@render filters()}
					</div>
					{#if loadState === 'loading'}
						<div class="flex flex-col gap-4" role="status" aria-label="Loading proposals by authority">
							{#each [0, 1] as i (i)}
								<Skeleton class="h-48 rounded-lg" />
							{/each}
						</div>
					{:else if loadState === 'failed'}
						<Notice variant="error" heading="The review queue could not load.">
							<p class="m-0">{loadError}</p>
							<p class="mt-3 mb-0"><Button variant="outline" onclick={() => loadQueue()}>Try again</Button></p>
						</Notice>
					{:else}
						<AuthorityGroups
							{items}
							{checks}
							bind:decided
							{reduceMotion}
							focusId={focusFact}
							onLoadAll={topic.trim() ? undefined : loadAllFor}
							onRefresh={() => {
								refreshNote = 'Some facts were decided elsewhere — refreshing.';
								void loadQueue(undefined, false, true);
							}}
							onDecided={(status) => {
								if (status === 'rejected') void loadSuppressions();
							}}
							onOpenQueue={openInQueue}
						/>
					{/if}
				</div>
			{:else}
			<div class="grid items-start gap-4 lg:grid-cols-5">
				<div class="flex min-w-0 flex-col gap-3 lg:col-span-3">
					{@render viewSwitch()}
					{@render filters()}

					{#if loadState === 'loading'}
						<div class="flex flex-col gap-2" role="status" aria-label="Loading the review queue">
							{#each [0, 1, 2, 3, 4] as i (i)}
								<Skeleton class="h-16 rounded-sm" />
							{/each}
						</div>
					{:else if loadState === 'failed'}
						<Notice variant="error" heading="The review queue could not load.">
							<p class="m-0">{loadError}</p>
							<p class="mt-3 mb-0"><Button variant="outline" onclick={() => loadQueue()}>Try again</Button></p>
						</Notice>
					{:else if items.length === 0}
						<Card.Root class="items-start px-6 py-10">
							<InboxIcon class="size-6 text-ink-muted" aria-hidden="true" />
							<p class="m-0 text-h3 font-semibold">Queue clear</p>
							<p class="m-0 text-sm text-ink-muted">Nothing proposed right now. Draft from a fresh capture to fill it.</p>
							<Button href={resolve('/(admin)/trace')}>Open the trace console</Button>
						</Card.Root>
					{:else}
						<p class="m-0 text-xs text-ink-muted">
							{queueCount} proposed · {decidedCount} decided this session · riskiest first
						</p>
						<div class="overflow-hidden rounded-md border border-line">
							<table class="block w-full border-collapse text-sm md:table">
								<caption class="sr-only">Proposed facts, riskiest first</caption>
								<thead class="sr-only bg-surface md:not-sr-only">
									<tr class="text-left text-xs text-ink-muted">
										<th scope="col" class="px-3 py-2 font-medium">Proposed fact</th>
										<th scope="col" class="px-3 py-2 font-medium">Signals</th>
										<th scope="col" class="px-3 py-2 text-right font-medium">Age</th>
									</tr>
								</thead>
								<tbody class="block md:table-row-group">
									{#each sorted as item (item.id)}
										{@const decision = decided[item.id]}
										{@const risk = riskOf(item)}
										{@const check = checks[item.id]}
										<tr
											id={`fact-row-${item.id}`}
											class={`flex flex-col border-t border-l-2 border-t-line transition-colors duration-150 md:table-row ${item.id === selectedId ? 'border-l-accent-strong bg-surface' : 'border-l-transparent hover:bg-surface'}`}
											aria-current={item.id === selectedId ? 'true' : undefined}
										>
											<td class="min-w-0 px-3 pt-2 md:py-2">
												{#if decision}
													<p class="m-0 flex items-center gap-2 py-1 font-semibold text-verified" role="status" in:fly={{ y: -4, duration: reduceMotion ? 0 : 150 }}>
														<CheckIcon class="size-4 flex-none" aria-hidden="true" />Fact #{item.id} decided: {decision}.
													</p>
												{:else}
													<button
														type="button"
														class="flex w-full min-w-0 cursor-pointer flex-col gap-0.5 text-left"
														onclick={() => select(item.id)}
													>
														<span class="block max-w-full truncate font-mono text-xs text-ink-muted">#{item.id} · {item.authority_slug} · {item.topic}</span>
														<span class="line-clamp-2 text-ink">{item.statement}</span>
													</button>
												{/if}
											</td>
											<td class="px-3 py-1 md:py-2">
												<span class="flex flex-wrap gap-1">
													{#if risk.badge}
														<Badge variant="outline" class={riskCls(risk.badge)}>{risk.badge}</Badge>
													{/if}
													{#if check && WONDER[check.verdict]}
														<Badge variant="outline" class={WONDER[check.verdict].cls}>
															<SparklesIcon aria-hidden="true" />{WONDER[check.verdict].label}
														</Badge>
													{/if}
												</span>
											</td>
											<td class="px-3 pb-2 text-xs whitespace-nowrap text-ink-muted md:py-2 md:text-right">
												{ago(item.created_at, nowSec())}
											</td>
										</tr>
									{/each}
								</tbody>
							</table>
						</div>
						{#if items.length >= limit}
							<div>
								<Button variant="outline" disabled={loadingMore} onclick={() => loadQueue(items[items.length - 1]?.id, true)}>
									{loadingMore ? 'Working…' : 'Load more'}
								</Button>
							</div>
						{/if}
					{/if}
				</div>

				<aside class="min-w-0 lg:sticky lg:top-20 lg:col-span-2" aria-label="Selected fact">
					{#if loadState !== 'ready' || items.length === 0}
						{#if loadState === 'loading'}
							<Skeleton class="h-96 rounded-md" />
						{/if}
					{:else if !selected || !selectedCard}
						<Card.Root class="items-start px-6 py-10">
							<CheckCheckIcon class="size-6 text-verified" aria-hidden="true" />
							<p class="m-0 text-h3 font-semibold">All caught up</p>
							<p class="m-0 text-sm text-ink-muted">Every proposal in view is decided. The dossiers already show the accepted facts.</p>
							<Button variant="outline" href={resolve('/(admin)/trace')}>Draft more in the trace console</Button>
						</Card.Root>
					{:else}
						{@const item = selected}
						{@const card = selectedCard}
						{@const risk = riskOf(item)}
						{@const check = checks[item.id]}
						<Card.Root class="gap-4 shadow-md">
							<Card.Header>
								<Card.Title class="flex flex-wrap items-center gap-2">
									<h2 class="m-0 text-h3 font-semibold">Fact #{item.id}</h2>
									<Badge variant="secondary">{item.topic}</Badge>
									{#if risk.badge}<Badge variant="outline" class={riskCls(risk.badge)}>{risk.badge}</Badge>{/if}
								</Card.Title>
								<Card.Description>
									<span class="font-mono">{item.authority_slug}</span> · {modelName(item.created_by.split(' ')[0] ?? '')} · {ago(item.created_at, nowSec())}
								</Card.Description>
							</Card.Header>
							<Card.Content class="flex flex-col gap-4">
								{#if decided[item.id]}
									<p class="m-0 text-sm font-semibold text-ink-muted">Already decided ({decided[item.id]}). Press j for the next one.</p>
								{/if}
								{#if item.diff}
									{@const parts = markChanged(item.diff.base_statement, item.statement)}
									<div class="flex flex-col gap-1" aria-label="Change from live fact">
										<p class="m-0 text-xs text-ink-muted">Live fact #{item.diff.base_fact_id} ({item.diff.base_status})</p>
										<p class="m-0 text-body">
											{#each parts.base as seg, i (i)}<span class={seg.changed ? 'rounded-xs bg-error-bg line-through' : ''}>{seg.text}</span>{/each}
										</p>
										<p class="m-0 text-xs text-ink-muted">Proposed</p>
										<p class="m-0 text-body font-semibold">
											{#each parts.next as seg, i (i)}<span class={seg.changed ? 'rounded-xs bg-verified-bg' : ''}>{seg.text}</span>{/each}
										</p>
									</div>
								{:else}
									<p class="m-0 text-body font-semibold">{item.statement}</p>
								{/if}

								<section aria-label="Citations" class="flex flex-col gap-2">
									{#if item.citations.length === 0}
										<p class="m-0 text-sm text-error">No citation recorded — reject or re-draft.</p>
									{:else}
										{#each item.citations as cite, ci (ci)}
											<figure class="m-0 flex flex-col gap-1 rounded-sm border-l-2 border-verified bg-surface px-3 py-2">
												<blockquote class="m-0 font-quote text-body italic">
													{card.quoteOpen || cite.quote.length <= 280 ? cite.quote : `${cite.quote.slice(0, 280)}…`}
												</blockquote>
												<figcaption class="flex flex-wrap items-center gap-2 font-mono text-xs text-ink-muted">
													<CheckIcon class="size-3 text-verified" aria-hidden="true" />quote-verified · capture {cite.capture_id} · bytes {cite.start_byte}–{cite.end_byte}
													{#if cite.quote.length > 280}
														<Button variant="link" size="xs" onclick={() => (card.quoteOpen = !card.quoteOpen)}>
															{card.quoteOpen ? 'Collapse' : 'Expand'} quote
														</Button>
													{/if}
												</figcaption>
											</figure>
										{/each}
									{/if}
									{#if check}
										<p class={`m-0 flex items-start gap-2 text-sm ${check.verdict === 'contradicted' ? 'text-error' : 'text-ink-muted'}`}>
											<SparklesIcon class="mt-0.5 size-4 flex-none" aria-hidden="true" />
											<span>
												{modelName(check.model)} cross-check: {check.verdict.replace('_', ' ')}{check.quote && check.verdict === 'contradicted' ? ` — “${check.quote}”` : '.'}
											</span>
										</p>
									{/if}
								</section>

								<div class="rounded-sm border border-warning bg-warning-bg px-3 py-2" role="note" aria-label="Untrusted model output">
									<p class="m-0 text-xs font-semibold">Untrusted model output — verify against the quote, not this box.</p>
									<p class="m-0 text-sm">{item.reasoning_untrusted ?? 'Model gave no summary.'}</p>
								</div>

								{#if item.blast_radius.conflict}
									<div class="flex flex-col gap-1 rounded-sm bg-error-bg px-3 py-2 text-sm">
										<p class="m-0 flex items-center gap-2 font-semibold"><TriangleAlertIcon class="size-4" aria-hidden="true" />Conflicts with a live fact</p>
										{#each item.blast_radius.conflicts as sib (sib.id)}
											<p class="m-0">#{sib.id} {sib.statement} <span class="font-mono text-xs">({sib.status})</span></p>
										{/each}
									</div>
								{/if}

								<dl class="m-0 grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
									<div>
										<dt class="text-xs text-ink-muted">Source</dt>
										<dd class="m-0">{item.provenance.trust} · {item.provenance.capture_kind} #{item.provenance.capture_id}</dd>
									</div>
									<div>
										<dt class="text-xs text-ink-muted">Authenticity</dt>
										<dd class="m-0">{item.provenance.authenticity}</dd>
									</div>
									<div>
										<dt class="text-xs text-ink-muted">Open tasks</dt>
										<dd class="m-0">{item.blast_radius.open_tasks}</dd>
									</div>
									<div class="min-w-0">
										<dt class="text-xs text-ink-muted">Evidence sidecar</dt>
										<dd class="m-0 truncate font-mono text-xs">{item.provenance.sidecar}</dd>
									</div>
								</dl>

								{#if !decided[item.id]}
									<div class="flex flex-wrap gap-2">
										<Button size="lg" disabled={card.acting} onclick={() => accept(item.id)} aria-keyshortcuts="a">
											Accept
										</Button>
										<Button size="lg" variant="outline" disabled={card.acting} onclick={() => startEdit(item)}>
											Accept with edit
										</Button>
										<Button
											size="lg"
											variant="outline"
											disabled={card.acting}
											aria-keyshortcuts="r"
											aria-expanded={card.rejectOpen}
											onclick={() => (card.rejectOpen ? (card.rejectOpen = false) : openReject(item))}
										>
											{card.rejectOpen ? 'Hide reject' : 'Reject…'}
										</Button>
									</div>
									{#if batch.length >= 2}
										<div>
											<Button variant="secondary" size="lg" disabled={batchActing} onclick={() => acceptBatch()}>
												<CheckCheckIcon aria-hidden="true" />
												Accept all verified from this capture ({batch.length})
											</Button>
										</div>
									{/if}
								{/if}
								{#if batchNote}
									<div class="flex flex-wrap items-center gap-2 rounded-sm bg-error-bg px-3 py-2 text-sm" role="alert">
										<TriangleAlertIcon class="size-4 flex-none text-error" aria-hidden="true" />
										<span class="min-w-0 flex-1 font-semibold">{batchNote.text}</span>
										{#if batchNote.pass.length > 0}
											{@const pass = batchNote.pass}
											<Button size="sm" variant="outline" disabled={batchActing} onclick={() => acceptBatch(pass)}>
												Accept the {pass.length} that pass
											</Button>
										{/if}
									</div>
								{/if}

								{#if card.editing}
									{@const keys = canonicalKeys(card.topic)}
									{@const current = statementKey(card.editText)}
									<div class="flex flex-col gap-2">
										<div class="flex flex-col gap-1">
											<span id={`topic-label-${item.id}`} class="text-sm font-medium">Topic</span>
											<Select.Root
												type="single"
												value={card.topic}
												onValueChange={(value) => {
													if (value) card.topic = value;
												}}
												disabled={card.acting || item.topic === LOCKED_TOPIC}
											>
												<Select.Trigger
													id={`topic-${item.id}`}
													class="h-9 w-full font-mono"
													aria-labelledby={`topic-label-${item.id} topic-${item.id}`}
												>
													{card.topic}
												</Select.Trigger>
												<Select.Content>
													{#each TOPIC_CHOICES as choice (choice.topic)}
														<Select.Item value={choice.topic} label={choice.topic} disabled={!canMoveTopic(item.topic, choice.topic)}>
															<span class="flex min-w-0 flex-col">
																<span class="font-mono">{choice.topic}</span>
																<span class="text-xs text-ink-muted">{choice.where}</span>
															</span>
														</Select.Item>
													{/each}
												</Select.Content>
											</Select.Root>
											{#if card.topic !== item.topic}
												<p class="m-0 text-xs text-ink-muted">Accepting moves it from {item.topic} to {card.topic}.</p>
											{/if}
										</div>
										<label class="flex flex-col gap-1 text-sm font-medium" for={`edit-${item.id}`}>
											Edited statement
											<Textarea id={`edit-${item.id}`} rows={3} bind:value={card.editText} />
										</label>
										{#if keys.length > 0}
											<div class="flex flex-wrap items-center gap-1" role="group" aria-labelledby={`keys-label-${item.id}`}>
												<span id={`keys-label-${item.id}`} class="mr-1 text-xs font-medium text-ink-muted">Key</span>
												{#each keys as chip (chip.key)}
													{@const on = current !== null && splitKey(current).base === chip.key}
													<Button
														size="xs"
														variant={on ? 'secondary' : 'outline'}
														class="font-mono"
														title={chip.label}
														aria-pressed={on}
														disabled={card.acting}
														onclick={() => (card.editText = chipStatement(card.editText, chip.key, card.topic, item.topic))}
													>
														{#if on}<CheckIcon aria-hidden="true" />{/if}{chip.key}
													</Button>
												{/each}
											</div>
										{/if}
										<p class="m-0 text-xs text-ink-muted">{keyNote(card.editText, card.topic)}</p>
										{#if card.editText !== item.statement}
											{@const preview = markChanged(item.statement, card.editText)}
											<p class="m-0 rounded-sm border border-line px-3 py-2 text-sm" aria-label="Diff preview">
												{#each preview.next as seg, pi (pi)}<span class={seg.changed ? 'rounded-xs bg-verified-bg' : ''}>{seg.text}</span>{/each}
											</p>
										{/if}
										<p class="m-0 text-xs text-ink-muted">{card.editText.length}/2000 chars</p>
										<div class="flex gap-2">
											<Button
												disabled={card.acting || card.editText.trim().length === 0 || card.editText.length > 2000}
												onclick={() => acceptEdit(item)}
											>
												{card.acting ? 'Working…' : 'Submit accept'}
											</Button>
											<Button variant="ghost" onclick={() => (card.editing = false)}>Cancel</Button>
										</div>
									</div>
								{/if}

								{#if card.rejectOpen}
									<div class="flex flex-col gap-2">
										<label class="flex flex-col gap-1 text-sm font-medium" for={`reason-${item.id}`}>
											Reject reason (required)
											<Textarea id={`reason-${item.id}`} rows={2} bind:value={card.reason} />
										</label>
										<p class="m-0 text-xs text-ink-muted">{card.reason.trim().length}/1024 chars</p>
										<label class="flex items-start gap-2 text-sm" for={`suppress-${item.id}`}>
											<input id={`suppress-${item.id}`} type="checkbox" class="mt-1" bind:checked={card.suppressOn} />
											Also suppress future copies of this statement
										</label>
										{#if card.suppressOn}
											<fieldset class="m-0 flex flex-col gap-1 border-0 p-0 text-sm">
												<legend class="mb-1 text-xs font-medium text-ink-muted">Scope</legend>
												<label class="flex items-center gap-2"><input type="radio" name={`scope-${item.id}`} value="authority" bind:group={card.scope} />This authority</label>
												<label class="flex items-center gap-2"><input type="radio" name={`scope-${item.id}`} value="global" bind:group={card.scope} />Global (all authorities)</label>
												<label class="mt-1 flex items-center gap-2" for={`lock-${item.id}`}>
													<input id={`lock-${item.id}`} type="checkbox" bind:checked={card.lockTopic} />
													Lock rule to topic “{item.topic}”
												</label>
											</fieldset>
										{/if}
										<div>
											<Button
												variant="destructive"
												disabled={card.acting || card.reason.trim().length === 0 || card.reason.length > 1024}
												onclick={() => reject(item)}
											>
												{card.acting ? 'Working…' : 'Submit reject'}
											</Button>
										</div>
									</div>
								{/if}

								{#if card.error}
									<p class="m-0 text-sm font-semibold text-error" role="alert">{card.error}</p>
								{/if}
							</Card.Content>
						</Card.Root>
					{/if}
				</aside>
			</div>
			{/if}

			<div class="grid items-start gap-4 lg:grid-cols-2">
				<section aria-labelledby="blocked" class="flex flex-col gap-2">
					<h2 id="blocked" class="m-0 text-h3 font-semibold">Blocked by the verifier</h2>
					<p class="m-0 text-sm text-ink-muted">
						Records the verifier refused: unknown topic, bad span, or a value outside its quote.
						Nothing here reaches the dossier.
					</p>
					{#if blockedState === 'loading'}
						<Skeleton class="h-16 rounded-sm" />
					{:else if blockedState === 'failed'}
						<p class="m-0 text-sm text-ink-muted" role="status">{blockedError}</p>
						<div><Button variant="outline" size="sm" onclick={loadBlocked}>Try again</Button></div>
					{:else if blocked.length === 0}
						<p class="m-0 text-sm text-ink-muted">Nothing blocked — every recent draft passed the verifier.</p>
					{:else}
						<p class="m-0 text-xs text-ink-muted">{blockedCount} blocked</p>
						<ul class="m-0 flex list-none flex-col p-0">
							{#each blocked as row (row.id)}
								<li class="flex flex-col gap-0.5 border-t border-line py-2 text-sm">
									<span class="font-mono text-xs text-ink-muted">
										{row.authority_slug} · {row.topic} · attempt {row.attempt} · bytes {row.span_start}–{row.span_end}
									</span>
									<span><span class="font-semibold">{row.kind.replaceAll('_', ' ')}</span>: {row.field} “{row.value}”</span>
								</li>
							{/each}
						</ul>
					{/if}
				</section>

				<section aria-labelledby="skips" class="flex flex-col gap-2">
					<h2 id="skips" class="m-0 text-h3 font-semibold">Suppression log</h2>
					<p class="m-0 text-sm text-ink-muted">Statements the runner skipped: path A is a live duplicate, path B a matched rule.</p>
					{#if skipsState === 'loading'}
						<Skeleton class="h-16 rounded-sm" />
					{:else if skipsState === 'failed'}
						<p class="m-0 text-sm text-ink-muted" role="status">{skipsError}</p>
						<div><Button variant="outline" size="sm" onclick={loadSuppressions}>Try again</Button></div>
					{:else if suppressions.length === 0}
						<p class="m-0 text-sm text-ink-muted">No suppressions recorded.</p>
					{:else}
						<ul class="m-0 flex list-none flex-col p-0">
							{#each suppressions as skip (skip.id)}
								<li class="flex flex-col gap-0.5 border-t border-line py-2 text-sm">
									<span class="font-mono text-xs text-ink-muted">
										#{skip.id} · {skip.authority_slug} · {skip.topic} · path {skip.path}{skip.rule_id === null ? '' : ` · rule #${skip.rule_id}`}
									</span>
									<span>{skip.statement}</span>
								</li>
							{/each}
						</ul>
					{/if}
				</section>
			</div>

			<section aria-labelledby="rules" class="flex flex-col gap-3">
				<div class="flex flex-wrap items-baseline justify-between gap-2">
					<h2 id="rules" class="m-0 text-h3 font-semibold">Suppress rules</h2>
					<label class="flex items-center gap-2 text-sm" for="show-revoked">
						<input id="show-revoked" type="checkbox" checked={showRevoked} onchange={toggleRevoked} />
						Show revoked rules
					</label>
				</div>
				<p class="m-0 max-w-prose text-sm text-ink-muted">
					Rules skip matching statements before they reach the queue. Accepted facts are never
					touched, and every rule is reversible.
				</p>
				<form class="grid gap-2 md:grid-cols-4" onsubmit={saveRule} aria-label="Create a suppress rule">
					<label class="flex flex-col gap-1 text-xs font-medium text-ink-muted md:col-span-4" for="rule-statement">
						Statement to suppress
						<Textarea id="rule-statement" rows={2} bind:value={ruleStatement} />
					</label>
					<label class="flex flex-col gap-1 text-xs font-medium text-ink-muted" for="rule-authority">
						Authority (blank = global)
						<Input id="rule-authority" class="h-9" bind:value={ruleAuthority} autocomplete="off" />
					</label>
					<label class="flex flex-col gap-1 text-xs font-medium text-ink-muted" for="rule-topic">
						Topic (blank = all)
						<Input id="rule-topic" class="h-9" bind:value={ruleTopic} autocomplete="off" />
					</label>
					<label class="flex flex-col gap-1 text-xs font-medium text-ink-muted" for="rule-reason">
						Reason
						<Input id="rule-reason" class="h-9" bind:value={ruleReason} autocomplete="off" />
					</label>
					<div class="flex items-end">
						<Button type="submit" variant="outline" size="lg" disabled={ruleSaving}>{ruleSaving ? 'Saving…' : 'Create rule'}</Button>
					</div>
				</form>
				{#if ruleError}
					<p class="m-0 text-sm font-semibold text-error" role="alert">{ruleError}</p>
				{/if}
				{#if rulesState === 'loading'}
					<Skeleton class="h-16 rounded-sm" />
				{:else if rulesState === 'failed'}
					<p class="m-0 text-sm text-ink-muted" role="status">{rulesError}</p>
					<div><Button variant="outline" size="sm" onclick={loadRules}>Try again</Button></div>
				{:else if rules.length === 0}
					<p class="m-0 text-sm text-ink-muted">No suppress rules.</p>
				{:else}
					<ul class="m-0 flex list-none flex-col p-0">
						{#each rules as rule (rule.id)}
							<li class="flex flex-wrap items-start justify-between gap-2 border-t border-line py-2 text-sm">
								<span class="flex min-w-0 flex-col gap-0.5">
									<span class="font-mono text-xs text-ink-muted">
										Rule #{rule.id} · {rule.authority_slug ?? 'global'} · {rule.topic ?? 'all topics'}{rule.revoked_at !== null ? ' · revoked' : ''}
									</span>
									<span>{rule.statement}</span>
									<span class="text-xs text-ink-muted">{rule.reason || 'no reason'} · by {rule.created_by}</span>
								</span>
								{#if rule.revoked_at === null}
									<Button variant="ghost" size="sm" onclick={() => revoke(rule.id)}>Revoke</Button>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</section>
		</Tabs.Content>

		<Tabs.Content value="plans">
			<PlanReview
				onCount={(n) => (counts.plans = n)}
				initialSlug={composeSlug}
				initialRun={bootTab === 'plans' ? bootRun : null}
			/>
		</Tabs.Content>

		<Tabs.Content value="offices">
			<OfficeReview
				onCount={(n) => (counts.offices = n)}
				initialSlug={composeSlug}
				initialRun={bootTab === 'offices' ? bootRun : null}
			/>
		</Tabs.Content>

		<Tabs.Content value="forms">
			<FormMapReview onCount={(n) => (counts.forms = n)} />
		</Tabs.Content>

		<Tabs.Content value="translations" class="flex flex-col gap-3">
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Muse Spark renders each accepted fact into Spanish and French. A deterministic guard holds
				back any render that drops or rewrites digits, currency amounts or addresses; held rows stay
				English on the public dossier.
			</p>
			<div class="flex flex-wrap gap-2">
				<Button variant="outline" onclick={backfill}>Backfill missing (20)</Button>
				<Button variant="ghost" onclick={() => { void loadAccepted(); void loadTranslations(); }}>Refresh</Button>
			</div>
			{#if trNote}
				<p class="m-0 text-sm font-semibold" role="status">{trNote}</p>
			{/if}
			{#if acceptedState === 'loading' || trState === 'loading' || acceptedState === 'idle'}
				<div class="flex flex-col gap-2" role="status" aria-label="Loading translations">
					<Skeleton class="h-16 rounded-sm" />
					<Skeleton class="h-16 rounded-sm" />
				</div>
			{:else if acceptedState === 'failed'}
				<Notice variant="warning" heading="Accepted facts are unavailable.">
					<p class="m-0">{acceptedError}</p>
					<p class="mt-3 mb-0"><Button variant="outline" onclick={() => loadAccepted()}>Try again</Button></p>
				</Notice>
			{:else if trState === 'failed'}
				<Notice variant="warning" heading="Translations are unavailable.">
					<p class="m-0">{trError}</p>
					<p class="mt-3 mb-0"><Button variant="outline" onclick={() => loadTranslations()}>Try again</Button></p>
				</Notice>
			{:else if acceptedFacts.length === 0}
				<p class="m-0 text-sm text-ink-muted">No accepted facts yet. Accept from the Facts tab first.</p>
			{:else}
				<p class="m-0 text-xs text-ink-muted">{acceptedFacts.length} accepted · {trRows.length} rows</p>
				<ul class="m-0 flex list-none flex-col p-0">
					{#each acceptedFacts as fact (fact.id)}
						<li class="flex flex-col gap-2 border-t border-line py-3">
							<p class="m-0 flex flex-wrap items-center gap-2 text-sm">
								<span class="font-mono text-xs text-ink-muted">Fact #{fact.id}</span>
								<Badge variant="secondary">{fact.topic}</Badge>
								<span class="font-semibold">{fact.statement}</span>
							</p>
							<ul class="m-0 flex list-none flex-col gap-1 p-0">
								{#each ['es', 'fr'] as lang (lang)}
									{@const row = trFor(fact.id, lang)}
									<li class="flex flex-wrap items-center gap-2 text-sm">
										<span class="w-6 font-mono text-xs">{lang}</span>
										<Badge variant="outline">{trLabel(row)}</Badge>
										{#if row?.statement}<span class="min-w-0 break-words text-ink-muted">{row.statement}</span>{/if}
										<Button
											variant="ghost"
											size="sm"
											disabled={trActing[trKey(fact.id, lang)]}
											onclick={() => translateOne(fact.id, lang)}
										>
											{trActing[trKey(fact.id, lang)] ? 'Working…' : 'Translate'}
										</Button>
									</li>
								{/each}
							</ul>
						</li>
					{/each}
				</ul>
			{/if}
		</Tabs.Content>
	</Tabs.Root>
</div>
