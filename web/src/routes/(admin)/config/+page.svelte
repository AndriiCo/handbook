<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import Heading from '#lib/components/brand/Heading.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Switch } from '#lib/components/ui/switch/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import {
		APPLIES_TEXT,
		CONFIG_GUIDE,
		OTHER_GROUP,
		guideEntry,
		richText,
		type Applies,
		type GuideGroup,
		type GuideKey
	} from '#lib/admin/config-guide.js';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import BookOpenIcon from '@lucide/svelte/icons/book-open';
	import LockIcon from '@lucide/svelte/icons/lock';
	import SearchIcon from '@lucide/svelte/icons/search';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import {
		csrfHeaders,
		fetchConfigWithStart,
		saveConfigKey
	} from '../../../lib/admin/config-client.js';
	import {
		CANCELLED_COPY,
		confirmPasskey,
		cookieValue,
		loginErrorCopy
	} from '../../../lib/admin/staff-auth.js';
	import RequestErrorNotice from '../../../lib/admin/RequestErrorNotice.svelte';
	import SecretInput from '../../../lib/admin/SecretInput.svelte';
	import {
		CONFIRM_WORD,
		fetchStartedAt,
		isLockoutKey,
		requestRestart,
		restartNeeded,
		savedSinceStart,
		waitForRestart
	} from '../../../lib/admin/restart-client.js';
	import {
		fetchSetupStatus,
		fetchSetupSteps,
		saveSetupKey,
		setupKeyHeaders
	} from '../../../lib/admin/setup-client.js';
	import { STEPS, REQUIRED_KEYS } from '../../../lib/admin/setup-guides.js';

	// Typed config keys per secure-config.md §1.5. SECRET rows are
	// write-only: the page shows Set/Unset and a new-value box only —
	// values are never fetched, rendered, or kept after save.
	// The admin API keeps the /apostille prefix on the admin origin
	// (spec §1.5), so the root is literal — the base-deriving trick
	// other pages use would resolve to '' on the admin host.
	//
	// Dual-mode page (secure-config.md §9.4): while the server
	// reports setup_required, this is the first-boot wizard (setup
	// key only, no session); otherwise the normal staff key list.
	const api = () => `/apostille/api/v1`;

	interface Row {
		key: string;
		type: 'bool' | 'text' | 'secret';
		value?: boolean | string;
		set?: boolean;
		draft?: string;
		dirty: boolean;
		saving: boolean;
		error: string | null;
		saved: boolean;
		// False for wizard fields the server list omits (never set):
		// completion counts only server-present rows.
		present?: boolean;
		// Server clock, unix seconds: when the row was last saved.
		updated_at?: number;
		// Triage for a failed save: the server's `x-request-id`, and
		// whether a fresh passkey step-up would let it through.
		requestId?: string | null;
		stepUp?: boolean;
		confirming?: boolean;
	}

	interface GuideField {
		key: string;
		type: 'bool' | 'text' | 'secret';
		label: string;
		hint: string;
		placeholder?: string;
	}

	interface GuideStep {
		id: string;
		title: string;
		blurb: string;
		keys: GuideField[];
		guide: { intro: string; steps: string[] };
	}

	const steps = STEPS as GuideStep[];
	const requiredKeys = REQUIRED_KEYS as string[];

	let mode = $state<'checking' | 'setup' | 'normal'>('checking');
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let rows = $state<Row[]>([]);

	let setupKey = $state('');
	let keyInput = $state('');
	let keyError = $state<string | null>(null);
	let setupRows = $state<Row[]>([]);
	let setupState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let setupError = $state<string | null>(null);
	let stepIdx = $state(0);

	// Normal mode: the guide (web/src/lib/admin/config-guide.ts) is
	// left-joined with the server list, because GET /config returns
	// only keys that have a stored row. Every item lives in $state so
	// synthesized (never stored) rows keep working Save buttons.
	interface Item {
		group: GuideGroup;
		entry: GuideKey;
		row: Row;
		present: boolean;
		known: boolean;
	}

	const GROUPS: GuideGroup[] = [...CONFIG_GUIDE, OTHER_GROUP];
	const APPLIES_LABEL: Record<Applies, string> = {
		boot: 'Restart to apply',
		live: 'Applies in seconds',
		call: 'Read every call',
		poll: 'Next mail poll',
		locked: 'Server CLI only'
	};

	let items = $state<Item[]>([]);
	let query = $state('');
	let guideOpen = $state<Record<string, boolean>>({});

	// Restart (P393): boot-time keys apply at the next start. The
	// server checks the saved config first (422 when the next boot
	// would fail), then drains and exits; systemd starts it again.
	// The overlay polls /admin/version once a second until
	// started_at moves, then refetches the list in place — no page
	// reload (the WebSockets reconnect on their own).
	let startedAt = $state<number | null>(null);
	let restartState = $state<'idle' | 'asking' | 'restarting' | 'slow' | 'done'>('idle');
	let restartError = $state<string | null>(null);
	let restartWarnings = $state<string[]>([]);
	let restartElapsed = $state(0);
	let restartFrom = 0;
	let confirmText = $state('');
	let confirmKeys = $state<string[]>([]);

	const pendingBoot = $derived(
		items.filter(
			(item) => item.present && restartNeeded(item.entry.applies, item.row.updated_at, startedAt)
		)
	);
	const riskyKeys = $derived([
		...new Set([
			...items
				.filter((item) => item.present && savedSinceStart(item.row.updated_at, startedAt))
				.map((item) => item.entry.key)
				.filter(isLockoutKey),
			...confirmKeys
		])
	]);
	const confirmOk = $derived(
		riskyKeys.length === 0 || confirmText.trim().toLowerCase() === CONFIRM_WORD
	);

	async function restart() {
		restartError = null;
		restartState = 'asking';
		try {
			const confirm = riskyKeys.length ? confirmText : '';
			const answer = await requestRestart(api(), confirm, fetch, csrfHeaders);
			if (answer.kind === 'accepted') {
				restartWarnings = answer.warnings;
				restartFrom = answer.startedAt;
				await watchRestart();
				return;
			}
			if (answer.kind === 'refused') {
				restartError = `The next boot would fail, so nothing was restarted: ${answer.message}`;
			} else if (answer.kind === 'confirm') {
				confirmKeys = answer.riskyKeys;
				restartError = answer.message;
			} else {
				restartError = answer.message;
			}
		} catch (err) {
			restartError = err instanceof Error ? err.message : 'Restart failed.';
		}
		restartState = 'idle';
	}

	async function watchRestart() {
		restartState = 'restarting';
		restartElapsed = 0;
		try {
			startedAt = await waitForRestart(api(), restartFrom, {
				onTick: (ms) => (restartElapsed = Math.round(ms / 1000))
			});
		} catch {
			restartState = 'slow';
			return;
		}
		confirmText = '';
		confirmKeys = [];
		await load({ quiet: true });
		restartState = 'done';
	}

	async function load({ quiet = false }: { quiet?: boolean } = {}) {
		if (!quiet) loadState = 'loading';
		loadError = null;
		try {
			const fetched = await fetchConfigWithStart(api());
			startedAt = fetched.startedAt ?? (await fetchStartedAt(api()).catch(() => null));
			const list = fetched.rows;
			// A quiet (in-place) refresh keeps edits still in flight.
			const kept = new Map(
				quiet ? items.filter((i) => i.row.dirty || i.row.saving).map((i) => [i.row.key, i.row]) : []
			);
			rows = list.map((row) => kept.get(row.key) ?? { ...row, saved: false });
			const server = new Map(rows.map((row) => [row.key, row]));
			const joined: Item[] = [];
			for (const group of CONFIG_GUIDE) {
				for (const entry of group.keys) {
					const row = server.get(entry.key);
					joined.push({
						group,
						entry,
						known: true,
						present: !!row,
						row: row ?? synthRow({ key: entry.key, type: entry.type, label: entry.label, hint: '' })
					});
				}
			}
			for (const row of rows) {
				if (joined.some((item) => item.entry.key === row.key)) continue;
				const { group, entry, known } = guideEntry(row.key, row.type);
				joined.push({ group, entry, known, present: true, row });
			}
			items = joined;
			loadState = 'ready';
			if (quiet) return;
			await tick();
			followHash();
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Config load failed.';
			loadState = 'failed';
		}
	}

	// Deep links: /config#stripe opens that group's setup guide,
	// /config#key-stripe.secret_key lands on one key. The list loads
	// after the browser's own hash jump, so scroll once it exists.
	function followHash() {
		const id = decodeURIComponent(window.location.hash.slice(1));
		if (!id) return;
		if (GROUPS.some((g) => g.id === id)) guideOpen[id] = true;
		const group = GROUPS.some((g) => g.id === id)
			? id
			: items.find((item) => `key-${item.entry.key}` === id)?.group.id;
		if (group) pinActive(group);
		void tick().then(() => document.getElementById(id)?.scrollIntoView({ block: 'start' }));
	}

	function isSet(item: Item): boolean {
		if (!item.present) return false;
		if (item.row.type === 'bool') return true;
		if (item.row.type === 'secret') return item.row.set === true;
		return textValue(item.row).trim().length > 0;
	}

	function statusText(item: Item): string {
		const fallback = item.entry.default ? ` · default ${item.entry.default}` : '';
		if (!item.present) return `Not set${fallback}`;
		if (item.row.type === 'bool') return item.row.value === true ? 'On' : 'Off';
		if (item.row.type === 'secret') return item.row.set ? 'Set · value hidden' : 'Not set';
		return textValue(item.row).trim() ? 'Set' : `Empty${fallback}`;
	}

	function matches(item: Item, q: string): boolean {
		if (!q) return true;
		const hay = `${item.entry.key} ${item.entry.label} ${item.group.title}`.toLowerCase();
		return q
			.toLowerCase()
			.split(/\s+/)
			.filter(Boolean)
			.every((word) => hay.includes(word));
	}

	const sections = $derived(
		GROUPS.map((group) => {
			const all = items.filter((item) => item.group.id === group.id);
			return {
				group,
				all,
				shown: all.filter((item) => matches(item, query.trim())),
				setCount: all.filter(isSet).length
			};
		}).filter((s) => s.all.length > 0)
	);
	const visibleSections = $derived(sections.filter((s) => s.shown.length > 0));
	const totalSet = $derived(items.filter(isSet).length);

	async function saveItem(item: Item) {
		await save(item.row);
		if (item.row.saved) item.present = true;
	}

	// Full class strings per state (the tailwind checker reads literals).
	function appliesCls(applies: Applies): string {
		const base = 'inline-flex h-6 items-center rounded-full px-2 text-xs font-medium';
		if (applies === 'live' || applies === 'call') return `${base} bg-verified-bg text-verified`;
		if (applies === 'locked') return `${base} bg-surface-2 text-ink-muted`;
		return `${base} border border-line text-ink-muted`;
	}

	// Table of contents: chips that scroll sideways on phones, a
	// sticky list at xl. The group nearest the top is marked current,
	// except right after a jump: the clicked group stays current until
	// the reader scrolls (a short last group can never reach the top,
	// so the observer alone would mark the group above it).
	let activeId = $state<string | null>(null);
	let pinnedId: string | null = null;

	function pinActive(id: string) {
		activeId = id;
		pinnedId = id;
	}

	function tocCls(active: boolean, dim: boolean): string {
		const base =
			'flex h-8 items-center gap-2 rounded-full border px-3 text-sm whitespace-nowrap transition-colors focus-visible:ring-2 focus-visible:ring-focus focus-visible:outline-none xl:h-7 xl:w-full xl:justify-between xl:rounded-md xl:border-transparent xl:px-2';
		const tone = active
			? 'border-line bg-surface-2 font-medium text-ink'
			: dim
				? 'border-line bg-card text-ink-muted opacity-60 hover:bg-muted'
				: 'border-line bg-card text-ink hover:bg-muted xl:bg-transparent';
		return `${base} ${tone}`;
	}

	$effect(() => {
		if (mode !== 'normal' || loadState !== 'ready' || typeof IntersectionObserver === 'undefined') return;
		void visibleSections.length;
		const seen = new Map<string, boolean>();
		const observer = new IntersectionObserver(
			(records) => {
				for (const r of records) seen.set(r.target.id, r.isIntersecting);
				if (pinnedId) return;
				activeId = GROUPS.find((g) => seen.get(g.id))?.id ?? activeId;
			},
			{ rootMargin: '-64px 0px -60% 0px' }
		);
		for (const g of GROUPS) {
			const el = document.getElementById(g.id);
			if (el) observer.observe(el);
		}
		// Only the reader's own scrolling releases a pin; the jump's
		// own scroll events must not.
		const release = () => {
			pinnedId = null;
		};
		const onKey = (e: KeyboardEvent) => {
			if (['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '].includes(e.key)) release();
		};
		window.addEventListener('wheel', release, { passive: true });
		window.addEventListener('touchmove', release, { passive: true });
		window.addEventListener('keydown', onKey);
		return () => {
			observer.disconnect();
			window.removeEventListener('wheel', release);
			window.removeEventListener('touchmove', release);
			window.removeEventListener('keydown', onKey);
		};
	});

	function countCls(set: number, total: number): string {
		const base = 'inline-flex h-6 items-center rounded-full px-2 font-mono text-xs tabular-nums';
		return set === total ? `${base} bg-verified-bg text-verified` : `${base} bg-surface-2 text-ink-muted`;
	}

	async function loadSteps() {
		setupState = 'loading';
		setupError = null;
		try {
			const list = await fetchSetupSteps(api(), setupKey);
			// Left-join every wizard field with the server list so
			// ALL rows live in $state: previously unset keys got
			// fresh plain objects per render, whose edits Svelte
			// never tracked — their Save buttons stayed disabled
			// forever. In-flight edits survive the refetch; the
			// "Saved." flag carries across it.
			const prev = new Map(setupRows.map((row) => [row.key, row]));
			const server = new Map(list.map((row) => [row.key, row]));
			setupRows = steps.flatMap((step) => step.keys).map((field) => {
				const old = prev.get(field.key);
				if (old && (old.dirty || old.saving)) return old;
				const row = server.get(field.key);
				const fresh: Row = row
					? { ...row, saved: false, present: true }
					: { ...synthRow(field), present: false };
				fresh.saved = old?.saved ?? false;
				return fresh;
			});
			setupState = 'ready';
		} catch (err) {
			setupError = err instanceof Error ? err.message : 'Setup steps failed.';
			setupState = 'failed';
		}
	}

	function startSetup(event: SubmitEvent) {
		event.preventDefault();
		keyError = null;
		if (!setupKeyHeaders(keyInput.trim())) {
			keyError =
				'That key is malformed — `setup_` + 64 hex chars, as printed by `xtask setup-key-mint`.';
			return;
		}
		setupKey = keyInput.trim();
		keyInput = '';
		void loadSteps();
	}

	function forgetKey() {
		setupKey = '';
		setupRows = [];
		setupState = 'idle';
		setupError = null;
		stepIdx = 0;
	}

	onMount(async () => {
		try {
			const status = await fetchSetupStatus(api());
			if (status.setup_required) {
				mode = 'setup';
				return;
			}
		} catch {
			// Status unreachable: fall through to the normal list,
			// which surfaces the real error.
		}
		mode = 'normal';
		await load();
	});

	onMount(() => {
		const onHash = () => {
			if (mode === 'normal' && loadState === 'ready') followHash();
		};
		window.addEventListener('hashchange', onHash);
		return () => window.removeEventListener('hashchange', onHash);
	});

	function markDirty(row: Row) {
		row.dirty = true;
		row.saved = false;
		row.error = null;
		row.requestId = null;
		row.stepUp = false;
	}

	async function save(row: Row) {
		row.saving = true;
		row.error = null;
		try {
			const { updatedAt } = await saveConfigKey(api(), row);
			if (updatedAt !== null) row.updated_at = updatedAt;
			row.dirty = false;
			row.saved = true;
			if (row.type === 'secret') {
				row.set = true;
				row.draft = '';
			}
		} catch (err) {
			row.error = err instanceof Error ? err.message : 'Save failed.';
			const info = err as { requestId?: string | null; stepUp?: boolean };
			row.requestId = info?.requestId ?? null;
			row.stepUp = info?.stepUp === true;
		} finally {
			row.saving = false;
		}
	}

	// Config writes need a passkey step-up from the last 10 minutes:
	// confirm in place (same UV ceremony as login, for this session's
	// own staff id), then retry the save — no re-login, no lost draft.
	async function staffGet(path: string) {
		const res = await fetch(`${api()}${path}`, { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	async function staffPost(path: string, body: unknown) {
		const res = await fetch(`${api()}${path}`, {
			method: 'POST',
			headers: {
				'content-type': 'application/json',
				'x-csrf': cookieValue(document.cookie, 'hb_csrf')
			},
			credentials: 'same-origin',
			body: JSON.stringify(body)
		});
		const parsed = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body: parsed };
	}

	async function confirmAndSave(row: Row) {
		row.confirming = true;
		try {
			const res = await confirmPasskey({
				get: staffGet,
				post: staffPost,
				credentials: navigator.credentials
			});
			if (!res.ok) {
				row.error = loginErrorCopy(res.status, res.body?.error ?? '');
				return;
			}
			row.stepUp = false;
			await save(row);
		} catch (err) {
			row.error =
				err instanceof DOMException && err.name === 'NotAllowedError'
					? CANCELLED_COPY
					: 'Passkey check failed — try again.';
		} finally {
			row.confirming = false;
		}
	}

	async function saveSetup(row: Row): Promise<boolean> {
		row.saving = true;
		row.error = null;
		try {
			await saveSetupKey(api(), setupKey, row);
			row.dirty = false;
			row.saved = true;
			if (row.type === 'secret') {
				row.set = true;
				row.draft = '';
			}
			await loadSteps();
			return true;
		} catch (err) {
			const message = err instanceof Error ? err.message : 'Save failed.';
			if (/window already closed/.test(message)) {
				setupError = message;
				setupState = 'failed';
				return false;
			}
			row.error = message;
			return false;
		} finally {
			row.saving = false;
		}
	}

	// Step change with auto-save: dirty fields on the current step
	// save first (sequentially, showing row errors); a failure
	// keeps the operator on the step instead of losing edits.
	let navSaving = $state(false);

	async function goStep(i: number) {
		if (i === stepIdx || navSaving) return;
		const dirty = steps[stepIdx].keys.map(setupRowFor).filter((row) => row.dirty);
		if (dirty.length > 0) {
			navSaving = true;
			try {
				for (const row of dirty) {
					if (!(await saveSetup(row))) return;
				}
			} finally {
				navSaving = false;
			}
		}
		stepIdx = i;
	}

	function synthRow(field: GuideField): Row {
		if (field.type === 'bool') {
			return { key: field.key, type: 'bool', value: false, dirty: false, saving: false, error: null, saved: false };
		}
		if (field.type === 'secret') {
			return { key: field.key, type: 'secret', set: false, draft: '', dirty: false, saving: false, error: null, saved: false };
		}
		return { key: field.key, type: 'text', value: '', dirty: false, saving: false, error: null, saved: false };
	}

	function setupRowFor(field: GuideField): Row {
		// loadSteps left-joins every field, so the find always hits;
		// the synth fallback only guards a future divergence.
		return setupRows.find((row) => row.key === field.key) ?? synthRow(field);
	}

	function textValue(row: Row): string {
		return typeof row.value === 'string' ? row.value : '';
	}

	function requiredMet(key: string): boolean {
		const row = setupRows.find((r) => r.key === key);
		if (!row) return false;
		if (row.type === 'bool') return row.value === true;
		if (row.type === 'secret') return row.set === true;
		return textValue(row).trim().length > 0;
	}

	const missingRequired = $derived(requiredKeys.filter((key) => !requiredMet(key)));

	function stepDoneCount(step: GuideStep): number {
		return step.keys.filter((field) => {
			const row = setupRows.find((r) => r.key === field.key);
			if (!row) return false;
			if (row.type === 'bool') return row.present === true;
			if (row.type === 'secret') return row.set === true;
			return textValue(row).trim().length > 0;
		}).length;
	}

	function stepFor(key: string): number {
		const idx = steps.findIndex((step) => step.keys.some((field) => field.key === key));
		return idx < 0 ? 0 : idx;
	}

	// Full class strings per state (not stacked class: directives):
	// the active fill must never co-occur with the idle border in
	// the cascade — and the tailwind checker only reads literals.
	function stepBtnCls(active: boolean): string {
		const base = 'min-h-11 cursor-pointer rounded-sm border px-3 font-bold disabled:cursor-default';
		return active ? `${base} border-accent-fill bg-accent-fill text-accent-ink` : `${base} border-line bg-bg`;
	}
</script>

<svelte:head>
	<title>{mode === 'setup' ? 'First-time setup' : 'Configuration'} — Admin — Apostille Handbook</title>
</svelte:head>

{#if mode === 'checking'}
	<div class="mt-6 flex flex-col gap-6 pb-12">
		<Heading level={1} eyebrow="System">Configuration</Heading>
		{@render loadingBlock()}
	</div>
{:else if mode === 'setup'}
	<Heading level={1} eyebrow="System">First-time setup</Heading>

	<Notice variant="warning" heading="One key opens this wizard.">
		<p>
			Paste the setup key printed once by <code>xtask setup-key-mint</code> on the
			server. It works from any browser until the first staff passkey enrolment
			burns it — then this wizard closes permanently and the normal config list
			takes over. The key is never stored in the browser.
		</p>
	</Notice>

	{#if !setupKey}
		<form
			class="mt-4 grid max-w-[28rem] gap-3"
			onsubmit={(e) => {
				e.preventDefault();
				startSetup(e);
			}}
		>
			<div class="flex flex-col gap-[0.35rem]">
				<label class="font-bold" for="setup-key">Setup key</label>
				<SecretInput
					id="setup-key"
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-mono font-normal text-ink"
					bind:value={keyInput}
					placeholder="setup_…"
					hint="One-time key from the server console. Your browser will not offer to save it."
				/>
			</div>
			{#if keyError}
				<Notice variant="error" heading="Bad key."><p>{keyError}</p></Notice>
			{/if}
			<p>
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
					type="submit">Start / resume setup</button
				>
			</p>
		</form>
	{:else if setupState === 'loading' || setupState === 'idle'}
		<p aria-live="polite">Loading setup progress…</p>
	{:else if setupState === 'failed'}
		<Notice variant="error" heading="Setup unavailable."><p>{setupError}</p></Notice>
		<p class="flex gap-2">
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
				type="button"
				onclick={loadSteps}>Retry</button
			>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-line bg-bg px-5 font-bold"
				type="button"
				onclick={forgetKey}>Use a different key</button
			>
		</p>
	{:else}
		<nav aria-label="Setup steps">
			<ol class="m-0 flex list-none flex-wrap gap-2 p-0">
				{#each steps as step, i (step.id)}
					<li>
						<button
							class={stepBtnCls(i === stepIdx)}
							type="button"
							disabled={i === stepIdx || navSaving}
							onclick={() => goStep(i)}
							aria-current={i === stepIdx ? 'step' : undefined}
						>
							{i + 1}. {step.title} ({stepDoneCount(step)}/{step.keys.length})
						</button>
					</li>
				{/each}
			</ol>
		</nav>

		{@const step = steps[stepIdx]}
		<section aria-labelledby="setup-step">
			<h2 id="setup-step">{stepIdx + 1}. {step.title}</h2>
			<p class="max-w-[44rem]">{step.blurb}</p>

			<div class="my-3 max-w-[44rem] rounded-sm border border-line bg-surface-2 p-4" role="note" aria-label="Retrieval guide">
				<p class="m-0 mb-1 text-sm font-bold">How to get these values</p>
				<p class="m-0 mb-1 text-sm">{step.guide.intro}</p>
				<ol class="m-0 text-sm">
					{#each step.guide.steps as text (text)}
						<li class="mb-1">{text}</li>
					{/each}
				</ol>
			</div>

			<ul class="m-0 mt-4 grid max-w-[44rem] list-none gap-4 p-0 pb-8">
				{#each step.keys as field (field.key)}
					{@const row = setupRowFor(field)}
					<li class="rounded-sm border border-line px-4 py-[0.9rem]">
						<div class="mb-2 flex flex-wrap items-baseline gap-x-3 gap-y-1">
							<code class="font-mono font-semibold break-all">{field.label}</code>
							<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">{field.type}</span>
							{#if requiredKeys.includes(field.key)}
								<span class="text-[0.8rem] font-bold tracking-[0.08em] text-error uppercase">required</span>
							{/if}
						</div>
						<p class="m-0 mb-2 font-mono text-sm break-all">{field.key}</p>
						<p class="m-0 mb-2 text-sm text-ink-muted">{field.hint}</p>
						{#if field.type === 'bool'}
							<label class="min-h-11">
								<input
									class="mr-2 size-5 accent-accent align-middle"
									type="checkbox"
									checked={row.value === true}
									onchange={(e) => {
										row.value = e.currentTarget.checked;
										markDirty(row);
									}}
								/>
								Enabled
							</label>
						{:else if field.type === 'text'}
							<label class="flex min-h-11 flex-col gap-[0.35rem] font-bold" for={`setup-${field.key}`}>
								Value
								<input
									class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink"
									id={`setup-${field.key}`}
									type="text"
									value={textValue(row)}
									placeholder={field.placeholder ?? ''}
									maxlength={4096}
									oninput={(e) => {
										row.value = e.currentTarget.value;
										markDirty(row);
										}}
								/>
							</label>
						{:else}
							<p class="mt-1 mb-2 font-bold" aria-live="polite">
								{row.set ? 'Set (value hidden)' : 'Not set'}
							</p>
							<div class="flex flex-col gap-[0.35rem]">
								<label class="font-bold" for={`setup-${field.key}`}>
									New value for {field.key} (write-only, overwrites)
								</label>
								<SecretInput
									id={`setup-${field.key}`}
									class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink"
									value={row.draft ?? ''}
									oninput={(e) => {
										row.draft = e.currentTarget.value;
										markDirty(row);
									}}
								/>
							</div>
						{/if}
						{#if row.error}
							<RequestErrorNotice
								message={row.error}
								requestId={row.requestId}
								stepUp={row.stepUp}
								busy={row.confirming}
								onConfirm={() => confirmAndSave(row)}
							/>
						{/if}
						{#if row.saved}
							<Notice variant="success" heading="Saved.">
								<p>“{row.key}” updated and audited.</p>
							</Notice>
						{/if}
						<p>
							<button
								class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
								onclick={() => saveSetup(row)}
								disabled={!row.dirty || row.saving || (row.type === 'secret' && !row.draft)}
							>
								{row.saving ? 'Saving…' : 'Save'}
							</button>
						</p>
					</li>
				{/each}
			</ul>

			<div class="flex gap-2 pb-8">
				{#if stepIdx > 0}
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-line bg-bg px-5 font-bold disabled:cursor-not-allowed disabled:opacity-55"
						type="button"
						disabled={navSaving}
						onclick={() => goStep(stepIdx - 1)}>← {steps[stepIdx - 1].title}</button
					>
				{/if}
				{#if stepIdx < steps.length - 1}
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-55"
						type="button"
						disabled={navSaving}
						onclick={() => goStep(stepIdx + 1)}>{navSaving ? 'Saving…' : `${steps[stepIdx + 1].title} →`}</button
					>
				{/if}
			</div>
		</section>

		<section aria-labelledby="setup-review">
			<h2 id="setup-review">Review</h2>
			<p>
				{requiredKeys.length - missingRequired.length} of {requiredKeys.length} required keys set.
			</p>
			{#if missingRequired.length === 0}
				<Notice variant="success" heading="Setup complete.">
					<p>
						Every required key is set. Enrol the first staff passkey to burn the setup
						key and close this wizard — the normal config list takes over after.
					</p>
				</Notice>
			{:else}
				<p>Still missing:</p>
				<ul class="m-0 list-none p-0 pb-4">
					{#each missingRequired as key (key)}
						<li>
							<button
								class="min-h-11 cursor-pointer font-mono disabled:cursor-not-allowed disabled:opacity-55"
								type="button"
								disabled={navSaving}
								onclick={() => goStep(stepFor(key))}
							>
								{key}
							</button>
						</li>
					{/each}
				</ul>
			{/if}
			<p class="flex flex-wrap gap-2 pb-8">
				<a
					class="inline-flex min-h-11 items-center rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink"
					href={`${resolve('/(admin)/enrol')}?setup=1`}
					>Enrol the first passkey (burns the setup key)</a
				>
			</p>
		</section>
	{/if}
{:else}
	<div class="mt-6 flex flex-col gap-6 pb-12">
		<header class="flex flex-col gap-2">
			<Heading level={1} eyebrow="System">Configuration</Heading>
			<p class="m-0 max-w-prose text-sm text-ink-muted">
				Encrypted server config, grouped by what it powers. Each group carries its setup guide once;
				each key says what it does, where to get it, its format and when a change takes effect.
				Secrets are write-only and every save writes an audit row.
			</p>
		</header>

		{#if loadState === 'loading'}
			{@render loadingBlock()}
		{:else if loadState === 'failed'}
			<Notice variant="error" heading="Config could not load.">
				<p class="m-0">{loadError}</p>
				<p class="mt-3 mb-0"><Button onclick={load}>Try again</Button></p>
			</Notice>
		{:else}
			{#if rows.length === 0}
				<Notice variant="info" heading="No key is stored yet.">
					<p class="m-0">
						Every key below runs on its compiled default until you save it here.
					</p>
				</Notice>
			{/if}

			{@render restartCard()}

			<div class="flex flex-col gap-6 xl:grid xl:grid-cols-4 xl:items-start xl:gap-8">
				<aside class="flex min-w-0 flex-col gap-3 xl:sticky xl:top-20 xl:col-span-1">
					<p class="m-0 text-sm text-ink-muted" aria-live="polite">
						<span class="font-semibold text-ink tabular-nums">{totalSet}</span> of
						<span class="tabular-nums">{items.length}</span> keys set ·
						<span class="tabular-nums">{items.length - totalSet}</span> on defaults
					</p>
					<div class="relative w-full md:max-w-sm xl:max-w-none">
						<SearchIcon
							class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-ink-muted"
							aria-hidden="true"
						/>
						<Input
							type="search"
							class="h-10 pl-9"
							aria-label="Filter keys by name, key or group"
							placeholder="Filter, e.g. stripe or port"
							bind:value={query}
						/>
					</div>
					<nav aria-label="Config groups" class="-mx-4 min-w-0 md:mx-0">
						<ul
							class="m-0 flex list-none gap-2 overflow-x-auto px-4 pb-1 md:flex-wrap md:overflow-visible md:px-0 xl:flex-col xl:flex-nowrap xl:gap-0.5"
						>
							{#each sections as s (s.group.id)}
								<li class="flex-none">
									<a
										href={`#${s.group.id}`}
										onclick={() => pinActive(s.group.id)}
										aria-current={activeId === s.group.id ? 'location' : undefined}
										class={tocCls(activeId === s.group.id, s.shown.length === 0)}
									>
										<span class="truncate" title={s.group.title}>{s.group.title}</span>
										<span class="font-mono text-xs text-ink-muted tabular-nums">{s.setCount}/{s.all.length}</span>
									</a>
								</li>
							{/each}
						</ul>
					</nav>
				</aside>

				<div class="flex min-w-0 flex-col gap-6 xl:col-span-3">
					{#if visibleSections.length === 0}
						<div
							class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line bg-card px-6 py-12 text-center"
						>
							<SearchIcon class="size-8 text-ink-muted" aria-hidden="true" />
							<p class="m-0 text-h3 font-semibold">No key matches “{query.trim()}”</p>
							<Button variant="outline" onclick={() => (query = '')}>Show every key</Button>
						</div>
					{/if}

					{#each visibleSections as s (s.group.id)}
						{@render section(s.group, s.shown, s.setCount, s.all.length)}
					{/each}
				</div>
			</div>
		{/if}
	</div>
{/if}

{#snippet rich(text: string)}
	{#each richText(text) as run, i (i)}
		{#if run.kind === 'link'}
			<a
				class="inline font-medium wrap-anywhere text-accent-strong underline decoration-line underline-offset-2 hover:decoration-current"
				href={run.href}
				target="_blank"
				rel="noreferrer"
				>{run.text}<ExternalLinkIcon class="ml-0.5 inline size-3 align-baseline" aria-hidden="true" /><span class="sr-only">
					(opens in a new tab)</span
				></a
			>
		{:else if run.kind === 'code'}
			<code class="rounded-xs bg-surface-2 px-1 font-mono text-xs wrap-anywhere">{run.text}</code>
		{:else if run.kind === 'key'}
			<a class="rounded-xs bg-surface-2 px-1 font-mono text-xs wrap-anywhere text-accent-strong hover:underline" href={run.href}
				>{run.text}</a
			>
		{:else}{run.text}{/if}
	{/each}
{/snippet}

{#snippet loadingBlock()}
	<div class="flex flex-col gap-4" role="status" aria-label="Loading config keys">
		<div class="flex flex-wrap gap-2">
			{#each [0, 1, 2, 3, 4, 5, 6] as i (i)}
				<Skeleton class="h-8 w-28 rounded-full" />
			{/each}
		</div>
		{#each [0, 1] as i (i)}
			<div class="flex flex-col gap-3 rounded-lg border border-line bg-card p-4 shadow-sm md:p-6">
				<Skeleton class="h-6 w-48 rounded-xs" />
				<Skeleton class="h-4 w-full max-w-prose rounded-xs" />
				<Skeleton class="h-4 w-2/3 max-w-prose rounded-xs" />
				<Skeleton class="mt-2 h-24 w-full rounded-md" />
			</div>
		{/each}
	</div>
{/snippet}

{#snippet section(group: GuideGroup, shown: Item[], setCount: number, total: number)}
	<section
		id={group.id}
		aria-labelledby={`group-${group.id}`}
		class="scroll-mt-20 overflow-hidden rounded-lg border border-line bg-card shadow-sm"
	>
		<header class="flex flex-col gap-2 p-4 md:px-6 md:pt-6">
			<div class="flex flex-wrap items-center gap-x-3 gap-y-1">
				<h2 id={`group-${group.id}`} class="m-0 text-h3 font-semibold">{group.title}</h2>
				<span class={countCls(setCount, total)}>{setCount} of {total} set</span>
			</div>
			<p class="m-0 max-w-prose text-sm text-ink-muted">{@render rich(group.summary)}</p>
		</header>
		<details class="group border-t border-line bg-surface" bind:open={guideOpen[group.id]}>
			<summary
				class="flex min-h-11 cursor-pointer list-none items-center gap-2 px-4 text-sm font-medium select-none hover:text-accent-strong focus-visible:ring-2 focus-visible:ring-focus focus-visible:outline-none md:px-6"
			>
				<ChevronRightIcon
					class="size-4 flex-none transition-transform duration-200 group-open:rotate-90 motion-reduce:transition-none"
					aria-hidden="true"
				/>
				<BookOpenIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
				Setup guide
				<span class="text-ink-muted">· {group.setup.length} step{group.setup.length === 1 ? '' : 's'}</span>
			</summary>
			<ol class="m-0 flex list-none flex-col gap-3 px-4 pt-1 pb-4 md:px-6 md:pb-6">
				{#each group.setup as step, i (i)}
					<li class="flex gap-3 text-sm">
						<span
							class="flex size-6 flex-none items-center justify-center rounded-full bg-surface-2 font-mono text-xs font-semibold text-accent-strong"
							aria-hidden="true">{i + 1}</span
						>
						<p class="m-0 max-w-prose min-w-0">
							<span class="sr-only">Step {i + 1}: </span>{@render rich(step)}
						</p>
					</li>
				{/each}
			</ol>
		</details>
		<ul class="m-0 list-none divide-y divide-line border-t border-line p-0">
			{#each shown as item (item.entry.key)}
				{@render keyRow(item)}
			{/each}
		</ul>
	</section>
{/snippet}

{#snippet keyRow(item: Item)}
	{@const row = item.row}
	{@const entry = item.entry}
	{@const inputId = `cfg-${entry.key}`}
	<li id={`key-${entry.key}`} class="grid scroll-mt-20 gap-4 p-4 md:px-6 lg:grid-cols-5 lg:gap-8">
		<div class="flex min-w-0 flex-col gap-2 lg:col-span-3">
			<div class="flex flex-wrap items-center gap-2">
				<h3 class="m-0 text-body font-semibold">{entry.label}</h3>
				<Badge variant="outline" class="font-mono">
					{#if entry.type === 'secret'}<LockIcon aria-hidden="true" />{/if}{entry.type}
				</Badge>
				<span class={appliesCls(entry.applies)}>{APPLIES_LABEL[entry.applies]}</span>
				{#if item.present && restartNeeded(entry.applies, row.updated_at, startedAt)}
					<span
						class="inline-flex h-6 items-center rounded-full bg-warning-bg px-2 text-xs font-medium text-warning"
						>Restart needed</span
					>
				{/if}
			</div>
			<code class="font-mono text-xs break-all text-ink-muted">{entry.key}</code>
			<dl class="m-0 flex flex-col gap-2 text-sm">
				<div>
					<dt class="text-xs font-medium text-ink-muted">What it does</dt>
					<dd class="m-0">{@render rich(entry.what)}</dd>
				</div>
				<div>
					<dt class="text-xs font-medium text-ink-muted">Where to get it</dt>
					<dd class="m-0">{@render rich(entry.how)}</dd>
				</div>
				<div>
					<dt class="text-xs font-medium text-ink-muted">Format</dt>
					<dd class="m-0 font-mono text-xs text-ink-muted">{@render rich(entry.format)}</dd>
				</div>
				<div>
					<dt class="text-xs font-medium text-ink-muted">Takes effect</dt>
					<dd class="m-0">{APPLIES_TEXT[entry.applies]}</dd>
				</div>
			</dl>
		</div>

		<div class="flex min-w-0 flex-col gap-3 lg:col-span-2">
			<p class="m-0 flex items-center gap-2 text-sm" aria-live="polite">
				<span
					class={`size-2 flex-none rounded-full ${isSet(item) ? 'bg-verified' : 'bg-ink-muted'}`}
					aria-hidden="true"
				></span>
				<span class={isSet(item) ? 'font-medium' : 'text-ink-muted'}>{statusText(item)}</span>
			</p>

			{#if item.present && item.known && row.type !== entry.type}
				<Notice variant="warning" heading="Stored with the wrong type.">
					<p class="m-0 text-sm">
						The server holds this key as {row.type}, but the registry types it {entry.type}. The
						server skips a mistyped row at boot and runs on the default.
					</p>
				</Notice>
			{/if}

			{#if entry.readOnly}
				<p class="m-0 flex items-center gap-2 rounded-md border border-dashed border-line px-3 py-2 text-sm text-ink-muted">
					<LockIcon class="size-4 flex-none" aria-hidden="true" />
					Managed from the server CLI. This page cannot change it.
				</p>
			{:else}
				{#if row.type === 'bool'}
					<div class="flex min-h-11 items-center gap-3">
						<!-- Hit area stays inside the control (the row is the touch target). -->
						<Switch
							id={inputId}
							class="after:inset-x-0"
							checked={row.value === true}
							onCheckedChange={(checked) => {
								row.value = checked;
								markDirty(row);
							}}
							aria-label={`${entry.label} (${entry.key})`}
						/>
						<label for={inputId} class="text-sm">{row.value === true ? 'On' : 'Off'}</label>
					</div>
				{:else if row.type === 'text'}
					<div class="flex flex-col gap-1">
						<label for={inputId} class="text-xs font-medium text-ink-muted">Value</label>
						<Input
							id={inputId}
							type="text"
							class="h-10 font-mono"
							value={textValue(row)}
							placeholder={entry.default && entry.default !== 'unset' ? entry.default : ''}
							maxlength={4096}
							autocomplete="off"
							spellcheck={false}
							oninput={(e) => {
								row.value = e.currentTarget.value;
								markDirty(row);
							}}
						/>
					</div>
				{:else}
					<div class="flex flex-col gap-1">
						<label for={inputId} class="text-xs font-medium text-ink-muted">
							New value for {entry.label} (write-only, overwrites)
						</label>
						<SecretInput
							id={inputId}
							class="h-10 rounded-md border border-line bg-bg px-3 font-mono text-sm text-ink"
							value={row.draft ?? ''}
							oninput={(e) => {
								row.draft = e.currentTarget.value;
								markDirty(row);
							}}
						/>
					</div>
				{/if}
				{#if row.error}
					<RequestErrorNotice
						message={row.error}
						requestId={row.requestId}
						stepUp={row.stepUp}
						busy={row.confirming}
						onConfirm={() => confirmAndSave(row)}
					/>
				{/if}
				<div class="flex flex-wrap items-center gap-3">
					<Button
						size="lg"
						onclick={() => saveItem(item)}
						disabled={!row.dirty || row.saving || (row.type === 'secret' && !row.draft)}
					>
						{row.saving ? 'Saving…' : 'Save'}
					</Button>
					{#if row.saved}
						<span class="inline-flex items-center gap-1 text-sm text-verified" role="status">
							<CheckIcon class="size-4" aria-hidden="true" /> Saved and audited
						</span>
					{/if}
				</div>
			{/if}
		</div>
	</li>
{/snippet}

{#snippet restartCard()}
	<section
		aria-labelledby="restart-heading"
		class="flex flex-col gap-3 rounded-lg border border-line bg-card p-4 shadow-sm md:p-6"
	>
		<div class="flex flex-wrap items-center gap-x-3 gap-y-1">
			<h2 id="restart-heading" class="m-0 text-h3 font-semibold">Restart the service</h2>
			{#if pendingBoot.length > 0}
				<span
					class="inline-flex h-6 items-center rounded-full bg-warning-bg px-2 text-xs font-medium text-warning"
					>{pendingBoot.length} boot-time change{pendingBoot.length === 1 ? '' : 's'} waiting</span
				>
			{/if}
		</div>
		<p class="m-0 max-w-prose text-sm text-ink-muted">
			Keys marked “Restart to apply” take effect at the next start. Restart checks the saved config
			first and refuses if the next boot would fail. This page then waits and reloads its data in
			place.
		</p>
		{#if pendingBoot.length > 0}
			<p class="m-0 flex flex-wrap items-center gap-1 text-sm">
				Saved since the last start:
				{#each pendingBoot as item (item.entry.key)}
					<a
						class="rounded-xs bg-surface-2 px-1 font-mono text-xs text-accent-strong hover:underline"
						href={`#key-${item.entry.key}`}>{item.entry.key}</a
					>
				{/each}
			</p>
		{/if}
		{#if riskyKeys.length > 0}
			<Notice variant="warning" heading="These changes can lock staff out.">
				<p class="m-0 text-sm">
					{riskyKeys.join(', ')} changed since the last start. A wrong value here can keep the admin
					host unreachable after the restart. Check them, then type “{CONFIRM_WORD}” below.
				</p>
			</Notice>
			<div class="flex max-w-sm flex-col gap-1">
				<label for="restart-confirm" class="text-xs font-medium text-ink-muted"
					>Type “{CONFIRM_WORD}” to confirm</label
				>
				<Input
					id="restart-confirm"
					type="text"
					class="h-10 font-mono"
					autocomplete="off"
					spellcheck={false}
					bind:value={confirmText}
				/>
			</div>
		{/if}
		{#if restartError}
			<Notice variant="error" heading="Not restarted.">
				<p class="m-0 text-sm">{restartError}</p>
			</Notice>
		{/if}
		{#if restartState === 'done' && startedAt}
			<Notice variant="success" heading="Restarted.">
				<p class="m-0 text-sm">
					Running since {new Date(startedAt * 1000).toLocaleTimeString()}. Boot-time keys saved
					before the restart are live.
				</p>
				{#each restartWarnings as warning (warning)}
					<p class="m-0 text-sm">Boot warning: {warning}</p>
				{/each}
			</Notice>
		{/if}
		<div>
			<Button
				size="lg"
				onclick={restart}
				disabled={!confirmOk || restartState === 'asking' || restartState === 'restarting'}
			>
				{restartState === 'asking' ? 'Checking the config…' : 'Restart service'}
			</Button>
		</div>
	</section>
{/snippet}

{#if restartState === 'restarting' || restartState === 'slow'}
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
		role="dialog"
		aria-modal="true"
		aria-labelledby="restart-overlay-title"
	>
		<div class="flex max-w-md flex-col gap-3 rounded-lg border border-line bg-card p-6 shadow-sm">
			{#if restartState === 'restarting'}
				<h2 id="restart-overlay-title" class="m-0 text-h3 font-semibold">Restarting…</h2>
				<p class="m-0 text-sm text-ink-muted" role="status">
					Waiting for the service to come back ({restartElapsed} s). The data reloads by itself.
				</p>
			{:else}
				<h2 id="restart-overlay-title" class="m-0 text-h3 font-semibold">Still restarting</h2>
				<p class="m-0 text-sm text-ink-muted">
					The service has not come back after two minutes. Check
					<code class="font-mono text-xs">journalctl -u apostille</code> on the server.
				</p>
				<div class="flex flex-wrap gap-2">
					<Button onclick={watchRestart}>Keep waiting</Button>
					<Button variant="outline" onclick={() => (restartState = 'idle')}>Close</Button>
				</div>
			{/if}
		</div>
	</div>
{/if}
