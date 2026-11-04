<script lang="ts">
	import { onMount } from 'svelte';
	import { fly } from 'svelte/transition';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { toast } from 'svelte-sonner';
	import Heading from '#lib/components/brand/Heading.svelte';
	import Notice from '#lib/public-ui/Notice.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import AuthorityPicker from '#lib/admin/AuthorityPicker.svelte';
	import OpenQuestions from '#lib/admin/OpenQuestions.svelte';
	import { OpenQuestionsState } from '#lib/admin/first-source.svelte.js';
	import type { AuthorityOption } from '#lib/admin/authorities.js';
	import { ago, dateTime } from '#lib/admin/format.js';
	import {
		MAX_CALL_QUESTIONS,
		TOP_N,
		appendQuestions,
		emailHref,
		parseEmailQuery,
		phoneContacts,
		phoneLabel,
		questionLines,
		toE164
	} from '#lib/admin/outreach-client.js';
	import {
		defaultScript,
		dialBody,
		dialCall,
		e164Valid,
		fetchCalls
	} from '#lib/admin/calls-client.js';
	import { browserPhone } from '#lib/admin/browser-phone.svelte.js';
	import { fakeSlugs } from '#lib/admin/fake-client.js';
	import HeadsetIcon from '@lucide/svelte/icons/headset';
	import PhoneOutgoingIcon from '@lucide/svelte/icons/phone-outgoing';
	import PhoneIcon from '@lucide/svelte/icons/phone';
	import MailIcon from '@lucide/svelte/icons/mail';
	import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';

	interface CallSummary {
		call_id: string;
		status: string;
		authority: string | null;
		started_at: number;
		ended_at: number | null;
		recording_status: string;
		questions_asked: number;
		questions_total: number;
	}

	// Voice calls (voice-telnyx.md §7), First source: pick the office,
	// add what its facts still leave open (ranked by the server from
	// the fact store), dial. The dial contract is unchanged: the
	// questions box is the source of truth. The admin API keeps the
	// /apostille prefix on the admin origin, so the root is literal.
	const api = () => `/apostille/api/v1`;

	let calls = $state<CallSummary[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);

	let to = $state('');
	let authority = $state('');
	let questionsText = $state('');
	let consent = $state(false);
	let dialing = $state(false);
	let dialError = $state<string | null>(null);
	let names = $state<Record<string, string>>({});
	let rows = $state<Record<string, AuthorityOption>>({});
	let picker = $state<AuthorityPicker | null>(null);
	let reduceMotion = $state(false);
	let nowSec = $state(Math.floor(Date.now() / 1000));
	// W-3 `telnyx.mode = fake`: calls play a recorded WAV, so there is
	// no audio to carry and no microphone to ask for.
	let fakeCalls = $state(false);

	const open = new OpenQuestionsState();
	const motion = $derived(reduceMotion ? 0 : 200);
	const lines = $derived(questionLines(questionsText));
	const authorityName = $derived(names[authority] ?? (open.set?.name || authority));
	const phones = $derived(open.slug === authority ? phoneContacts(open.contacts) : []);
	const picked = $derived(rows[authority] ?? null);
	// "Email these questions instead": the script if it has lines,
	// else the strongest open questions.
	const emailQuestions = $derived(
		lines.length > 0 ? lines : open.questions.slice(0, TOP_N).map((q) => q.question)
	);
	const emailLink = $derived(emailHref(resolve('/(admin)/email'), authority, emailQuestions));

	async function load() {
		loadState = 'loading';
		loadError = null;
		try {
			calls = await fetchCalls(api());
			nowSec = Math.floor(Date.now() / 1000);
			loadState = 'ready';
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Call list failed.';
			loadState = 'failed';
		}
	}

	function syncUrl() {
		const params = new URLSearchParams(location.search);
		if (authority) params.set('authority', authority);
		else params.delete('authority');
		const query = params.toString();
		history.replaceState(history.state, '', query ? `?${query}` : location.pathname);
	}

	function pick(slug: string) {
		authority = slug;
		open.select(slug);
		syncUrl();
	}

	function onLoaded(list: AuthorityOption[]) {
		names = Object.fromEntries(list.map((r) => [r.slug, r.name]));
		rows = Object.fromEntries(list.map((r) => [r.slug, r]));
	}

	const plural = (n: number, one: string) => `${n} ${one}${n === 1 ? '' : 's'}`;

	onMount(() => {
		reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		const wanted = parseEmailQuery(location.search).authority;
		if (wanted) pick(wanted);
		void load();
		void fetch(`${api()}/admin/version`, { credentials: 'same-origin' })
			.then((res) => (res.ok ? res.json() : null))
			.then((data) => (fakeCalls = fakeSlugs(data?.fakes).includes('telnyx')))
			.catch(() => {});
		return () => open.dispose();
	});

	function add(newLines: string[]) {
		const out = appendQuestions(questionsText, newLines);
		questionsText = out.text;
		if (out.full) toast.info(`The call script is full: ${MAX_CALL_QUESTIONS} questions at most.`);
		else if (out.added > 1) toast.success(`Added ${out.added} questions to the call script.`);
	}

	function loadScript() {
		const out = appendQuestions(questionsText, defaultScript());
		questionsText = out.text;
	}

	function tidyNumber() {
		const e164 = toE164(to);
		if (e164) to = e164;
	}

	async function dial(event: SubmitEvent) {
		event.preventDefault();
		dialError = null;
		tidyNumber();
		const body = dialBody(to.trim(), authority, lines, consent);
		if (!body) {
			dialError = !e164Valid(to.trim())
				? 'Enter the office number as E.164 (e.g. +13605550100).'
				: !consent
					? 'Confirm you will announce the recording to the callee.'
					: `Check the questions (1–${MAX_CALL_QUESTIONS} lines, each 1–280 chars).`;
			return;
		}
		dialing = true;
		try {
			// You talk through this tab: log the browser phone in first
			// (this click lets the browser ask for the microphone). If
			// it cannot connect, the server still decides — a fake call
			// needs no phone, a real one refuses with the reason.
			if (!fakeCalls) await browserPhone.connect(api()).catch(() => {});
			const out = await dialCall(api(), body);
			if (out.audio === 'browser') browserPhone.expect(out.call_id);
			await goto(`${resolve('/(admin)/calls')}/${encodeURIComponent(out.call_id)}`);
		} catch (err) {
			dialError = err instanceof Error ? err.message : 'Dial failed.';
		} finally {
			dialing = false;
		}
	}

	const STATUS_TONE: Record<string, string> = {
		completed: 'border-verified bg-verified-bg text-verified',
		in_progress: 'border-line bg-surface-2 text-accent-strong',
		ringing: 'border-line bg-surface-2 text-accent-strong',
		queued: 'border-line bg-surface-2 text-accent-strong',
		failed: 'border-error bg-error-bg text-error'
	};
	const statusTone = (s: string) => STATUS_TONE[s] ?? 'border-line bg-surface text-ink-muted';
	const statusText = (s: string) => s.replaceAll('_', ' ');
</script>

<svelte:head>
	<title>Voice calls — Admin — Apostille Handbook</title>
</svelte:head>

<div class="mt-6 flex flex-col gap-6 pb-12">
	<header class="flex flex-col gap-1">
		<Heading level={1} eyebrow="First source">Voice calls</Heading>
		<p class="m-0 max-w-prose text-sm text-ink-muted">
			Ask the office itself when its published pages leave a question open. Pick the office, add
			what its facts still need, then dial.
		</p>
	</header>

	<Card.Root>
		<Card.Content class="flex flex-col gap-4 lg:flex-row lg:items-end">
			<div class="min-w-0 flex-1 lg:max-w-md">
				<AuthorityPicker
					bind:this={picker}
					bind:value={authority}
					id="authority"
					label="Office (optional)"
					placeholder="No authority"
					clearable
					showContacts
					{onLoaded}
					onChange={pick}
				/>
			</div>
			{#if picked}
				<dl class="m-0 flex flex-wrap gap-x-6 gap-y-1 text-sm lg:pb-2" in:fly={{ y: -4, duration: motion }}>
					<div class="flex items-baseline gap-1.5">
						<dt class="sr-only">Accepted facts</dt>
						<dd class="m-0 font-semibold tabular-nums">{plural(picked.fact_count, 'accepted fact')}</dd>
					</div>
					<div class="flex items-baseline gap-1.5">
						<dt class="sr-only">Waiting in Review</dt>
						<dd class={`m-0 tabular-nums ${picked.proposed_count > 0 ? 'text-changed' : 'text-ink-muted'}`}>
							{picked.proposed_count} waiting in Review
						</dd>
					</div>
					<div class="flex items-baseline gap-1.5">
						<dt class="sr-only">Contacts</dt>
						<dd class="m-0 text-ink-muted tabular-nums">{plural(picked.contact_count, 'contact')} on file</dd>
					</div>
				</dl>
			{:else}
				<p class="m-0 text-sm text-ink-muted lg:pb-2">
					Optional: a call without an office is logged unassigned and gets no ranked questions.
				</p>
			{/if}
			{#if phones.length > 0}
				<div class="flex min-w-0 flex-col gap-1 lg:ml-auto" in:fly={{ y: -4, duration: motion }}>
					<span class="text-xs font-medium text-ink-muted">Numbers on file</span>
					<div class="flex flex-wrap gap-2">
						{#each phones as phone (phone.id)}
							<Button
								variant={to === phone.value ? 'secondary' : 'outline'}
								size="lg"
								class="font-mono"
								aria-pressed={to === phone.value}
								onclick={() => (to = phone.value)}
							>
								<PhoneIcon aria-hidden="true" />
								{phoneLabel(phone.value)}
								{#if phone.details}<span class="font-sans text-xs text-ink-muted">{phone.details}</span>{/if}
							</Button>
						{/each}
					</div>
				</div>
			{/if}
		</Card.Content>
	</Card.Root>

	<div class="grid items-start gap-6 xl:grid-cols-2">
		<OpenQuestions
			state={open}
			name={authorityName}
			script={questionsText}
			{motion}
			onAdd={add}
			onPick={() => picker?.focus()}
			onDefault={loadScript}
		/>

		<!-- Phones: with no office picked, the dial form comes first. -->
		<Card.Root class={`min-w-0 ${authority ? '' : 'order-first xl:order-none'}`}>
			<Card.Header>
				<Card.Title><h2 class="m-0 text-h3 font-semibold">Dial an office</h2></Card.Title>
				<Card.Description class="text-ink-muted">
					Washington requires every party's consent to record. Dialing is staff-only and audited.
				</Card.Description>
			</Card.Header>
			<Card.Content>
				<form class="flex flex-col gap-4" onsubmit={dial}>
					<label class="flex flex-col gap-1" for="to">
						<span class="text-xs font-medium text-ink-muted">Office number (E.164)</span>
						<Input
							id="to"
							type="tel"
							class="h-9 font-mono"
							bind:value={to}
							onblur={tidyNumber}
							placeholder="e.g. +13605550100"
							autocomplete="off"
						/>
					</label>
					<label class="flex flex-col gap-1" for="questions">
						<span class="flex items-baseline justify-between gap-2 text-xs font-medium text-ink-muted">
							Call script (one question per line)
							<span class="font-mono tabular-nums">{lines.length} / {MAX_CALL_QUESTIONS}</span>
						</span>
						<Textarea
							id="questions"
							class="min-h-32"
							bind:value={questionsText}
							placeholder="Add questions from the ranked list, or type your own."
						/>
					</label>
					<div class="flex flex-wrap gap-2">
						<Button type="button" variant="outline" onclick={loadScript}>
							<ScrollTextIcon aria-hidden="true" />
							Load the default script
						</Button>
						{#if questionsText.trim()}
							<Button type="button" variant="ghost" onclick={() => (questionsText = '')}>Clear</Button>
						{/if}
					</div>
					<div class="flex flex-wrap items-center gap-2 rounded-md border border-line px-3 py-2 text-sm" role="status">
						<HeadsetIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
						<span class="min-w-0 flex-1">
							{#if fakeCalls}
								Fake calls: a recorded answer plays, nobody's phone rings, no microphone needed.
							{:else if browserPhone.phase === 'ready'}
								Browser phone ready: you talk through this tab.
							{:else if browserPhone.phase === 'in_call'}
								Browser phone is on a call.
							{:else if browserPhone.phase === 'connecting'}
								Connecting the browser phone… (the first time takes about 5 s)
							{:else if browserPhone.phase === 'error'}
								<span class="text-error">{browserPhone.error}</span>
							{:else}
								You talk through this tab. Dial connects the browser phone and asks for the microphone.
							{/if}
						</span>
						{#if !fakeCalls && (browserPhone.phase === 'off' || browserPhone.phase === 'error')}
							<Button type="button" variant="outline" size="sm" onclick={() => browserPhone.connect(api()).catch(() => {})}>
								Connect now
							</Button>
						{/if}
					</div>
					<label class="flex items-start gap-2 text-sm" for="consent">
						<input id="consent" class="mt-1 size-4 flex-none accent-accent-strong" type="checkbox" bind:checked={consent} />
						I will announce that this call is recorded before asking anything.
					</label>
					{#if dialError}
						<Notice variant="error" heading="Not dialed.">
							<p class="m-0">{dialError}</p>
						</Notice>
					{/if}
					<div class="flex flex-col-reverse gap-2 border-t border-line pt-4 sm:flex-row sm:items-center sm:justify-between">
						{#if authority}
							<Button variant="outline" size="lg" href={emailLink}>
								<MailIcon aria-hidden="true" />
								Email these questions instead
							</Button>
						{:else}
							<span class="text-xs text-ink-muted">Pick an office to email it instead.</span>
						{/if}
						<Button type="submit" size="lg" disabled={dialing}>
							{#if dialing}
								<LoaderCircleIcon class="motion-safe:animate-spin" aria-hidden="true" />
								Dialing…
							{:else}
								<PhoneOutgoingIcon aria-hidden="true" />
								Dial
							{/if}
						</Button>
					</div>
				</form>
			</Card.Content>
		</Card.Root>
	</div>

	<section class="flex flex-col gap-3" aria-labelledby="recent">
		<h2 id="recent" class="m-0 text-h3 font-semibold">Recent calls</h2>
		{#if loadState === 'loading'}
			<div class="overflow-hidden rounded-lg border border-line bg-card" role="status" aria-label="Loading calls">
				<div class="flex flex-col divide-y divide-line">
					{#each [0, 1, 2] as i (i)}
						<div class="flex items-center gap-4 px-4 py-3">
							<Skeleton class="h-4 w-48 rounded-xs" />
							<Skeleton class="hidden h-4 flex-1 rounded-xs md:block" />
							<Skeleton class="h-6 w-24 rounded-full" />
						</div>
					{/each}
				</div>
			</div>
		{:else if loadState === 'failed'}
			<Notice variant="error" heading="Calls could not load.">
				<p class="m-0">{loadError}</p>
				<p class="mt-3 mb-0"><Button variant="outline" onclick={load}>Try again</Button></p>
			</Notice>
		{:else if calls.length === 0}
			<div class="flex flex-col items-center gap-3 rounded-lg border border-dashed border-line bg-card px-6 py-10 text-center">
				<PhoneIcon class="size-8 text-ink-muted" aria-hidden="true" />
				<p class="m-0 text-h3 font-semibold">No calls yet</p>
				<p class="m-0 max-w-prose text-sm text-ink-muted">
					Each call lands here with its questions, transcript and recording state.
				</p>
				<Button variant="outline" onclick={() => document.getElementById('to')?.focus()}>Start a call</Button>
			</div>
		{:else}
			<ul class="m-0 flex list-none flex-col divide-y divide-line overflow-hidden rounded-lg border border-line bg-card p-0">
				{#each calls as call (call.call_id)}
					<li>
						<a
							class="flex min-h-11 flex-wrap items-center gap-x-3 gap-y-1 px-4 py-3 text-ink no-underline hover:bg-muted/50 focus-visible:bg-muted/50"
							href={`${resolve('/(admin)/calls')}/${encodeURIComponent(call.call_id)}`}
						>
							<span class="min-w-0 flex-1 truncate font-medium">
								{call.authority ? (names[call.authority] ?? call.authority) : 'No authority'}
							</span>
							<span class="text-xs text-ink-muted tabular-nums">
								{call.questions_asked}/{call.questions_total} asked · recording {call.recording_status}
							</span>
							<span class={`inline-flex h-6 items-center rounded-full border px-2 text-xs font-medium capitalize ${statusTone(call.status)}`}>
								{statusText(call.status)}
							</span>
							<time class="text-xs text-ink-muted" datetime={new Date(call.started_at * 1000).toISOString()} title={dateTime(call.started_at)}>
								{ago(call.started_at, Math.max(nowSec, call.started_at))}
							</time>
							<ChevronRightIcon class="size-4 text-ink-muted" aria-hidden="true" />
						</a>
					</li>
				{/each}
			</ul>
		{/if}
	</section>
</div>
