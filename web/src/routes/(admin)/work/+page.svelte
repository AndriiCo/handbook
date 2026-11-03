<script lang="ts">
	import { onMount } from 'svelte';
	import { toast } from 'svelte-sonner';
	import Notice from '#lib/public-ui/Notice.svelte';
	import Heading from '#lib/components/brand/Heading.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import * as Select from '#lib/components/ui/select/index.js';
	import * as Sheet from '#lib/components/ui/sheet/index.js';
	import * as Table from '#lib/components/ui/table/index.js';
	import { ago, dateTime } from '#lib/admin/format.js';
	import {
		DEFAULT_ATTEMPTS,
		LIST_LIMIT,
		MAX_ATTEMPTS,
		STATUSES,
		WorkError,
		auditLabel,
		canStagePacks,
		enqueueBody,
		enqueueWork,
		examplePayload,
		fetchResultText,
		fetchWorkItem,
		fetchWorkList,
		fmtBytes,
		isJsonType,
		isStatus,
		leaseText,
		parsePayload,
		poll,
		pretty,
		resultHref,
		resultView,
		retryWork,
		stagePacks,
		stagedText,
		statusChanges,
		statusLabel,
		statusTone,
		type WorkDetail,
		type WorkItem,
		type WorkList
	} from '#lib/admin/work-client.js';
	import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
	import SendIcon from '@lucide/svelte/icons/send';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import EyeIcon from '@lucide/svelte/icons/eye';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import PlugZapIcon from '@lucide/svelte/icons/plug-zap';
	import UnplugIcon from '@lucide/svelte/icons/unplug';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import LoaderIcon from '@lucide/svelte/icons/loader-circle';
	import PackagePlusIcon from '@lucide/svelte/icons/package-plus';
	import PackageCheckIcon from '@lucide/svelte/icons/package-check';

	// Production work queue (work-queue.md §3): staff enqueue jobs here,
	// plant workers dial out over one WebSocket and check them out one
	// at a time. The list re-reads every 5 s while the tab is visible
	// (no ops-topic events for work items yet); the detail sheet
	// re-reads when its row moves. Results download as a plain GET
	// (cookie auth); POSTs carry the CSRF pair via work-client.js.
	// A finished research.nonparty result can be staged into
	// <store>/packs-inbox/<id>/ for the Packs page (re-staging overwrites).
	const API = '/apostille/api/v1';

	let list = $state<WorkList | null>(null);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let pollError = $state<string | null>(null);
	let refreshing = $state(false);
	let filter = $state('all');
	let lastRead = $state<number | null>(null);
	let nowMs = $state(Date.now());
	let announce = $state('');

	let kind = $state('');
	let payloadText = $state('');
	let maxAttempts = $state<number | null>(DEFAULT_ATTEMPTS);
	let formError = $state<{ field: 'kind' | 'payload' | 'max_attempts' | 'server'; text: string } | null>(null);
	let enqueuing = $state(false);

	let selectedId = $state<number | null>(null);
	let sheetOpen = $state(false);
	let sheetEl = $state<HTMLElement | null>(null);
	let detail = $state<WorkDetail | null>(null);
	let detailState = $state<'loading' | 'ready' | 'failed'>('loading');
	let detailError = $state<string | null>(null);
	let viewer = $state<{
		state: 'idle' | 'loading' | 'ready' | 'failed';
		view: ReturnType<typeof resultView> | null;
		error: string | null;
	}>({ state: 'idle', view: null, error: null });
	let retrying = $state<Record<number, true>>({});
	let staging = $state<Record<number, true>>({});
	let stageNote = $state<{ id: number; ok: boolean; text: string } | null>(null);

	let poller: ReturnType<typeof poll> | null = null;
	let seq = 0;
	let lastStatus = new Map<number, string>();

	const FILTERS = [
		{ value: 'all', label: 'All statuses' },
		...STATUSES.map((s) => ({ value: s, label: statusLabel(s) }))
	];

	// Server clock: the list's Date header corrects a skewed laptop.
	const now = $derived(Math.floor(nowMs / 1000) + (list?.offset ?? 0));
	const kinds = $derived(list?.kinds ?? []);
	const capable = $derived(new Set(list?.workers.flatMap((w) => w.capabilities) ?? []));
	const chosenKind = $derived(kinds.find((k) => k.kind === kind) ?? null);
	const payloadCheck = $derived(parsePayload(payloadText));
	const payloadBytes = $derived(payloadCheck.ok ? new TextEncoder().encode(JSON.stringify(payloadCheck.payload)).length : 0);
	const selectedRow = $derived(list?.items.find((i) => i.id === selectedId) ?? null);
	const shown = $derived(detail?.item ?? selectedRow);
	const filterLabel = $derived(FILTERS.find((f) => f.value === filter)?.label ?? 'All statuses');

	const errorText = (err: unknown, fallback: string) => (err instanceof Error && err.message ? err.message : fallback);
	const iso = (at: number) => new Date(at * 1000).toISOString();

	async function load(quiet = false) {
		const mine = ++seq;
		const wanted = filter;
		if (quiet) refreshing = true;
		else {
			loadState = 'loading';
			loadError = null;
		}
		try {
			const next = await fetchWorkList(API, wanted);
			// A newer read (or a filter change) owns the screen now.
			if (mine !== seq || wanted !== filter) return;
			if (list) {
				const lines = statusChanges(lastStatus, next.items);
				if (lines.length > 3) announce = `${lines.slice(0, 3).join(' ')} ${lines.length - 3} more changed.`;
				else if (lines.length > 0) announce = lines.join(' ');
			}
			lastStatus = new Map(next.items.map((i) => [i.id, i.status]));
			list = next;
			loadState = 'ready';
			loadError = null;
			pollError = null;
			lastRead = Date.now();
			if (!kind && next.kinds.length > 0) pickKind(next.kinds[0].kind);
			syncDetail(next.items);
		} catch (err) {
			if (mine !== seq) return;
			const text = errorText(err, 'The work list failed to load.');
			if (loadState === 'ready' && list) pollError = text;
			else {
				loadError = text;
				loadState = 'failed';
			}
		} finally {
			if (mine === seq) refreshing = false;
			nowMs = Date.now();
		}
	}

	function refresh() {
		if (poller) poller.now();
		else void load(true);
	}

	function pickKind(value: string) {
		kind = value;
		payloadText = examplePayload(list?.kinds ?? [], value);
		formError = null;
	}

	function fieldId(field: 'kind' | 'payload' | 'max_attempts' | 'server'): string {
		return field === 'kind' ? 'work-kind' : field === 'max_attempts' ? 'work-attempts' : 'work-payload';
	}

	async function enqueue() {
		formError = null;
		const built = enqueueBody(kind, payloadText, maxAttempts);
		if (!built.ok) {
			formError = { field: built.field, text: built.error };
			document.getElementById(fieldId(built.field))?.focus();
			return;
		}
		enqueuing = true;
		try {
			const created = await enqueueWork(API, built.body);
			toast.success(`Job ${created.id} queued (${statusLabel(created.status).toLowerCase()}).`);
			refresh();
		} catch (err) {
			formError = { field: 'server', text: errorText(err, 'The job was not queued.') };
		} finally {
			enqueuing = false;
		}
	}

	async function retry(id: number) {
		if (retrying[id]) return;
		retrying = { ...retrying, [id]: true };
		try {
			const out = await retryWork(API, id);
			toast.success(`Job ${out.id} is ${statusLabel(out.status).toLowerCase()} again, with its attempts reset.`);
			refresh();
			if (selectedId === id) void loadDetail(id, true);
		} catch (err) {
			toast.error(errorText(err, `Job ${id} could not be retried.`));
			if (err instanceof WorkError && err.status === 409) refresh();
		} finally {
			const next = { ...retrying };
			delete next[id];
			retrying = next;
		}
	}

	async function stage(id: number) {
		if (staging[id]) return;
		staging = { ...staging, [id]: true };
		try {
			const text = stagedText(await stagePacks(API, id));
			stageNote = { id, ok: true, text };
			toast.success(text);
			refresh();
			if (selectedId === id) void loadDetail(id, true);
		} catch (err) {
			const text = errorText(err, `Job ${id} could not be staged.`);
			stageNote = { id, ok: false, text };
			toast.error(text);
			// 409 / 404: the row is not what this screen last read.
			if (err instanceof WorkError && (err.status === 409 || err.status === 404)) refresh();
		} finally {
			const next = { ...staging };
			delete next[id];
			staging = next;
		}
	}

	async function loadDetail(id: number, quiet = false) {
		if (!quiet) {
			detailState = 'loading';
			detailError = null;
		}
		try {
			const next = await fetchWorkItem(API, id);
			if (selectedId !== id) return;
			detail = next;
			detailState = 'ready';
			detailError = null;
		} catch (err) {
			if (selectedId !== id) return;
			detailError = errorText(err, `Job ${id} failed to load.`);
			if (!quiet || !detail) detailState = 'failed';
		}
	}

	// The open sheet re-reads only when its row moved in the list.
	function syncDetail(items: WorkItem[]) {
		if (selectedId === null || !sheetOpen || !detail) return;
		const row = items.find((i) => i.id === selectedId);
		const d = detail.item;
		if (!row) return;
		if (
			row.updated_at !== d.updated_at ||
			row.status !== d.status ||
			row.attempts !== d.attempts ||
			row.lease_expires_at !== d.lease_expires_at ||
			row.staged !== d.staged
		) {
			void loadDetail(row.id, true);
		}
	}

	function openDetail(id: number, view = false) {
		if (selectedId !== id) {
			selectedId = id;
			detail = null;
			viewer = { state: 'idle', view: null, error: null };
			stageNote = null;
			void loadDetail(id);
		}
		sheetOpen = true;
		syncUrl();
		if (view) void viewResult(id);
	}

	function closed() {
		selectedId = null;
		detail = null;
		viewer = { state: 'idle', view: null, error: null };
		stageNote = null;
		syncUrl();
	}

	// JSON results only: anything else shows its size and digest, and
	// Download has the bytes.
	async function viewResult(id: number) {
		const row = (detail?.item.id === id ? detail.item : null) ?? list?.items.find((i) => i.id === id) ?? null;
		if (!row?.result || !isJsonType(row.result.content_type)) return;
		if (viewer.state === 'loading' || viewer.state === 'ready') return;
		viewer = { state: 'loading', view: null, error: null };
		try {
			const text = await fetchResultText(API, id);
			if (selectedId !== id) return;
			viewer = { state: 'ready', view: resultView(text), error: null };
		} catch (err) {
			if (selectedId !== id) return;
			viewer = { state: 'failed', view: null, error: errorText(err, 'The result failed to load.') };
		}
	}

	function setFilter(value: string) {
		filter = isStatus(value) ? value : 'all';
		syncUrl();
		refresh();
	}

	function syncUrl() {
		const params = new URLSearchParams(location.search);
		if (filter !== 'all') params.set('status', filter);
		else params.delete('status');
		if (selectedId !== null) params.set('job', String(selectedId));
		else params.delete('job');
		const query = params.toString();
		history.replaceState(history.state, '', query ? `?${query}` : location.pathname);
	}

	function focusSheet(e: Event) {
		// Land on the sheet itself, never on Retry; the next Tab shows
		// a ring as usual.
		e.preventDefault();
		sheetEl?.focus({ preventScroll: true, focusVisible: false } as FocusOptions);
	}

	onMount(() => {
		// Deep link: ?status=failed&job=7 (as /staff-orders).
		const params = new URLSearchParams(location.search);
		const status = params.get('status');
		if (isStatus(status)) filter = status;
		const job = Number(params.get('job'));
		void load().then(() => {
			if (Number.isInteger(job) && job > 0) openDetail(job);
		});
		poller = poll(() => load(true));
		const clock = setInterval(() => (nowMs = Date.now()), 5000);
		return () => {
			poller?.stop();
			poller = null;
			clearInterval(clock);
		};
	});

	// Full class strings per tone (the tailwind checker reads literals).
	const TONE = {
		done: 'border-verified bg-verified-bg text-verified',
		active: 'border-line bg-surface-2 text-accent-strong',
		error: 'border-error bg-error-bg text-error',
		muted: 'border-line bg-surface text-ink-muted'
	} as const;
	// Full width on phones, a roomy reading column from sm up.
	const SHEET = 'gap-0 overflow-y-auto p-0 outline-none data-[side=right]:w-full data-[side=right]:sm:max-w-2xl';
	const PRE = 'm-0 max-h-80 overflow-auto rounded-md border border-line bg-surface p-3 font-mono text-xs leading-relaxed whitespace-pre';
</script>

<svelte:head>
	<title>Work queue — Admin — Apostille Handbook</title>
</svelte:head>

{#snippet badge(status: string)}
	<span
		class={`inline-flex h-6 items-center gap-1.5 rounded-full border px-2 text-xs font-medium whitespace-nowrap transition-colors duration-200 ${TONE[statusTone(status)]}`}
		data-status={status}
	>
		<span aria-hidden="true" class={`size-1.5 rounded-full bg-current ${status === 'leased' ? 'motion-safe:animate-pulse' : ''}`}></span>
		{statusLabel(status)}
	</span>
{/snippet}

{#snippet stagedBadge(id: number)}
	<span
		class="inline-flex h-6 items-center gap-1 rounded-full border border-line bg-surface-2 px-2 text-xs font-medium whitespace-nowrap text-accent-strong"
		title={`Staged in packs-inbox/${id}`}
	>
		<PackageCheckIcon class="size-3.5" aria-hidden="true" />
		Staged
	</span>
{/snippet}

<div class="mt-6 flex flex-col gap-6 pb-12">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div class="flex min-w-0 flex-col gap-1">
			<Heading level={1} eyebrow="Content">Work queue</Heading>
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Research, scraping and eval jobs for plant workers, which dial out to this host and check jobs
				out one at a time. The list re-reads every 5 seconds while this tab is visible.
			</p>
		</div>
		<div class="flex flex-wrap items-center gap-2">
			{#if lastRead !== null}
				<span class="text-xs text-ink-muted">
					Refreshed <time datetime={new Date(lastRead).toISOString()}>{ago(Math.floor(lastRead / 1000), Math.floor(nowMs / 1000))}</time>
				</span>
			{/if}
			<Button variant="outline" size="lg" onclick={refresh} disabled={loadState === 'loading'}>
				<RefreshCwIcon class={refreshing ? 'motion-safe:animate-spin' : ''} aria-hidden="true" />
				Refresh
			</Button>
		</div>
	</header>

	<p class="sr-only" aria-live="polite">{announce}</p>

	{#if loadState === 'loading'}
		<div class="flex flex-col gap-6" role="status" aria-label="Loading the work queue">
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
				<Skeleton class="h-28 w-full rounded-md" />
				<Skeleton class="h-9 w-32 rounded-md" />
			</div>
		</div>
	{:else if loadState === 'failed'}
		<Notice variant="warning" heading="The work queue is unavailable.">
			<p class="m-0">{loadError}</p>
			<p class="mt-3 mb-0"><Button onclick={() => load()}>Try again</Button></p>
		</Notice>
	{:else if list}
		<section aria-label="Counts">
			<dl class="m-0 grid grid-cols-2 gap-3 md:grid-cols-4">
				{#each STATUSES as s (s)}
					<div class="flex flex-col gap-1 rounded-lg border border-line bg-card p-4 shadow-sm">
						<dt class="text-xs font-medium text-ink-muted">{statusLabel(s)}</dt>
						<dd class={`m-0 text-h3 font-semibold tabular-nums ${s === 'failed' && list.counts.failed > 0 ? 'text-error' : ''}`}>
							{list.counts[s]}
						</dd>
					</div>
				{/each}
			</dl>
		</section>

		<div class="grid gap-6 lg:grid-cols-5">
			<section
				aria-labelledby="enqueue-title"
				class="flex min-w-0 flex-col gap-4 rounded-lg border border-line bg-card p-4 shadow-sm md:p-6 lg:col-span-3"
			>
				<div class="flex flex-col gap-1">
					<h2 id="enqueue-title" class="m-0 flex items-center gap-2 text-h3 font-semibold">
						<SendIcon class="size-5 text-accent-strong" aria-hidden="true" /> Enqueue a job
					</h2>
					<p class="m-0 text-sm text-ink-muted">
						The payload starts as the kind's example. The next worker that announces the kind checks it
						out; a failed attempt goes back in the queue until max attempts.
					</p>
				</div>
				{#if kinds.length === 0}
					<Notice variant="info" heading="No job kinds listed.">
						<p class="m-0 text-sm">The server names no job kinds, so there is nothing to enqueue yet.</p>
					</Notice>
				{:else}
					<form
						class="flex flex-col gap-4"
						novalidate
						onsubmit={(event) => {
							event.preventDefault();
							void enqueue();
						}}
					>
						<div class="grid gap-3 sm:grid-cols-3">
							<div class="flex min-w-0 flex-col gap-1 sm:col-span-2">
								<span id="work-kind-label" class="text-xs font-medium text-ink-muted">Kind</span>
								<Select.Root type="single" value={kind} onValueChange={pickKind}>
									<Select.Trigger
										id="work-kind"
										class="h-10 w-full font-mono"
										aria-labelledby="work-kind-label work-kind"
										aria-describedby="work-kind-about"
									>
										{kind || 'Choose a kind'}
									</Select.Trigger>
									<Select.Content>
										{#each kinds as k (k.kind)}
											<Select.Item value={k.kind} label={k.kind}>
												<span class="font-mono">{k.kind}</span>
												{#if !capable.has(k.kind)}<span class="text-xs text-ink-muted">no worker</span>{/if}
											</Select.Item>
										{/each}
									</Select.Content>
								</Select.Root>
							</div>
							<div class="flex min-w-0 flex-col gap-1">
								<label for="work-attempts" class="text-xs font-medium text-ink-muted">Max attempts</label>
								<Input
									id="work-attempts"
									type="number"
									class="h-10"
									min="1"
									max={MAX_ATTEMPTS}
									step="1"
									inputmode="numeric"
									bind:value={maxAttempts}
									aria-invalid={formError?.field === 'max_attempts' || undefined}
									aria-describedby={formError?.field === 'max_attempts' ? 'work-attempts-hint work-form-error' : 'work-attempts-hint'}
								/>
								<span id="work-attempts-hint" class="text-xs text-ink-muted">1 to {MAX_ATTEMPTS}; blank means {DEFAULT_ATTEMPTS}.</span>
							</div>
						</div>
						<p id="work-kind-about" class="m-0 text-sm text-ink-muted">
							{#if chosenKind}
								{chosenKind.about || 'No description from the server.'}
								{#if !capable.has(chosenKind.kind)}
									<span class="block text-changed">No connected worker takes this kind; the job waits until one does.</span>
								{/if}
							{/if}
						</p>
						<div class="flex flex-col gap-1">
							<div class="flex flex-wrap items-end justify-between gap-2">
								<label for="work-payload" class="text-xs font-medium text-ink-muted">Payload (JSON object)</label>
								{#if chosenKind}
									<Button variant="ghost" size="sm" onclick={() => pickKind(kind)}>Use the example</Button>
								{/if}
							</div>
							<Textarea
								id="work-payload"
								class="max-h-96 min-h-40 font-mono text-xs"
								bind:value={payloadText}
								spellcheck="false"
								autocomplete="off"
								autocapitalize="off"
								aria-invalid={(formError?.field === 'payload' && !payloadCheck.ok) || undefined}
								aria-describedby="work-payload-hint"
							/>
							<span id="work-payload-hint" class={`text-xs ${payloadCheck.ok ? 'text-ink-muted' : 'text-changed'}`}>
								{payloadCheck.ok ? `JSON object, ${fmtBytes(payloadBytes)} of 64 KiB.` : payloadCheck.error}
							</span>
						</div>
						<!-- A payload error stays in the hint above (the textarea's description, focused on submit). -->
						{#if formError && formError.field !== 'payload'}
							<p id="work-form-error" class="m-0 rounded-md border border-error bg-error-bg px-3 py-2 text-sm" role="alert">
								{formError.text}
							</p>
						{/if}
						<div>
							<Button type="submit" size="lg" disabled={enqueuing || !kind}>
								{#if enqueuing}
									<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" /> Enqueuing…
								{:else}
									<SendIcon aria-hidden="true" /> Enqueue job
								{/if}
							</Button>
						</div>
					</form>
				{/if}
			</section>

			<section
				aria-labelledby="workers-title"
				class="flex min-w-0 flex-col gap-4 self-start rounded-lg border border-line bg-card p-4 shadow-sm md:p-6 lg:col-span-2"
			>
				<div class="flex flex-col gap-1">
					<h2 id="workers-title" class="m-0 flex items-center gap-2 text-h3 font-semibold">
						<PlugZapIcon class="size-5 text-accent-strong" aria-hidden="true" /> Connected workers
					</h2>
					<p class="m-0 text-sm text-ink-muted">Sockets open right now, with the kinds each one announced.</p>
				</div>
				{#if list.workers.length === 0}
					<div class="flex flex-col items-center gap-2 rounded-md border border-dashed border-line px-4 py-8 text-center">
						<UnplugIcon class="size-7 text-ink-muted" aria-hidden="true" />
						<p class="m-0 text-sm text-ink-muted">
							No worker is connected. Pending jobs wait until plant runs <code class="font-mono text-xs">cargo xtask worker</code>.
						</p>
					</div>
				{:else}
					<ul class="m-0 flex list-none flex-col gap-3 p-0">
						{#each list.workers as w (w.name)}
							<li class="flex flex-col gap-2 rounded-md border border-line p-3">
								<div class="flex flex-wrap items-center gap-x-3 gap-y-1">
									<span class="font-mono text-sm font-medium">{w.name}</span>
									{#if w.version}<span class="font-mono text-xs text-ink-muted">v{w.version}</span>{/if}
									<span class="ml-auto text-sm">
										{#if w.job_id !== null}
											<button
												type="button"
												class="rounded-xs font-mono text-accent-strong outline-none hover:underline focus-visible:ring-2 focus-visible:ring-focus"
												aria-haspopup="dialog"
												onclick={() => w.job_id !== null && openDetail(w.job_id)}
											>
												On job {w.job_id}
											</button>
										{:else}
											<span class="text-ink-muted">Idle</span>
										{/if}
									</span>
								</div>
								<p class="m-0 text-xs text-ink-muted">
									{#if w.last_seen_at !== null}
										Last seen <time datetime={iso(w.last_seen_at)} title={dateTime(w.last_seen_at)}>{ago(w.last_seen_at, now)}</time>
									{/if}
									{#if w.connected_at !== null}
										· connected <time datetime={iso(w.connected_at)} title={dateTime(w.connected_at)}>{ago(w.connected_at, now)}</time>
									{/if}
								</p>
								{#if w.capabilities.length > 0}
									<ul class="m-0 flex list-none flex-wrap gap-1.5 p-0" aria-label={`Kinds ${w.name} takes`}>
										{#each w.capabilities as c (c)}
											<li class="rounded-full bg-surface-2 px-2 py-0.5 font-mono text-xs text-ink-muted">{c}</li>
										{/each}
									</ul>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</section>
		</div>

		<section aria-labelledby="jobs-title" class="flex flex-col gap-3">
			<div class="flex flex-wrap items-end justify-between gap-3">
				<h2 id="jobs-title" class="m-0 text-h3 font-semibold">Jobs</h2>
				<div class="flex w-full min-w-0 flex-col gap-1 sm:w-56">
					<span id="work-filter-label" class="text-xs font-medium text-ink-muted">Status</span>
					<Select.Root type="single" value={filter} onValueChange={setFilter}>
						<Select.Trigger id="work-filter" class="h-9 w-full" aria-labelledby="work-filter-label work-filter">
							{filterLabel}
						</Select.Trigger>
						<Select.Content>
							{#each FILTERS as f (f.value)}
								<Select.Item value={f.value} label={f.label}>{f.label}</Select.Item>
							{/each}
						</Select.Content>
					</Select.Root>
				</div>
			</div>

			{#if list.items.length === 0}
				<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line bg-card px-6 py-10 text-center">
					<InboxIcon class="size-8 text-ink-muted" aria-hidden="true" />
					{#if filter === 'all'}
						<p class="m-0 text-sm text-ink-muted">No jobs yet. Enqueue the first one above.</p>
					{:else}
						<p class="m-0 text-sm text-ink-muted">No {statusLabel(filter).toLowerCase()} jobs.</p>
						<Button variant="outline" onclick={() => setFilter('all')}>Show all statuses</Button>
					{/if}
				</div>
			{:else}
				<div class="overflow-hidden rounded-lg border border-line bg-card shadow-sm">
					<Table.Root aria-label="Jobs">
						<Table.Header class="bg-surface">
							<tr class="border-b border-line">
								<Table.Head class="px-3 text-xs text-ink-muted sm:px-4">Job</Table.Head>
								<Table.Head class="text-xs text-ink-muted">Status</Table.Head>
								<Table.Head class="hidden text-right text-xs text-ink-muted sm:table-cell">Attempts</Table.Head>
								<Table.Head class="hidden text-xs text-ink-muted md:table-cell">Lease</Table.Head>
								<Table.Head class="hidden text-xs text-ink-muted lg:table-cell">Updated</Table.Head>
								<Table.Head class="hidden text-xs text-ink-muted xl:table-cell">Error</Table.Head>
								<Table.Head class="pr-3 text-right text-xs text-ink-muted sm:pr-4">Actions</Table.Head>
							</tr>
						</Table.Header>
						<Table.Body>
							{#each list.items as item (item.id)}
								<tr class={`border-b border-line last:border-b-0 ${selectedId === item.id ? 'bg-muted' : ''}`}>
									<th scope="row" class="px-3 py-2.5 text-left font-normal sm:px-4">
										<button
											type="button"
											class="w-fit rounded-xs text-left font-mono text-sm font-medium text-ink outline-none hover:underline focus-visible:ring-2 focus-visible:ring-focus"
											aria-haspopup="dialog"
											aria-label={`Job ${item.id}: details`}
											onclick={() => openDetail(item.id)}
										>
											#{item.id}
										</button>
										<span class="block font-mono text-xs text-ink-muted">{item.kind}</span>
									</th>
									<Table.Cell>
										<div class="flex flex-wrap items-center gap-1.5">
											{@render badge(item.status)}
											{#if item.staged}{@render stagedBadge(item.id)}{/if}
										</div>
									</Table.Cell>
									<Table.Cell class="hidden text-right font-mono text-sm tabular-nums sm:table-cell">
										{item.attempts} / {item.max_attempts}
									</Table.Cell>
									<Table.Cell class="hidden text-xs md:table-cell">
										{#if item.lease_owner}
											<span class="block font-mono">{item.lease_owner}</span>
										{/if}
										{#if item.lease_expires_at !== null}
											<time class="block text-ink-muted" datetime={iso(item.lease_expires_at)} title={dateTime(item.lease_expires_at)}
												>{leaseText(item.lease_expires_at, now)}</time
											>
										{:else if item.status === 'pending'}
											<span class={`block ${capable.has(item.kind) ? 'text-ink-muted' : 'text-changed'}`}>
												{capable.has(item.kind) ? 'Waiting for a worker' : 'No worker for this kind'}
											</span>
										{:else}
											<span class="text-ink-muted">—</span>
										{/if}
									</Table.Cell>
									<Table.Cell class="hidden text-xs text-ink-muted lg:table-cell">
										<time class="block" datetime={iso(item.updated_at)} title={dateTime(item.updated_at)}>{ago(item.updated_at, now)}</time>
										<span class="block">
											created <time datetime={iso(item.created_at)} title={dateTime(item.created_at)}>{ago(item.created_at, now)}</time>
										</span>
									</Table.Cell>
									<Table.Cell class="hidden max-w-xs xl:table-cell">
										{#if item.error}
											<span class="block truncate text-xs text-error" title={item.error}>{item.error}</span>
										{:else}
											<span class="text-xs text-ink-muted">—</span>
										{/if}
									</Table.Cell>
									<!-- Icons only on phones; each button keeps its full name. -->
									<Table.Cell class="pr-3 sm:pr-4">
										<div class="flex flex-wrap justify-end gap-1.5">
											{#if item.result}
												<Button
													variant="outline"
													size="sm"
													aria-haspopup="dialog"
													aria-label={`View the result of job ${item.id}`}
													onclick={() => openDetail(item.id, true)}
												>
													<EyeIcon aria-hidden="true" /><span class="hidden sm:inline">View</span>
												</Button>
												<Button
													variant="outline"
													size="sm"
													href={resultHref(API, item.id)}
													download
													aria-label={`Download the result of job ${item.id}`}
												>
													<DownloadIcon aria-hidden="true" /><span class="hidden sm:inline">Download</span>
												</Button>
											{/if}
											{#if canStagePacks(item)}
												<Button
													variant="outline"
													size="sm"
													disabled={!!staging[item.id]}
													aria-label={`${item.staged ? 'Stage again' : 'Stage as packs'}: job ${item.id}`}
													onclick={() => stage(item.id)}
												>
													{#if staging[item.id]}<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<PackagePlusIcon
															aria-hidden="true"
														/>{/if}
													<span class="hidden sm:inline">{item.staged ? 'Stage again' : 'Stage as packs'}</span>
												</Button>
											{/if}
											{#if item.status === 'failed'}
												<Button
													variant="outline"
													size="sm"
													disabled={!!retrying[item.id]}
													aria-label={`Retry job ${item.id}`}
													onclick={() => retry(item.id)}
												>
													{#if retrying[item.id]}<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<RotateCcwIcon
															aria-hidden="true"
														/>{/if}
													<span class="hidden sm:inline">Retry</span>
												</Button>
											{/if}
										</div>
									</Table.Cell>
								</tr>
							{/each}
						</Table.Body>
					</Table.Root>
				</div>
			{/if}
			<p class="m-0 text-xs text-ink-muted">
				{#if list.items.length > 0}
					{list.items.length} job{list.items.length === 1 ? '' : 's'} shown{list.items.length >= LIST_LIMIT
						? `, the newest ${LIST_LIMIT}`
						: ''}. Select a job number for its payload, summary and audit trail.
				{/if}
				{#if pollError}<span class="text-changed" role="status"> Last refresh failed: {pollError}</span>{/if}
			</p>
		</section>
	{/if}
</div>

<Sheet.Root bind:open={sheetOpen} onOpenChangeComplete={(o) => !o && closed()}>
	<Sheet.Content bind:ref={sheetEl} tabindex={-1} onOpenAutoFocus={focusSheet} class={SHEET}>
		<Sheet.Header class="gap-3 border-b border-line p-6 pr-14">
			<p class="m-0 font-mono text-xs text-ink-muted">{shown?.kind ?? 'Work item'}</p>
			<Sheet.Title class="font-mono text-h3 font-semibold tracking-tight">Job {selectedId ?? ''}</Sheet.Title>
			<Sheet.Description class="m-0 text-sm text-ink-muted">
				{#if shown}
					Attempt {shown.attempts} of {shown.max_attempts}{shown.created_by ? ` · enqueued by ${shown.created_by}` : ''}
				{:else}
					Loading this job.
				{/if}
			</Sheet.Description>
			{#if shown}
				<div class="flex flex-wrap items-center gap-2 pt-1" aria-live="polite">
					{@render badge(shown.status)}
					{#if shown.staged}{@render stagedBadge(shown.id)}{/if}
					{#if shown.lease_owner}
						<span class="text-xs text-ink-muted">
							<span class="font-mono">{shown.lease_owner}</span>{#if shown.lease_expires_at !== null}, {leaseText(shown.lease_expires_at, now)}{/if}
						</span>
					{/if}
				</div>
			{/if}
		</Sheet.Header>

		{#if detailState === 'loading' && !detail}
			<div class="flex flex-col gap-6 p-6" role="status" aria-label="Loading the job">
				<Skeleton class="h-10 w-2/3 rounded-md" />
				<Skeleton class="h-28 rounded-md" />
				<Skeleton class="h-40 rounded-md" />
			</div>
		{:else if detailState === 'failed' && !detail}
			<div class="p-6">
				<Notice variant="error" heading="This job could not load.">
					<p class="m-0">{detailError}</p>
					<p class="mt-3 mb-0">
						<Button onclick={() => selectedId !== null && loadDetail(selectedId)}>Try again</Button>
					</p>
				</Notice>
			</div>
		{:else if detail}
			{@const item = detail.item}
			{#if item.status === 'failed' || item.error}
				<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-error">
					<h3 id="sheet-error" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Error</h3>
					{#if item.error}
						<p class="m-0 rounded-md border border-error bg-error-bg px-3 py-2 font-mono text-xs wrap-anywhere">{item.error}</p>
					{/if}
					{#if item.status === 'failed'}
						<div>
							<Button size="lg" disabled={!!retrying[item.id]} onclick={() => retry(item.id)}>
								{#if retrying[item.id]}<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<RotateCcwIcon
										aria-hidden="true"
									/>{/if}
								Retry with attempts reset
							</Button>
						</div>
					{/if}
				</section>
			{/if}

			{#if item.result}
				{@const result = item.result}
				<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-result">
					<h3 id="sheet-result" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Result</h3>
					<dl class="m-0 grid grid-cols-3 gap-x-4 gap-y-1.5 text-sm">
						<dt class="text-ink-muted">Type</dt>
						<dd class="col-span-2 m-0 font-mono text-xs">{result.content_type}</dd>
						<dt class="text-ink-muted">Size</dt>
						<dd class="col-span-2 m-0 font-mono text-xs">{fmtBytes(result.bytes)} ({result.bytes} bytes)</dd>
						<dt class="text-ink-muted">SHA-256</dt>
						<dd class="col-span-2 m-0 font-mono text-xs break-all">{result.sha256}</dd>
					</dl>
					<div class="flex flex-wrap items-center gap-2">
						{#if isJsonType(result.content_type) && viewer.state !== 'ready'}
							<Button variant="outline" disabled={viewer.state === 'loading'} onclick={() => viewResult(item.id)}>
								{#if viewer.state === 'loading'}<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<EyeIcon
										aria-hidden="true"
									/>{/if}
								View here
							</Button>
						{/if}
						<Button variant="outline" href={resultHref(API, item.id)} download>
							<DownloadIcon aria-hidden="true" /> Download
						</Button>
						{#if canStagePacks(item)}
							<Button variant="outline" disabled={!!staging[item.id]} onclick={() => stage(item.id)}>
								{#if staging[item.id]}<LoaderIcon class="motion-safe:animate-spin" aria-hidden="true" />{:else}<PackagePlusIcon
										aria-hidden="true"
									/>{/if}
								{item.staged ? 'Stage again' : 'Stage as packs'}
							</Button>
						{/if}
					</div>
					{#if canStagePacks(item) || stageNote?.id === item.id}
						<div aria-live="polite">
							{#if stageNote?.id === item.id}
								<p
									class={`m-0 rounded-md border px-3 py-2 text-sm ${stageNote.ok ? 'border-verified bg-verified-bg' : 'border-error bg-error-bg'}`}
								>
									{stageNote.text}
								</p>
							{:else if item.staged}
								<p class="m-0 text-sm text-ink-muted">
									Already staged in <code class="font-mono text-xs">packs-inbox/{item.id}</code>; staging again
									overwrites it.
								</p>
							{:else}
								<p class="m-0 text-sm text-ink-muted">
									Staging copies these packs into <code class="font-mono text-xs">packs-inbox/{item.id}</code> for the
									Packs page.
								</p>
							{/if}
						</div>
					{/if}
					<div class="flex flex-col gap-2" aria-live="polite">
						{#if !isJsonType(result.content_type)}
							<p class="m-0 text-sm text-ink-muted">Not JSON, so it does not show here: download it to read it.</p>
						{:else if viewer.state === 'failed'}
							<p class="m-0 text-sm text-error" role="alert">{viewer.error}</p>
						{:else if viewer.state === 'ready' && viewer.view}
							{#if viewer.view.truncated}
								<p class="m-0 text-sm text-changed">
									Shown up to the first {fmtBytes(viewer.view.text.length)} of {fmtBytes(viewer.view.chars)} of text; download
									it for the rest.
								</p>
							{/if}
							{#if !viewer.view.parsed}
								<p class="m-0 text-sm text-ink-muted">The bytes do not parse as JSON; they show as sent.</p>
							{/if}
							<!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable regions take focus so the keyboard can scroll them) -->
							<pre class={PRE} role="region" tabindex="0" aria-label={`Result of job ${item.id}`}>{viewer.view.text}</pre>
						{/if}
					</div>
				</section>
			{/if}

			<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-facts">
				<h3 id="sheet-facts" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Timing</h3>
				<dl class="m-0 grid grid-cols-3 gap-x-4 gap-y-1.5 text-sm">
					<dt class="text-ink-muted">Created</dt>
					<dd class="col-span-2 m-0">
						<time datetime={iso(item.created_at)}>{dateTime(item.created_at)}</time>
						<span class="text-ink-muted">· {ago(item.created_at, now)}</span>
					</dd>
					<dt class="text-ink-muted">Updated</dt>
					<dd class="col-span-2 m-0">
						<time datetime={iso(item.updated_at)}>{dateTime(item.updated_at)}</time>
						<span class="text-ink-muted">· {ago(item.updated_at, now)}</span>
					</dd>
					<dt class="text-ink-muted">Lease</dt>
					<dd class="col-span-2 m-0">
						{#if item.lease_owner || item.lease_expires_at !== null}
							<span class="font-mono">{item.lease_owner ?? '?'}</span>{#if item.lease_expires_at !== null}, {leaseText(item.lease_expires_at, now)}{/if}
						{:else}
							<span class="text-ink-muted">None</span>
						{/if}
					</dd>
				</dl>
			</section>

			<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-payload">
				<h3 id="sheet-payload" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Payload</h3>
				<!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable regions take focus so the keyboard can scroll them) -->
				<pre class={PRE} role="region" tabindex="0" aria-labelledby="sheet-payload">{pretty(item.payload)}</pre>
			</section>

			<section class="flex flex-col gap-3 border-b border-line px-6 py-5" aria-labelledby="sheet-summary">
				<h3 id="sheet-summary" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Summary</h3>
				{#if item.summary === null}
					<p class="m-0 text-sm text-ink-muted">No summary: the worker reports one when the job completes.</p>
				{:else}
					<!-- svelte-ignore a11y_no_noninteractive_tabindex (scrollable regions take focus so the keyboard can scroll them) -->
					<pre class={PRE} role="region" tabindex="0" aria-labelledby="sheet-summary">{pretty(item.summary)}</pre>
				{/if}
			</section>

			<section class="flex flex-col gap-3 px-6 py-5" aria-labelledby="sheet-audit">
				<h3 id="sheet-audit" class="m-0 text-xs font-semibold tracking-wide text-ink-muted uppercase">Audit trail</h3>
				{#if detail.audit.length === 0}
					<p class="m-0 text-sm text-ink-muted">No audit rows for this job.</p>
				{:else}
					<ol class="m-0 flex list-none flex-col divide-y divide-line rounded-md border border-line p-0">
						{#each detail.audit as a, i (`${a.at}|${a.action}|${i}`)}
							<li class="flex flex-col gap-0.5 px-3 py-2">
								<p class="m-0 flex flex-wrap items-baseline gap-x-2 text-sm">
									<span class="font-medium">{auditLabel(a.action)}</span>
									<span class="font-mono text-xs text-ink-muted">{a.actor}</span>
									<time class="ml-auto text-xs text-ink-muted" datetime={iso(a.at)}>{dateTime(a.at)}</time>
								</p>
								{#if a.reason}<p class="m-0 font-mono text-xs text-ink-muted wrap-anywhere">{a.reason}</p>{/if}
							</li>
						{/each}
					</ol>
				{/if}
			</section>
		{/if}
	</Sheet.Content>
</Sheet.Root>
