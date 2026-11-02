<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import {
		acceptBody,
		acceptFact,
		createRule,
		fetchQueue,
		fetchQueueItem,
		fetchRules,
		fetchSuppressions,
		rejectBody,
		rejectFact,
		revokeRule,
		ruleBody
	} from '../../../lib/admin/review-client.js';
	import { subscribe } from '../../../lib/live.svelte.js';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';

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
		rejectOpen: boolean;
		reason: string;
		suppressOn: boolean;
		scope: string;
		lockTopic: boolean;
		acting: boolean;
		error: string | null;
	}

	// Review queue (analysis-agent.md §11.4): one proposed fact per
	// card, fixed field order, reasoning quarantined. Same API root
	// as the calls page.
	const api = () => `/apostille/api/v1`;

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
	let tab = $state<'queue' | 'translations'>('queue');
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
	let acceptedState = $state<'loading' | 'ready' | 'failed'>('loading');
	let acceptedError = $state<string | null>(null);
	let trRows = $state<TranslationRow[]>([]);
	let trState = $state<'loading' | 'ready' | 'failed'>('loading');
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

	async function readJson(res: Response, what: string) {
		if (!res.ok) {
			const body = await res.json().catch(() => ({}));
			throw new Error(body?.error ? `${what} -> ${body.error}` : `${what} -> ${res.status}`);
		}
		return res.json();
	}

	async function loadAccepted() {
		acceptedState = 'loading';
		acceptedError = null;
		try {
			const res = await fetch(`${api()}/facts?status=accepted&limit=100`);
			const body = await readJson(res, 'accepted facts');
			acceptedFacts = Array.isArray(body?.facts) ? body.facts : [];
			acceptedState = 'ready';
		} catch (err) {
			acceptedError = err instanceof Error ? err.message : 'Accepted facts failed.';
			acceptedState = 'failed';
		}
	}

	async function loadTranslations() {
		trState = 'loading';
		trError = null;
		try {
			const res = await fetch(`${api()}/translations/status`);
			const body = await readJson(res, 'translations');
			trRows = Array.isArray(body?.translations) ? body.translations : [];
			trState = 'ready';
		} catch (err) {
			trError = err instanceof Error ? err.message : 'Translations failed.';
			trState = 'failed';
		}
	}

	function openTranslations() {
		tab = 'translations';
		if (acceptedState === 'loading') void loadAccepted();
		void loadTranslations();
	}

	async function translateOne(factId: number, lang: string) {
		const key = trKey(factId, lang);
		if (trActing[key]) return;
		trActing[key] = true;
		trNote = null;
		try {
			const res = await fetch(`${api()}/facts/${factId}/translate`, {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				body: JSON.stringify({ lang })
			});
			const body = await readJson(res, `translate #${factId}`);
			trNote =
				body?.guard_ok === false
					? `Fact #${factId} (${lang}): held by the number/fee/address guard — English stays live.`
					: `Fact #${factId} (${lang}): translated.`;
			await loadTranslations();
		} catch (err) {
			trNote = err instanceof Error ? err.message : `Translate #${factId} failed.`;
		} finally {
			trActing[key] = false;
		}
	}

	async function backfill() {
		trNote = null;
		try {
			const res = await fetch(`${api()}/translations/backfill`, {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				body: JSON.stringify({ limit: 20 })
			});
			const body = await readJson(res, 'backfill');
			trNote = `Backfill: ${body?.queued ?? 0} queued, ${body?.rendered ?? 0} rendered, ${body?.guard_failed ?? 0} held by guard.`;
			await loadTranslations();
		} catch (err) {
			trNote = err instanceof Error ? err.message : 'Backfill failed.';
		}
	}

	function cardUi(id: number): CardUi {
		let card = ui[id];
		if (!card) {
			card = {
				quoteOpen: false,
				editing: false,
				editText: '',
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

	function age(at: number): string {
		const mins = Math.max(0, Math.floor((Date.now() / 1000 - at) / 60));
		if (mins < 1) return 'just now';
		if (mins < 60) return `${mins}m ago`;
		const hours = Math.floor(mins / 60);
		if (hours < 48) return `${hours}h ago`;
		return `${Math.floor(hours / 24)}d ago`;
	}

	// Riskiest first: changed value vs live sibling, conflict, missing
	// citation, low-trust source. The one-word badge says why.
	const TRUSTED = new Set(['official', 'staff_verified']);

	function riskOf(item: QueueItem): { rank: number; badge: string | null } {
		if (item.diff) return { rank: 0, badge: 'Changed' };
		if (item.blast_radius?.conflict) return { rank: 1, badge: 'Conflict' };
		if (!item.citations || item.citations.length === 0) return { rank: 2, badge: 'Uncited' };
		if (!TRUSTED.has(item.provenance?.trust)) return { rank: 3, badge: 'Low-trust' };
		return { rank: 4, badge: null };
	}

	let sorted = $derived(
		items
			.map((item, index) => ({ item, index }))
			.sort((a, b) => riskOf(a.item).rank - riskOf(b.item).rank || a.index - b.index)
			.map((row) => row.item)
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

	// Records the verifier refused (the staff
	// `/draft-runs/recent` endpoint reads the `analyze.rejected` audit
	// rows). Failed reads keep the queue usable — this section alone
	// reports the error.
	async function loadBlocked() {
		blockedState = 'loading';
		blockedError = null;
		try {
			const res = await fetch(`${api()}/draft-runs/recent?limit=20`);
			if (!res.ok) throw new Error(`blocked -> ${res.status}`);
			const body = await res.json();
			blocked = Array.isArray(body?.rejections) ? body.rejections : [];
			blockedCount = typeof body?.count === 'number' ? body.count : blocked.length;
			blockedState = 'ready';
		} catch (err) {
			blockedError = err instanceof Error ? err.message : 'Blocked list failed.';
			blockedState = 'failed';
		}
	}

	// Word-level change marks via common prefix/suffix trim (page
	// display only — the server stores the verbatim statements).
	interface DiffSeg {
		text: string;
		changed: boolean;
	}

	function markChanged(base: string, next: string): { base: DiffSeg[]; next: DiffSeg[] } {
		const a = base.split(/(\s+)/);
		const b = next.split(/(\s+)/);
		let pre = 0;
		while (pre < a.length && pre < b.length && a[pre] === b[pre]) pre++;
		let suf = 0;
		while (suf < a.length - pre && suf < b.length - pre && a[a.length - 1 - suf] === b[b.length - 1 - suf])
			suf++;
		const segs = (words: string[]): DiffSeg[] =>
			words.map((text, i) => ({ text, changed: i >= pre && i < words.length - suf }));
		return { base: segs(a), next: segs(b) };
	}

	function trustChip(trust: string): string {
		const base = 'inline-block rounded border px-2 py-0.5 text-sm font-bold';
		if (trust === 'official') return `${base} border-success bg-success-bg text-success`;
		if (trust === 'staff_verified') return `${base} border-accent-fill bg-surface text-accent-strong`;
		return `${base} border-line bg-surface-2 text-ink-muted`;
	}

	function syncUrl() {
		const params = new URLSearchParams();
		if (authority.trim()) params.set('authority', authority.trim());
		if (topic.trim()) params.set('topic', topic.trim());
		if (limit !== 50) params.set('limit', String(limit));
		const query = params.toString();
		history.replaceState(null, '', query ? `?${query}` : location.pathname);
	}

	async function loadQueue(afterId?: number, append = false) {
		if (append) {
			loadingMore = true;
		} else {
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
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Review queue failed.';
			loadState = append ? loadState : 'failed';
		} finally {
			loadingMore = false;
		}
	}

	function applyFilters(event: SubmitEvent) {
		event.preventDefault();
		syncUrl();
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
	// proposals publishes `proposal.created` per fact — insert the card
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
		} catch {
			// The next queue refresh picks it up; never break the feed.
		}
	}

	onMount(() => {
		const params = new URLSearchParams(location.search);
		authority = params.get('authority') ?? '';
		topic = params.get('topic') ?? '';
		const raw = Number(params.get('limit'));
		if ([25, 50, 100, 200].includes(raw)) limit = raw;
		void loadQueue();
		void loadRules();
		void loadSuppressions();
		void loadBlocked();
		const unsub = subscribe('proposals', (msg) => {
			void onProposal(msg);
		});
		return () => unsub();
	});

	async function accept(id: number) {
		const card = cardUi(id);
		if (decided[id] || card.acting) return;
		card.acting = true;
		card.error = null;
		// Optimistic: the card reads decided the moment staff clicks;
		// a failed POST rolls it back below.
		decided[id] = 'accepted';
		try {
			const out = await acceptFact(api(), id, acceptBody(undefined, undefined) ?? {});
			decided[id] = out.status;
		} catch (err) {
			delete decided[id];
			const message = err instanceof Error ? err.message : 'Accept failed.';
			if (/409|not proposed/.test(message)) {
				refreshNote = `Fact #${id}: already decided — refreshing.`;
				await loadQueue();
			} else {
				card.error = message;
			}
		} finally {
			card.acting = false;
		}
	}

	function startEdit(item: QueueItem) {
		const card = cardUi(item.id);
		card.editing = true;
		card.editText = item.statement;
		card.error = null;
	}

	async function acceptEdit(item: QueueItem) {
		const card = cardUi(item.id);
		const unchanged = card.editText === item.statement;
		const body = acceptBody(undefined, unchanged ? undefined : card.editText);
		if (!body) {
			card.error = 'Edited statement must be 1–2000 chars.';
			return;
		}
		card.acting = true;
		card.error = null;
		try {
			const out = await acceptFact(api(), item.id, body);
			decided[item.id] = out.status;
		} catch (err) {
			const message = err instanceof Error ? err.message : 'Accept failed.';
			if (/409|not proposed/.test(message)) {
				refreshNote = `Fact #${item.id}: already decided — refreshing.`;
				await loadQueue();
			} else {
				card.error = message;
			}
		} finally {
			card.acting = false;
		}
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
			void loadSuppressions();
		} catch (err) {
			const message = err instanceof Error ? err.message : 'Reject failed.';
			if (/409|not proposed/.test(message)) {
				refreshNote = `Fact #${item.id}: already decided — refreshing.`;
				await loadQueue();
			} else {
				card.error = message;
			}
		} finally {
			card.acting = false;
		}
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
</script>

<svelte:head>
	<title>Review queue — Admin — Apostille Handbook</title>
</svelte:head>

<h1>Review queue</h1>

<Notice variant="warning" heading="One click, one fact.">
	<p>
		Accept or reject each proposed fact on its citations — never on the model's own
		summary. Decisions are audited; conflicts never block, and the server (not this
		page) is the arbiter of what is decided.
	</p>
</Notice>

{#if refreshNote}
	<p class="font-bold" role="status">{refreshNote}</p>
{/if}

<div role="tablist" aria-label="Review sections" class="flex flex-wrap gap-2">
		<button
			type="button"
			role="tab"
			aria-selected={tab === 'queue'}
			disabled={tab === 'queue'}
			onclick={() => (tab = 'queue')}
		>
			Queue
		</button>
		<button
			type="button"
			role="tab"
			aria-selected={tab === 'translations'}
			disabled={tab === 'translations'}
			onclick={openTranslations}
		>
			Translations
		</button>
	</div>

{#if tab === 'queue'}
<section aria-labelledby="filters">
	<h2 id="filters">Queue</h2>
	<form class="flex flex-wrap items-end gap-3" onsubmit={applyFilters}>
		<p class="m-0">
			<label class="grid gap-1 font-bold" for="filter-authority">
				Authority
				<input
					class="p-2 font-normal"
					id="filter-authority"
					type="text"
					bind:value={authority}
					placeholder="usa-wa-sos"
					autocomplete="off"
				/>
			</label>
		</p>
		<p class="m-0">
			<label class="grid gap-1 font-bold" for="filter-topic">
				Topic
				<input
					class="p-2 font-normal"
					id="filter-topic"
					type="text"
					bind:value={topic}
					placeholder="fees"
					autocomplete="off"
				/>
			</label>
		</p>
		<p class="m-0">
			<label class="grid gap-1 font-bold" for="filter-limit">
				Per page
				<select class="p-2 font-normal" id="filter-limit" bind:value={limit}>
					<option value={25}>25</option>
					<option value={50}>50</option>
					<option value={100}>100</option>
					<option value={200}>200</option>
				</select>
			</label>
		</p>
		<p class="m-0"><button type="submit">Apply</button></p>
	</form>

	{#if loadState === 'loading'}
		<p>Loading…</p>
	{:else if loadState === 'failed'}
		<p class="font-bold text-error" role="alert">{loadError}</p>
		<p><button type="button" onclick={() => loadQueue()}>Retry</button></p>
	{:else if items.length === 0}
		<p class="text-ink-muted">Queue clear — nothing proposed right now.</p>
	{:else}
		<p class="text-sm text-ink-muted">{queueCount} proposed</p>
		<ol class="m-0 mt-4 list-none p-0 pb-8">
			{#each sorted as item (item.id)}
				{@const card = ui[item.id]}
				{@const decision = decided[item.id]}
				{@const risk = riskOf(item)}
				<li class="border-b border-line py-4">
					{#if decision}
						<p class="m-0 font-bold" role="status">
							Fact #{item.id} decided: {decision}.
						</p>
					{:else}
						<article aria-label={`Proposed fact ${item.id}`}>
							<header class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
								<h3 class="m-0 text-lg">
									<a href={`${api()}/queue/${item.id}`}>Fact #{item.id}</a>
								</h3>
								<span class="font-mono text-sm">{item.authority_slug}</span>
								<span class="inline-block rounded border border-line bg-surface-2 px-2 py-0.5 text-sm font-bold">
									{item.topic}
								</span>
								{#if risk.badge}
									<span class="inline-block rounded border border-warning bg-warning-bg px-2 py-0.5 text-sm font-bold">
										{risk.badge}
									</span>
								{/if}
								<span class="text-sm text-ink-muted">
									by {item.created_by} · {age(item.created_at)}
								</span>
							</header>

							{#if item.diff}
								{@const parts = markChanged(item.diff.base_statement, item.statement)}
								<section aria-label="Change from live fact">
									<p class="mb-1 text-sm font-bold text-ink-muted">
										Live fact #{item.diff.base_fact_id} [{item.diff.base_status}] →
									</p>
									<p class="m-0 mb-1">
										{#each parts.base as seg, i (i)}<span
											class={seg.changed ? 'bg-error-bg' : ''}
										>{seg.text}</span>{/each}
									</p>
									<p class="m-0 mb-1 font-bold">↓ proposed:</p>
									<p class="m-0">
										{#each parts.next as seg, i (i)}<span
											class={seg.changed ? 'bg-success-bg' : ''}
										>{seg.text}</span>{/each}
									</p>
								</section>
							{:else}
								<section aria-label="No live sibling">
									<p class="m-0 font-bold">{item.statement}</p>
									<p class="text-sm text-ink-muted">
										No live sibling — first claim on this topic.
									</p>
								</section>
							{/if}

							<section aria-label="Citations">
								<h4 class="mb-1 text-base">Citations</h4>
								{#if item.citations.length === 0}
									<p class="text-sm text-ink-muted">No citation recorded.</p>
								{:else}
									<ul class="m-0 list-none p-0">
										{#each item.citations as cite, ci (ci)}
											<li class="mb-1">
												<span class="font-mono text-sm">
													capture {cite.capture_id} · bytes {cite.start_byte}–{cite.end_byte}
												</span>
												<code class="block max-w-[60rem] overflow-x-auto p-2">
													{card.quoteOpen || cite.quote.length <= 280
														? cite.quote
														: `${cite.quote.slice(0, 280)}…`}
												</code>
												{#if cite.quote.length > 280}
													<button type="button" onclick={() => (card.quoteOpen = !card.quoteOpen)}>
														{card.quoteOpen ? 'Collapse' : 'Expand'} quote
													</button>
												{/if}
											</li>
										{/each}
									</ul>
								{/if}
							</section>

							<div class="my-3 max-w-[60rem] rounded border-2 border-warning bg-warning-bg p-3" role="note" aria-label="Untrusted model output">
								<p class="m-0 mb-1 text-sm font-bold">
									Untrusted model output — verify against citations, not this box.
								</p>
								{#if item.reasoning_untrusted}
									<p class="m-0 text-sm">{item.reasoning_untrusted}</p>
								{:else}
									<p class="m-0 text-sm italic">Model gave no summary — review citations.</p>
								{/if}
							</div>

							<section aria-label="Blast radius">
								<h4 class="mb-1 text-base">Blast radius</h4>
								{#if item.blast_radius.conflict}
									<Notice variant="warning" heading="Conflicting reports for this item.">
										{#each item.blast_radius.conflicts as sib (sib.id)}
											<p>{sib.statement} <span class="font-mono">[{sib.status}]</span></p>
										{/each}
									</Notice>
								{:else}
									<p class="text-sm text-ink-muted">No conflicting siblings.</p>
								{/if}
								<p class="text-sm">
									<a href={resolve('/(admin)/triage')}>
										{item.blast_radius.open_tasks} open task{item.blast_radius.open_tasks === 1 ? '' : 's'}
									</a>
								</p>
							</section>

							<section aria-label="Provenance">
								<h4 class="mb-1 text-base">Provenance</h4>
								<p class="m-0 flex flex-wrap items-center gap-2 text-sm">
									<span class={trustChip(item.provenance.trust)}>{item.provenance.trust}</span>
									<span>{item.provenance.capture_kind} capture #{item.provenance.capture_id}</span>
									<span class="text-ink-muted">authenticity: {item.provenance.authenticity}</span>
								</p>
								<p class="text-sm text-ink-muted">
									Evidence sidecar: <code>{item.provenance.sidecar}</code>
								</p>
							</section>

							<div class="mt-3 flex flex-wrap gap-2">
								<button type="button" disabled={card.acting} onclick={() => accept(item.id)}>
									{card.acting ? 'Working…' : 'Accept'}
								</button>
								<button type="button" disabled={card.acting} onclick={() => startEdit(item)}>
									Accept with edit
								</button>
								<button
									type="button"
									disabled={card.acting}
									onclick={() => (card.rejectOpen = !card.rejectOpen)}
								>
									{card.rejectOpen ? 'Hide reject' : 'Reject…'}
								</button>
							</div>

							{#if card.editing}
								<div class="mt-2 max-w-[60rem]">
									<label class="grid gap-1 font-bold" for={`edit-${item.id}`}>
										Edited statement
										<textarea
											class="p-2 font-normal"
											id={`edit-${item.id}`}
											rows="3"
											bind:value={card.editText}
										></textarea>
									</label>
									{#if card.editText !== item.statement}
										{@const preview = markChanged(item.statement, card.editText)}
										<div class="mt-2 border border-line p-2" aria-label="Diff preview">
											<p class="m-0 mb-1 text-sm font-bold text-ink-muted">
												Diff preview (edited vs current proposal):
											</p>
											<p class="m-0">
												{#each preview.next as seg, pi (pi)}<span
													class={seg.changed ? 'bg-success-bg' : ''}
												>{seg.text}</span>{/each}
											</p>
										</div>
									{/if}
									<p class="text-sm text-ink-muted">{card.editText.length}/2000 chars</p>
									<div class="flex gap-2">
										<button
											type="button"
											disabled={card.acting ||
												card.editText.trim().length === 0 ||
												card.editText.length > 2000}
											onclick={() => acceptEdit(item)}
										>
											{card.acting ? 'Working…' : 'Submit accept'}
										</button>
										<button type="button" onclick={() => (card.editing = false)}>Cancel</button>
									</div>
								</div>
							{/if}

							{#if card.rejectOpen}
								<div class="mt-2 max-w-[60rem]">
									<label class="grid gap-1 font-bold" for={`reason-${item.id}`}>
										Reject reason (required)
										<textarea
											class="p-2 font-normal"
											id={`reason-${item.id}`}
											rows="2"
											bind:value={card.reason}
										></textarea>
									</label>
									<p class="text-sm text-ink-muted">{card.reason.trim().length}/1024 chars</p>
									<details class="mb-2 border border-line p-2">
										<summary class="cursor-pointer font-bold">Also suppress future copies</summary>
										<p>
											<label class="flex items-start gap-2" for={`suppress-${item.id}`}>
												<input
													id={`suppress-${item.id}`}
													type="checkbox"
													bind:checked={card.suppressOn}
												/>
												Create a suppress rule for this statement
											</label>
										</p>
										{#if card.suppressOn}
											<p>
												<label class="grid max-w-[20rem] gap-1 font-bold" for={`scope-${item.id}`}>
													Scope
													<select
														class="p-2 font-normal"
														id={`scope-${item.id}`}
														bind:value={card.scope}
													>
														<option value="authority">This authority</option>
														<option value="global">Global (all authorities)</option>
													</select>
												</label>
											</p>
											<p>
												<label class="flex items-start gap-2" for={`lock-${item.id}`}>
													<input id={`lock-${item.id}`} type="checkbox" bind:checked={card.lockTopic} />
													Lock rule to topic “{item.topic}”
												</label>
											</p>
										{/if}
									</details>
									<p>
										<button
											type="button"
											disabled={card.acting ||
												card.reason.trim().length === 0 ||
												card.reason.length > 1024}
											onclick={() => reject(item)}
										>
											{card.acting ? 'Working…' : 'Submit reject'}
										</button>
									</p>
								</div>
							{/if}

							{#if card.error}
								<p class="font-bold text-error" role="alert">{card.error}</p>
							{/if}
						</article>
					{/if}
				</li>
			{/each}
		</ol>
		{#if items.length >= limit}
			<p>
				<button
					type="button"
					disabled={loadingMore}
					onclick={() => loadQueue(items[items.length - 1]?.id, true)}
				>
					{loadingMore ? 'Loading…' : 'Load more'}
				</button>
			</p>
		{/if}
	{/if}
</section>
{:else}
<section aria-labelledby="translations">
	<h2 id="translations">Translations</h2>
	<p class="max-w-[60rem] text-sm text-ink-muted">
		Muse Spark renders each accepted fact into Spanish and French. A deterministic
		guard holds back any render that drops or rewrites digits, currency amounts, or
		addresses — held rows stay English on the public dossier.
	</p>
	<p class="flex flex-wrap gap-2">
		<button type="button" onclick={backfill}>Backfill missing (20)</button>
		<button type="button" onclick={() => { void loadAccepted(); void loadTranslations(); }}>
			Refresh
		</button>
	</p>
	{#if trNote}
		<p class="font-bold" role="status">{trNote}</p>
	{/if}
	{#if acceptedState === 'loading' || trState === 'loading'}
		<p>Loading…</p>
	{:else if acceptedState === 'failed'}
		<p class="font-bold text-error" role="alert">{acceptedError}</p>
		<p><button type="button" onclick={() => loadAccepted()}>Retry</button></p>
	{:else if trState === 'failed'}
		<p class="font-bold text-error" role="alert">{trError}</p>
		<p><button type="button" onclick={() => loadTranslations()}>Retry</button></p>
	{:else if acceptedFacts.length === 0}
		<p class="text-ink-muted">No accepted facts yet — accept from the queue first.</p>
	{:else}
		<p class="text-sm text-ink-muted">{acceptedFacts.length} accepted · {trRows.length} rows</p>
		<ol class="m-0 mt-4 list-none p-0 pb-8">
			{#each acceptedFacts as fact (fact.id)}
				<li class="border-b border-line py-4">
					<article aria-label={`Accepted fact ${fact.id}`}>
						<header class="flex flex-wrap items-baseline gap-x-3 gap-y-1">
							<h3 class="m-0 text-lg">Fact #{fact.id}</h3>
							<span class="inline-block rounded border border-line bg-surface-2 px-2 py-0.5 text-sm font-bold">
								{fact.topic}
							</span>
						</header>
						<p class="m-0 font-bold">{fact.statement}</p>
						<ul class="m-0 mt-2 list-none p-0">
							{#each ['es', 'fr'] as lang (lang)}
								{@const row = trFor(fact.id, lang)}
								<li class="mb-1 flex flex-wrap items-center gap-2 text-sm">
									<span class="font-mono">{lang}</span>
									<span
										class="inline-block rounded border border-line bg-surface-2 px-2 py-0.5 text-sm font-bold"
									>
										{trLabel(row)}
									</span>
									{#if row?.statement}
										<span class="max-w-[60rem] break-words text-ink-muted">{row.statement}</span>
									{/if}
									{#if row?.model}
										<span class="font-mono text-xs text-ink-muted">{row.model}</span>
									{/if}
									<button
										type="button"
										disabled={trActing[trKey(fact.id, lang)]}
										onclick={() => translateOne(fact.id, lang)}
									>
										{trActing[trKey(fact.id, lang)] ? 'Working…' : 'Translate'}
									</button>
								</li>
							{/each}
						</ul>
					</article>
				</li>
			{/each}
		</ol>
	{/if}
</section>
{/if}

<section aria-labelledby="blocked">
	<h2 id="blocked">Blocked by the verifier</h2>
	{#if blockedState === 'ready'}
		<p class="text-sm text-ink-muted">{blockedCount} blocked</p>
	{/if}
	<p class="max-w-[60rem] text-sm text-ink-muted">
		Records the verifier refused — unknown topic, bad span, or a value outside its
		quote. Fix the evidence and re-draft; nothing here reaches the dossier.
	</p>
	{#if blockedState === 'failed'}
		<p class="font-bold text-error" role="alert">{blockedError}</p>
		<p><button type="button" onclick={loadBlocked}>Retry</button></p>
	{:else if blockedState === 'ready' && blocked.length === 0}
		<p class="text-ink-muted">Nothing blocked — every recent draft passed the verifier.</p>
	{:else if blockedState === 'ready'}
		<ul class="m-0 mt-4 list-none p-0 pb-8">
			{#each blocked as row (row.id)}
				<li class="border-b border-line py-[0.7rem]">
					<p class="m-0">
						<span class="font-mono text-sm">{row.authority_slug}</span>
						<span class="inline-block rounded border border-line bg-surface-2 px-2 py-0.5 text-sm font-bold">
							{row.topic}
						</span>
						<span class="text-sm text-ink-muted">
							attempt {row.attempt} · bytes {row.span_start}–{row.span_end}
						</span>
					</p>
					<p class="m-0 text-sm">
						<span class="font-bold">{row.kind}</span> — {row.field}: {row.value}
					</p>
				</li>
			{/each}
		</ul>
	{/if}
</section>

<section aria-labelledby="rules">
	<h2 id="rules">Suppress rules</h2>
	<p class="max-w-[60rem] text-sm text-ink-muted">
		Rules skip matching statements before they reach the queue (suppression is skip-only —
		accepted facts are never touched). Every rule is reversible.
	</p>
	<form class="max-w-[60rem]" onsubmit={saveRule}>
		<p>
			<label class="grid gap-1 font-bold" for="rule-statement">
				Statement to suppress
				<textarea
					class="p-2 font-normal"
					id="rule-statement"
					rows="2"
					bind:value={ruleStatement}
				></textarea>
			</label>
		</p>
		<div class="flex flex-wrap gap-3">
			<p class="m-0">
				<label class="grid gap-1 font-bold" for="rule-authority">
					Authority (blank = global)
					<input
						class="p-2 font-normal"
						id="rule-authority"
						type="text"
						bind:value={ruleAuthority}
						autocomplete="off"
					/>
				</label>
			</p>
			<p class="m-0">
				<label class="grid gap-1 font-bold" for="rule-topic">
					Topic (blank = all)
					<input
						class="p-2 font-normal"
						id="rule-topic"
						type="text"
						bind:value={ruleTopic}
						autocomplete="off"
					/>
				</label>
			</p>
			<p class="m-0">
				<label class="grid gap-1 font-bold" for="rule-reason">
					Reason
					<input
						class="p-2 font-normal"
						id="rule-reason"
						type="text"
						bind:value={ruleReason}
						autocomplete="off"
					/>
				</label>
			</p>
		</div>
		{#if ruleError}
			<p class="font-bold text-error" role="alert">{ruleError}</p>
		{/if}
		<p>
			<button type="submit" disabled={ruleSaving}>{ruleSaving ? 'Saving…' : 'Create rule'}</button>
		</p>
	</form>

	<p>
		<label class="flex items-start gap-2" for="show-revoked">
			<input id="show-revoked" type="checkbox" checked={showRevoked} onchange={toggleRevoked} />
			Show revoked rules
		</label>
	</p>
	{#if rulesState === 'loading'}
		<p>Loading…</p>
	{:else if rulesState === 'failed'}
		<p class="font-bold text-error" role="alert">{rulesError}</p>
		<p><button type="button" onclick={loadRules}>Retry</button></p>
	{:else if rules.length === 0}
		<p class="text-ink-muted">No suppress rules.</p>
	{:else}
		<ul class="m-0 mt-4 list-none p-0 pb-8">
			{#each rules as rule (rule.id)}
				<li class="border-b border-line py-[0.7rem]">
					<p class="m-0">
						<span class="font-bold">Rule #{rule.id}</span>
						<span class="font-mono text-sm">
							{rule.authority_slug ?? 'global'} · {rule.topic ?? 'all topics'}
						</span>
						{#if rule.revoked_at !== null}
							<span class="text-sm font-bold text-error">revoked</span>
						{/if}
					</p>
					<p class="m-0 text-sm">{rule.statement}</p>
					<p class="m-0 text-sm text-ink-muted">
						{rule.reason || 'no reason'} · by {rule.created_by}
					</p>
					{#if rule.revoked_at === null}
						<p class="m-0 mt-1">
							<button type="button" onclick={() => revoke(rule.id)}>Revoke</button>
						</p>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</section>

<section aria-labelledby="skips">
	<h2 id="skips">Suppression log</h2>
	<p class="max-w-[60rem] text-sm text-ink-muted">
		Read-only: statements the runner skipped. Path A = live duplicate, path B = matched
		rule.
	</p>
	{#if skipsState === 'loading'}
		<p>Loading…</p>
	{:else if skipsState === 'failed'}
		<p class="font-bold text-error" role="alert">{skipsError}</p>
		<p><button type="button" onclick={loadSuppressions}>Retry</button></p>
	{:else if suppressions.length === 0}
		<p class="text-ink-muted">No suppressions recorded.</p>
	{:else}
		<div class="overflow-x-auto pb-8">
			<table>
				<thead>
					<tr>
						<th scope="col">ID</th>
						<th scope="col">Authority</th>
						<th scope="col">Topic</th>
						<th scope="col">Path</th>
						<th scope="col">Rule</th>
						<th scope="col">Statement</th>
					</tr>
				</thead>
				<tbody>
					{#each suppressions as skip (skip.id)}
						<tr>
							<td>#{skip.id}</td>
							<td class="font-mono text-sm">{skip.authority_slug}</td>
							<td>{skip.topic}</td>
							<td>{skip.path}</td>
							<td>{skip.rule_id === null ? '—' : `#${skip.rule_id}`}</td>
							<td class="max-w-[40rem]">{skip.statement}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</section>
									
