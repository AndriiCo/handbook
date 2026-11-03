<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { flip } from 'svelte/animate';
	import { fade, fly } from 'svelte/transition';
	import { toast } from 'svelte-sonner';
	import { resolve } from '$app/paths';
	import * as Card from '#lib/components/ui/card/index.js';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import * as Popover from '#lib/components/ui/popover/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { acceptFact, bulkBody, bulkDecide, rejectBody, rejectFact, type QueueItem } from './review-client.js';
	import {
		blockedTag,
		groupByAuthority,
		isVerified,
		plural,
		riskCls,
		riskOf,
		splitBlocked,
		WONDER
	} from './review-signals.js';
	import { fetchStaffAuthorities } from './authorities.js';
	import { ago } from './format.js';
	import type { FactCheck } from './proposals.js';
	import ArrowUpRightIcon from '@lucide/svelte/icons/arrow-up-right';
	import CheckIcon from '@lucide/svelte/icons/check';
	import CheckCheckIcon from '@lucide/svelte/icons/check-check';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import QuoteIcon from '@lucide/svelte/icons/quote';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import SparklesIcon from '@lucide/svelte/icons/sparkles';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import XIcon from '@lucide/svelte/icons/x';

	// Proposed facts grouped per authority, decided in bulk (POST
	// /facts/bulk: one all-or-nothing decision per authority, one
	// `authority:{slug}` event per accepted batch). Rows start checked
	// only when verified, so staff can hold some back (the operator
	// keeps two proposed per slug for the video). Every decision is
	// optimistic and rolled back in full when the server refuses it.
	let {
		items,
		checks = {},
		decided = $bindable({}),
		reduceMotion = false,
		focusId = null,
		onLoadAll,
		onRefresh,
		onDecided,
		onOpenQueue
	}: {
		items: QueueItem[];
		checks?: Record<number, FactCheck>;
		decided?: Record<number, string>;
		reduceMotion?: boolean;
		/** Row to focus on mount (`?fact=<id>` deep links). */
		focusId?: number | null;
		/** Load every proposed fact of one authority (the view shows a page). */
		onLoadAll?: (slug: string) => Promise<void>;
		/** Re-read after the server said a fact was decided elsewhere. */
		onRefresh?: (slug: string) => void;
		onDecided?: (status: 'accepted' | 'rejected', ids: number[]) => void;
		/** Open the authority in the queue table (else a link to /review). */
		onOpenQueue?: (slug: string) => void;
	} = $props();

	const api = '/apostille/api/v1';
	// Quote chip (one string, like the TRIGGER consts: state variants
	// resolve to `[data-state=open]` selectors in the built CSS).
	const CHIP =
		'inline-flex h-6 cursor-pointer items-center gap-1 rounded-full border border-line px-2 font-mono text-xs text-ink-muted transition-colors hover:border-focus hover:text-ink data-[state=open]:border-focus data-[state=open]:text-ink';

	interface Note {
		text: string;
		retry: { label: string; run: () => void } | null;
	}

	let names = $state<Record<string, string>>({});
	let totals = $state<Record<string, number | null>>({});
	// Explicit checkbox choices; unset rows fall back to "verified".
	let picked = $state<Record<number, boolean>>({});
	let tags = $state<Record<number, string>>({});
	let busy = $state<Record<string, string>>({});
	let rowBusy = $state<Record<number, boolean>>({});
	let notes = $state<Record<string, Note>>({});
	let loadingAll = $state<Record<string, boolean>>({});
	let focused = $state<number | null>(null);
	let quoteFor = $state<number | null>(null);

	type Scope = 'unverified' | 'picked' | 'open';
	let rejectOpen = $state(false);
	let rejectSlug = $state('');
	let rejectRow = $state<number | null>(null);
	let rejectScope = $state<Scope>('unverified');
	let reason = $state('');
	let suppressOn = $state(false);
	let supScope = $state<'authority' | 'global'>('authority');
	let lockTopic = $state(true);
	let rejectError = $state<string | null>(null);

	const groups = $derived(groupByAuthority(items, decided, checks, names));
	const flat = $derived(groups.flatMap((g) => g.items));
	const openRows = $derived(flat.filter((i) => !decided[i.id]));
	const nowSec = $derived(Math.max(Date.now() / 1000, ...items.map((i) => i.created_at)));
	const verifiedTotal = $derived(groups.reduce((n, g) => n + g.verified, 0));

	const verified = (item: QueueItem) => isVerified(item, checks[item.id]);
	const isPicked = (item: QueueItem) => !decided[item.id] && (picked[item.id] ?? verified(item));
	const groupOf = (slug: string) => groups.find((g) => g.slug === slug) ?? null;
	const acceptIds = (slug: string) =>
		(groupOf(slug)?.items ?? []).filter((i) => isPicked(i) && verified(i)).map((i) => i.id);

	function scopeIds(slug: string, scope: Scope): number[] {
		const open = (groupOf(slug)?.items ?? []).filter((i) => !decided[i.id]);
		if (scope === 'picked') return open.filter(isPicked).map((i) => i.id);
		if (scope === 'unverified') return open.filter((i) => !verified(i)).map((i) => i.id);
		return open.map((i) => i.id);
	}

	const rejectTargets = $derived(
		rejectRow !== null ? [rejectRow] : rejectSlug ? scopeIds(rejectSlug, rejectScope).slice(0, 100) : []
	);
	const reasonOk = $derived(reason.trim().length > 0 && reason.trim().length <= 1024);

	function acceptLabel(slug: string, v: number): string {
		const k = acceptIds(slug).length;
		return k === v ? `Accept all verified (${v})` : `Accept ${k} of ${v} verified`;
	}

	function where(item: QueueItem): string {
		const cite = item.citations[0];
		if (!cite) return '';
		if (cite.capture_id > 0) return `capture ${cite.capture_id}`;
		try {
			return new URL(cite.source_url ?? '').hostname.replace(/^www\./, '');
		} catch {
			return 'source';
		}
	}

	function caption(item: QueueItem): string {
		const cite = item.citations[0];
		if (!cite) return '';
		if (cite.capture_id > 0) return `capture ${cite.capture_id} · bytes ${cite.start_byte}–${cite.end_byte}`;
		const day = (cite.captured_at ?? '').split('T')[0];
		return `${where(item)}${day ? ` · captured ${day}` : ''}`;
	}

	async function loadNames(fresh = false) {
		try {
			const rows = await fetchStaffAuthorities(fresh);
			const nextNames: Record<string, string> = {};
			const nextTotals: Record<string, number | null> = {};
			for (const row of rows) {
				nextNames[row.slug] = row.name;
				nextTotals[row.slug] = row.proposed_count;
			}
			names = nextNames;
			totals = nextTotals;
		} catch {
			// The slug stays the label; the groups never depend on it.
		}
	}

	onMount(() => {
		void loadNames();
	});

	// Deep link focus, once per id, as soon as that row is in view.
	let focusedFrom: number | null = null;
	$effect(() => {
		const id = focusId;
		if (id === null || id === focusedFrom || !items.some((i) => i.id === id)) return;
		focusedFrom = id;
		untrack(() => focus(id));
	});

	function focus(id: number | null) {
		focused = id;
		if (id === null) return;
		void tick().then(() => document.getElementById(`group-row-${id}`)?.scrollIntoView({ block: 'nearest' }));
	}

	function focusPast(ids: number[]) {
		if (focused === null || !ids.includes(focused)) return;
		const at = flat.findIndex((i) => i.id === focused);
		const next = flat.slice(at + 1).find((i) => !decided[i.id]) ?? flat.find((i) => !decided[i.id]);
		focus(next?.id ?? null);
	}

	function rollback(ids: number[]) {
		for (const id of ids) delete decided[id];
	}

	// A blocked batch changed nothing: tag each refused row inline,
	// uncheck it, and offer the rest as a fresh batch.
	function blocked(
		slug: string,
		ids: number[],
		results: { id: number; ok: boolean; code?: number; error?: string }[],
		verb: 'accepted' | 'rejected',
		retry: (pass: number[]) => void
	) {
		const { refused, passing } = splitBlocked(results);
		for (const r of refused) {
			tags[r.id] = r.tag;
			picked[r.id] = false;
		}
		const pass = passing.filter((id) => ids.includes(id));
		const text = `Nothing ${verb}: ${refused.length} of ${ids.length} blocked.`;
		const label = `${verb === 'accepted' ? 'Accept' : 'Reject'} the ${pass.length} that pass`;
		notes[slug] = { text, retry: pass.length > 0 ? { label, run: () => retry(pass) } : null };
		toast.error(text, pass.length > 0 ? { action: { label, onClick: () => retry(pass) } } : undefined);
		if (refused.some((r) => r.code === 409)) onRefresh?.(slug);
	}

	async function acceptGroup(slug: string, only?: number[]) {
		if (busy[slug]) return;
		const ids = (only ?? acceptIds(slug)).filter((id) => !decided[id]).slice(0, 100);
		const body = bulkBody('accept', slug, ids, { requireVerified: true });
		if (!body) return;
		delete notes[slug];
		for (const id of ids) {
			delete tags[id];
			decided[id] = 'accepted';
		}
		busy[slug] = `Accepting ${ids.length}…`;
		focusPast(ids);
		try {
			const out = await bulkDecide(api, body);
			if (out.ok) {
				for (const r of out.results) decided[r.id] = r.status ?? 'accepted';
				toast.success(`Accepted ${plural(out.count, 'fact')} — the ${slug} dossier updates live.`);
				onDecided?.('accepted', ids);
			} else {
				rollback(ids);
				blocked(slug, ids, out.results, 'accepted', (pass) => void acceptGroup(slug, pass));
			}
		} catch (err) {
			rollback(ids);
			const message = err instanceof Error ? err.message : 'Bulk accept failed.';
			notes[slug] = { text: `Nothing accepted: ${message}`, retry: null };
			toast.error(`Nothing accepted for ${slug}: ${message}`);
		} finally {
			delete busy[slug];
		}
	}

	async function acceptOne(item: QueueItem) {
		const id = item.id;
		if (decided[id] || rowBusy[id] || busy[item.authority_slug]) return;
		rowBusy[id] = true;
		delete tags[id];
		decided[id] = 'accepted';
		focusPast([id]);
		try {
			const out = await acceptFact(api, id, {});
			decided[id] = out.status;
			toast.success(`Fact #${id} accepted — the ${item.authority_slug} dossier updates live.`);
			onDecided?.('accepted', [id]);
		} catch (err) {
			delete decided[id];
			focus(id);
			const message = err instanceof Error ? err.message : 'Accept failed.';
			const stale = /409|not proposed/.test(message);
			tags[id] = blockedTag(stale ? 409 : 422, message);
			if (stale) onRefresh?.(item.authority_slug);
			else toast.error(`Fact #${id}: ${message}`);
		} finally {
			delete rowBusy[id];
		}
	}

	function openReject(slug: string, row: number | null = null) {
		rejectSlug = slug;
		rejectRow = row;
		const has = (s: Scope) => scopeIds(slug, s).length > 0;
		rejectScope = has('unverified') ? 'unverified' : has('picked') ? 'picked' : 'open';
		reason = '';
		suppressOn = false;
		supScope = 'authority';
		lockTopic = true;
		rejectError = null;
		rejectOpen = true;
	}

	async function rejectGroup(slug: string, ids: number[], text: string, sup: { scope: string; lock_topic: boolean } | undefined) {
		const body = bulkBody('reject', slug, ids, { reason: text, suppress: sup });
		if (!body || busy[slug]) return;
		delete notes[slug];
		for (const id of ids) {
			delete tags[id];
			decided[id] = 'rejected';
		}
		busy[slug] = `Rejecting ${ids.length}…`;
		focusPast(ids);
		try {
			const out = await bulkDecide(api, body);
			if (out.ok) {
				for (const r of out.results) decided[r.id] = r.status ?? 'rejected';
				toast.success(`Rejected ${plural(out.count, 'fact')}${sup ? ' and suppressed future copies' : ''}.`);
				onDecided?.('rejected', ids);
			} else {
				rollback(ids);
				blocked(slug, ids, out.results, 'rejected', (pass) => void rejectGroup(slug, pass, text, sup));
			}
		} catch (err) {
			rollback(ids);
			const message = err instanceof Error ? err.message : 'Bulk reject failed.';
			notes[slug] = { text: `Nothing rejected: ${message}`, retry: null };
			toast.error(`Nothing rejected for ${slug}: ${message}`);
		} finally {
			delete busy[slug];
		}
	}

	async function rejectOne(id: number, text: string, sup: { scope: string; lock_topic: boolean } | undefined) {
		const body = rejectBody(text, sup);
		if (!body || decided[id]) return;
		rowBusy[id] = true;
		delete tags[id];
		decided[id] = 'rejected';
		focusPast([id]);
		try {
			const out = await rejectFact(api, id, body);
			decided[id] = out.status;
			toast.success(`Fact #${id} rejected${sup ? ' and suppressed' : ''}.`);
			onDecided?.('rejected', [id]);
		} catch (err) {
			delete decided[id];
			const message = err instanceof Error ? err.message : 'Reject failed.';
			const stale = /409|not proposed/.test(message);
			tags[id] = blockedTag(stale ? 409 : 422, message);
			const slug = items.find((i) => i.id === id)?.authority_slug ?? '';
			if (stale) onRefresh?.(slug);
			else toast.error(`Fact #${id}: ${message}`);
		} finally {
			delete rowBusy[id];
		}
	}

	function submitReject() {
		if (!reasonOk) {
			rejectError = 'A reason (1–1024 chars) is required to reject.';
			return;
		}
		const ids = [...rejectTargets];
		if (ids.length === 0) return;
		const text = reason.trim();
		const sup = suppressOn ? { scope: supScope, lock_topic: lockTopic } : undefined;
		rejectOpen = false;
		if (rejectRow !== null) void rejectOne(rejectRow, text, sup);
		else void rejectGroup(rejectSlug, ids, text, sup);
	}

	async function loadAll(slug: string) {
		if (!onLoadAll || loadingAll[slug]) return;
		loadingAll[slug] = true;
		try {
			await onLoadAll(slug);
			void loadNames(true);
		} catch (err) {
			toast.error(err instanceof Error ? err.message : `Could not load every proposal for ${slug}.`);
		} finally {
			delete loadingAll[slug];
		}
	}

	// j/k move, x check, a accept, r reject, Shift+A accepts the
	// focused group's checked verified rows. Never while typing.
	function onKey(e: KeyboardEvent) {
		if (e.metaKey || e.ctrlKey || e.altKey || e.repeat) return;
		const t = e.target as HTMLElement | null;
		if (t && (t.closest('input:not([type="checkbox"]), textarea, select, [contenteditable="true"]') || t.isContentEditable))
			return;
		if (document.querySelector('[role="dialog"]')) return;
		const key = e.key.toLowerCase();
		const row = flat.find((i) => i.id === focused) ?? null;
		if (e.shiftKey) {
			if (key !== 'a') return;
			e.preventDefault();
			const slug = row?.authority_slug ?? openRows[0]?.authority_slug;
			if (slug) void acceptGroup(slug);
			return;
		}
		if (!['j', 'k', 'x', 'a', 'r'].includes(key)) return;
		e.preventDefault();
		if (key === 'j' || key === 'k') {
			if (openRows.length === 0) return;
			const at = openRows.findIndex((i) => i.id === focused);
			const step = key === 'j' ? 1 : -1;
			const next = at < 0 ? 0 : Math.min(openRows.length - 1, Math.max(0, at + step));
			focus(openRows[next].id);
			return;
		}
		if (!row || decided[row.id]) return;
		if (key === 'x') picked[row.id] = !isPicked(row);
		else if (key === 'a') void acceptOne(row);
		else openReject(row.authority_slug, row.id);
	}

	function share(n: number, total: number): string {
		return `width: ${total > 0 ? (100 * n) / total : 0}%`;
	}
</script>

<svelte:window onkeydown={onKey} />

{#if items.length === 0}
	<Card.Root class="items-start px-6 py-10">
		<InboxIcon class="size-6 text-ink-muted" aria-hidden="true" />
		<p class="m-0 text-h3 font-semibold">Queue clear</p>
		<p class="m-0 text-sm text-ink-muted">Nothing proposed right now. Draft from a fresh capture to fill it.</p>
		<Button href={resolve('/(admin)/trace')}>Open the trace console</Button>
	</Card.Root>
{:else}
	<p class="m-0 text-xs text-ink-muted">
		{plural(groups.length, 'authority', 'authorities')} · {openRows.length} proposed · {verifiedTotal} verified ·
		checked rows go in the batch
	</p>
	<div class="flex flex-col gap-4">
		{#each groups as group (group.slug)}
			{@const slug = group.slug}
			{@const total = totals[slug] ?? null}
			{@const note = notes[slug]}
			{@const done = group.items.length - group.open}
			<section
				aria-labelledby={`group-${slug}`}
				class="overflow-hidden rounded-lg border border-line bg-card shadow-sm"
				animate:flip={{ duration: reduceMotion ? 0 : 200 }}
			>
				<header class="flex flex-col gap-3 border-b border-line bg-surface px-4 py-4 lg:flex-row lg:items-start lg:justify-between">
					<div class="flex min-w-0 flex-col gap-2">
						<h2 id={`group-${slug}`} class="m-0 text-h3 font-semibold">{group.name}</h2>
						<p class="m-0 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-ink-muted">
							<span class="font-mono">{slug}</span>
							<span aria-hidden="true">·</span>
							<span>{group.open} proposed</span>
							<span aria-hidden="true">·</span>
							<span class="font-semibold text-verified">{group.verified} verified</span>
							{#if done > 0}
								<span aria-hidden="true">·</span>
								<span>{done} decided</span>
							{/if}
							{#if group.oldest > 0}
								<span aria-hidden="true">·</span>
								<span>oldest {ago(group.oldest, nowSec)}</span>
							{/if}
						</p>
						<div class="flex h-1.5 w-full max-w-64 overflow-hidden rounded-full bg-surface-2" aria-hidden="true">
							<span class="bg-verified transition-all duration-300 motion-reduce:transition-none" style={share(done, group.items.length)}></span>
							<span class="bg-verified-bg transition-all duration-300 motion-reduce:transition-none" style={share(group.verified, group.items.length)}></span>
						</div>
					</div>
					<div class="flex flex-wrap items-center gap-2">
						<Button
							size="lg"
							disabled={!!busy[slug] || acceptIds(slug).length === 0}
							onclick={() => acceptGroup(slug)}
							aria-keyshortcuts="Shift+A"
						>
							{#if busy[slug]}
								<LoaderCircleIcon class="animate-spin motion-reduce:animate-none" aria-hidden="true" />{busy[slug]}
							{:else}
								<CheckCheckIcon aria-hidden="true" />{acceptLabel(slug, group.verified)}
							{/if}
						</Button>
						<Button size="lg" variant="outline" disabled={!!busy[slug] || group.open === 0} onclick={() => openReject(slug)}>
							Reject…
						</Button>
						{#if onOpenQueue}
							<Button size="lg" variant="ghost" onclick={() => onOpenQueue(slug)}>
								Open in queue<ArrowUpRightIcon aria-hidden="true" />
							</Button>
						{:else}
							<Button size="lg" variant="ghost" href={`${resolve('/(admin)/review')}?authority=${encodeURIComponent(slug)}`}>
								Open in queue<ArrowUpRightIcon aria-hidden="true" />
							</Button>
						{/if}
					</div>
				</header>

				{#if note}
					<div class="flex flex-wrap items-center gap-2 border-b border-line bg-error-bg px-4 py-2 text-sm" role="alert" in:fly={{ y: -4, duration: reduceMotion ? 0 : 150 }}>
						<TriangleAlertIcon class="size-4 flex-none text-error" aria-hidden="true" />
						<span class="min-w-0 flex-1 font-semibold">{note.text}</span>
						{#if note.retry}
							<Button size="sm" variant="outline" onclick={note.retry.run}>{note.retry.label}</Button>
						{/if}
						<Button size="icon-sm" variant="ghost" aria-label="Dismiss" onclick={() => delete notes[slug]}>
							<XIcon aria-hidden="true" />
						</Button>
					</div>
				{/if}

				{#if onLoadAll && total !== null && total > group.items.length}
					<div class="flex flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-2 text-xs text-ink-muted">
						<span>Showing {group.items.length} of {total} proposed</span>
						<Button size="sm" variant="outline" disabled={!!loadingAll[slug]} onclick={() => loadAll(slug)}>
							{loadingAll[slug] ? 'Working…' : `Load all ${total}`}
						</Button>
					</div>
				{/if}

				<ul class="m-0 flex list-none flex-col p-0">
					{#each group.items as item (item.id)}
						{@const decision = decided[item.id]}
						{@const risk = riskOf(item)}
						{@const check = checks[item.id]}
						{@const ok = verified(item)}
						{@const cite = item.citations[0]}
						<li
							id={`group-row-${item.id}`}
							class={`flex gap-3 border-l-2 px-4 py-3 not-first:border-t not-first:border-t-line transition-colors duration-150 ${focused === item.id ? 'border-l-accent-strong bg-surface' : 'border-l-transparent'}`}
							aria-current={focused === item.id ? 'true' : undefined}
							onfocusin={() => (focused = item.id)}
						>
							<div class="flex size-5 flex-none items-center justify-center pt-0.5">
								{#if decision}
									{#if decision === 'rejected'}
										<XIcon class="size-4 text-ink-muted" aria-hidden="true" />
									{:else}
										<CheckIcon class="size-4 text-verified" aria-hidden="true" />
									{/if}
								{:else}
									<input
										id={`pick-${item.id}`}
										type="checkbox"
										class="size-4 cursor-pointer accent-accent-strong dark:scheme-dark"
										checked={isPicked(item)}
										disabled={!!busy[item.authority_slug]}
										onchange={(e) => (picked[item.id] = e.currentTarget.checked)}
										aria-label={`Select fact #${item.id}`}
									/>
								{/if}
							</div>
							<div class="flex min-w-0 flex-1 flex-col gap-1">
								<span class="block max-w-full truncate font-mono text-xs text-ink-muted">
									#{item.id} · {item.topic} · {ago(item.created_at, nowSec)}
								</span>
								{#if decision}
									<p
										class={`m-0 text-sm font-semibold ${decision === 'rejected' ? 'text-ink-muted' : 'text-verified'}`}
										role="status"
										in:fly={{ y: -4, duration: reduceMotion ? 0 : 150 }}
									>
										Fact #{item.id} {decision}.
									</p>
								{:else}
									<label for={`pick-${item.id}`} class="cursor-pointer text-sm text-ink">{item.statement}</label>
								{/if}
								<div class="flex flex-wrap items-center gap-1">
									{#if ok && !decision}
										<Badge variant="outline" class="border-verified text-verified"><ShieldCheckIcon aria-hidden="true" />Verified</Badge>
									{/if}
									{#if risk.badge}
										<Badge variant="outline" class={riskCls(risk.badge)}>{risk.badge}</Badge>
									{/if}
									{#if check && WONDER[check.verdict]}
										<Badge variant="outline" class={WONDER[check.verdict].cls}>
											<SparklesIcon aria-hidden="true" />{WONDER[check.verdict].label}
										</Badge>
									{/if}
									{#if tags[item.id]}
										<span in:fade={{ duration: reduceMotion ? 0 : 150 }}>
											<Badge variant="outline" class="border-error text-error"><CircleAlertIcon aria-hidden="true" />{tags[item.id]}</Badge>
										</span>
									{/if}
									{#if cite?.quote}
										<Popover.Root open={quoteFor === item.id} onOpenChange={(o) => (quoteFor = o ? item.id : quoteFor === item.id ? null : quoteFor)}>
											<Popover.Trigger
												class={CHIP}
												aria-label={`Quote for fact #${item.id}, ${where(item)}`}
											>
												<QuoteIcon class="size-3 text-verified" aria-hidden="true" />{where(item)}
											</Popover.Trigger>
											<Popover.Content align="start">
												<blockquote class="m-0 border-l-2 border-verified pl-3 font-quote text-sm italic">{cite.quote}</blockquote>
												<p class="m-0 font-mono text-xs text-ink-muted">{caption(item)}</p>
											</Popover.Content>
										</Popover.Root>
									{/if}
									{#if !decision}
										<span class="ml-auto flex gap-1">
											<Button
												size="sm"
												variant="outline"
												disabled={!!rowBusy[item.id] || !!busy[item.authority_slug]}
												aria-label={`Accept fact #${item.id}`}
												onclick={() => acceptOne(item)}
											>
												<CheckIcon aria-hidden="true" />Accept
											</Button>
											<Button
												size="sm"
												variant="ghost"
												disabled={!!rowBusy[item.id] || !!busy[item.authority_slug]}
												aria-label={`Reject fact #${item.id}`}
												onclick={() => openReject(item.authority_slug, item.id)}
											>
												Reject…
											</Button>
										</span>
									{/if}
								</div>
							</div>
						</li>
					{/each}
				</ul>
			</section>
		{/each}
	</div>
{/if}

<Dialog.Root bind:open={rejectOpen}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>
				{rejectRow !== null ? `Reject fact #${rejectRow}` : `Reject proposals from ${groupOf(rejectSlug)?.name ?? rejectSlug}`}
			</Dialog.Title>
			<Dialog.Description>
				Rejected facts never reach the dossier. The reason is audited{rejectRow === null ? ' once for the whole batch, which succeeds or fails as one' : ''}.
			</Dialog.Description>
		</Dialog.Header>
		{#if rejectRow === null}
			<fieldset class="m-0 flex flex-col gap-1 border-0 p-0 text-sm">
				<legend class="mb-1 text-xs font-medium text-ink-muted">Which facts</legend>
				{#each [['unverified', 'Unverified only'], ['picked', 'Checked rows'], ['open', 'All open']] as [value, label] (value)}
					{@const n = rejectSlug ? scopeIds(rejectSlug, value as Scope).length : 0}
					<label class="flex items-center gap-2">
						<input type="radio" class="accent-accent-strong dark:scheme-dark" name="reject-scope" {value} bind:group={rejectScope} disabled={n === 0} />
						{label} ({n})
					</label>
				{/each}
			</fieldset>
		{/if}
		<label class="flex flex-col gap-1 text-sm font-medium" for="bulk-reason">
			Reason (required)
			<Textarea id="bulk-reason" rows={3} bind:value={reason} maxlength={1024} />
		</label>
		<p class="m-0 text-xs text-ink-muted">{reason.trim().length}/1024 chars</p>
		<label class="flex items-start gap-2 text-sm" for="bulk-suppress">
			<input id="bulk-suppress" type="checkbox" class="mt-1 accent-accent-strong dark:scheme-dark" bind:checked={suppressOn} />
			Also suppress future copies of {rejectRow !== null ? 'this statement' : 'these statements'}
		</label>
		{#if suppressOn}
			<fieldset class="m-0 flex flex-col gap-1 border-0 p-0 text-sm">
				<legend class="mb-1 text-xs font-medium text-ink-muted">Rule scope</legend>
				<label class="flex items-center gap-2"><input type="radio" class="accent-accent-strong dark:scheme-dark" name="bulk-sup-scope" value="authority" bind:group={supScope} />This authority</label>
				<label class="flex items-center gap-2"><input type="radio" class="accent-accent-strong dark:scheme-dark" name="bulk-sup-scope" value="global" bind:group={supScope} />Global (all authorities)</label>
				<label class="mt-1 flex items-center gap-2" for="bulk-lock">
					<input id="bulk-lock" type="checkbox" class="accent-accent-strong dark:scheme-dark" bind:checked={lockTopic} />
					Lock each rule to its topic
				</label>
			</fieldset>
		{/if}
		{#if rejectError}
			<p class="m-0 text-sm font-semibold text-error" role="alert">{rejectError}</p>
		{/if}
		<Dialog.Footer>
			<Button variant="ghost" onclick={() => (rejectOpen = false)}>Cancel</Button>
			<Button variant="destructive" disabled={rejectTargets.length === 0 || !reasonOk} onclick={submitReject}>
				Reject {plural(rejectTargets.length, 'fact')}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
