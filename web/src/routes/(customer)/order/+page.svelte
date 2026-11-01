<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import * as Popover from '../../../lib/components/ui/popover/index.js';
	import * as Command from '../../../lib/components/ui/command/index.js';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import Flag from '../../../lib/components/brand/Flag.svelte';
	import FeeReceipt from '../../../lib/components/brand/FeeReceipt.svelte';
	import { silhouetteFor } from '../../../lib/public-ui/authority-meta.js';
	import silhouettes from '../../../lib/geo/silhouettes.json';
	import {
		listAuthorities,
		publicError
	} from '../../../lib/public-ui/handbook.js';
	import { FLAG, LANE_LABEL, laneOf, parseEta } from '../../../lib/public-ui/authority-meta.js';
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import LandmarkIcon from '@lucide/svelte/icons/landmark';
	import ClockIcon from '@lucide/svelte/icons/clock';

	let locale = $state<Locale>(resolveLocale());

	// Stepped order draft: destination → document → contact → review.
	// All state is local until a submit endpoint exists, and the
	// confirmation says so honestly.
	let step = $state(1);
	let authority = $state('');
	let docType = $state('');
	let name = $state('');
	let email = $state('');
	let street = $state('');
	let city = $state('');
	let postal = $state('');
	let done = $state(false);
	let comboOpen = $state(false);

	let index = $state<{ slug: string; name: string; country: string }[] | null>(null);
	let loadError = $state<string | null>(null);
	let destFacts = $state<any[] | null>(null);
	let templates = $state<{ id: number; code: string; title: string }[]>([]);
	let saveState = $state<'idle' | 'saving' | 'saved' | 'login' | 'error'>('idle');
	let savedId = $state<number | null>(null);

	const t = $derived(STRINGS[locale]);
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	async function get(path: string) {
		const res = await fetch(api(path), { credentials: 'same-origin' });
		const body = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body };
	}

	$effect(() => {
		listAuthorities({ get })
			.then((res) => {
				if (!res.ok) {
					loadError = publicError(res.status, 'index');
					return;
				}
				const rows = res.body?.authorities;
				index = Array.isArray(rows) ? rows : [];
				// A dossier CTA can preselect: ?destination=<slug>.
				const preset = page.url.searchParams.get('destination');
				if (preset && index.some((a) => a.slug === preset)) {
					authority = preset;
					void loadDestFacts(preset);
					void loadTemplates(preset);
				}
			})
			.catch(() => {
				loadError = publicError(500, 'index');
			});
	});

	async function loadDestFacts(slug: string) {
		destFacts = null;
		try {
			const res = await get(`/authorities/${encodeURIComponent(slug)}/facts`);
			const rows = res.body?.facts;
			destFacts = res.ok && Array.isArray(rows) ? rows : [];
		} catch {
			destFacts = [];
		}
	}

	function pick(slug: string) {
		authority = slug;
		comboOpen = false;
		saveState = 'idle';
		savedId = null;
		void loadDestFacts(slug);
		void loadTemplates(slug);
	}

	async function loadTemplates(slug: string) {
		templates = [];
		try {
			const res = await fetch(api(`/p2/forms?authority=${encodeURIComponent(slug)}`), {
				credentials: 'same-origin'
			});
			const body = await res.json().catch(() => ({}));
			templates = res.ok && Array.isArray(body?.templates) ? body.templates : [];
		} catch {
			templates = [];
		}
	}

	async function saveDraft() {
		if (templates.length === 0 || saveState === 'saving') return;
		saveState = 'saving';
		try {
			const res = await fetch(api('/p2/submissions'), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					template_id: templates[0].id,
					draft: true,
					values: {
						full_name: name.trim(),
						email: email.trim(),
						street: street.trim(),
						city: city.trim(),
						postal_code: postal.trim(),
						document_type: docType,
						destination: authority
					}
				})
			});
			if (res.status === 401) {
				saveState = 'login';
				return;
			}
			const body = await res.json().catch(() => ({}));
			if (res.status === 201 && typeof body?.id === 'number') {
				savedId = body.id;
				saveState = 'saved';
				return;
			}
			saveState = 'error';
		} catch {
			saveState = 'error';
		}
	}

	const STEPS = ['Destination', 'Document', 'Contact', 'Review'];
	const progress = $derived(done ? 1 : (step - 1) / (STEPS.length - 1));

	const authorityName = $derived(
		index?.find((a) => a.slug === authority)?.name ?? authority
	);
	const canNext1 = $derived(authority !== '');
	const canNext2 = $derived(docType !== '');
	const canNext3 = $derived(
		name.trim() !== '' && /.+@.+\..+/.test(email.trim()) && street.trim() !== ''
	);
	const missing1 = $derived(!canNext1 ? 'Choose a destination to continue.' : '');
	const missing2 = $derived(!canNext2 ? 'Choose a document type to continue.' : '');
	const missing3 = $derived(
		!canNext3 ? 'Add your name, a valid email and a street address to continue.' : ''
	);

	type Destination = { slug: string; name: string; country: string };
	const grouped = $derived.by(() => {
		const out: { lane: keyof typeof LANE_LABEL; rows: Destination[] }[] = [
			{ lane: 'hague', rows: [] },
			{ lane: 'legalization', rows: [] },
			{ lane: 'states', rows: [] }
		];
		for (const a of index ?? []) out.find((g) => g.lane === laneOf(a.slug))?.rows.push(a);
		return out.filter((g) => g.rows.length > 0);
	});

	const origin = silhouettes.washington;
	const dest = $derived(silhouetteFor(authority));
	const eta = $derived(destFacts ? parseEta(destFacts) : '…');

	const DOCS = ['Birth certificate', 'Marriage certificate', 'Corporate document', 'Other'];
</script>

<svelte:head>
	<title>{t.orderLabel} — Apostille Handbook</title>
</svelte:head>

<Heading level={1}>{t.orderLabel}</Heading>

{#if loadError}
	<Notice variant="error" heading="The order form is unavailable."><p>{loadError}</p></Notice>
{:else if index === null}
	<div role="status" aria-label="Loading order form" class="flex max-w-[32rem] flex-col gap-2">
		<span class="sr-only">Loading order form</span>
		<Skeleton class="h-11 w-full" />
		<Skeleton class="h-11 w-full" />
		<Skeleton class="h-11 w-2/3" />
	</div>
{:else if index.length === 0}
	<Notice variant="info" heading="No destinations yet.">
		<p>
			The handbook has no published authorities, so there is nothing to order yet.
			<a href={resolve('/(public)')}>Back to the authority index</a>.
		</p>
	</Notice>
{:else}
	<div class="grid gap-8 py-10 lg:grid-cols-12 xl:py-16">
		<div class="lg:col-span-7">
			<ol
				class="relative m-0 flex list-none flex-col gap-4 p-0 sm:flex-row sm:items-start sm:gap-0"
				aria-label="Order progress"
			>
				<span
					aria-hidden="true"
					class="absolute top-2 bottom-2 left-[13px] w-0.5 bg-surface-2 sm:top-[13px] sm:right-0 sm:bottom-auto sm:left-0 sm:h-0.5 sm:w-auto"
				></span>
				<span
					aria-hidden="true"
					class="absolute top-2 bottom-2 left-[13px] w-0.5 origin-top scale-y-(--p) bg-accent-fill transition-transform duration-300 sm:top-[13px] sm:right-6 sm:bottom-auto sm:left-6 sm:h-0.5 sm:w-auto sm:origin-left sm:scale-x-(--p) sm:scale-y-100"
					style={`--p: ${progress};`}
				></span>
				{#each STEPS as label, i (label)}
					{@const n = i + 1}
					{@const isCurrent = step === n && !done}
					{@const isPast = step > n || done}
					<li
						class="relative flex items-center gap-2 bg-bg pr-2 sm:flex-1 sm:flex-col sm:items-start sm:gap-1 sm:pt-8"
						aria-current={isCurrent ? 'step' : undefined}
					>
						<span
							aria-hidden="true"
							class={`flex size-7 flex-none items-center justify-center rounded-full border text-sm font-bold sm:absolute sm:top-0 sm:left-0 ${
								isCurrent
									? 'border-accent-fill bg-accent-fill text-accent-ink'
									: isPast
										? 'border-verified bg-verified text-white'
										: 'border-line bg-bg text-ink-muted'
							}`}
						>
							{#if isPast}
								<CheckIcon class="size-4" />
							{:else}
								{n}
							{/if}
						</span>
						<span
							class={`text-sm ${isCurrent ? 'font-bold text-ink' : isPast ? 'text-ink' : 'text-ink-muted'}`}
						>
							{label}
						</span>
					</li>
				{/each}
			</ol>

			{#if done}
				<Notice variant="success" heading="Order drafted.">
					<p>
						{authorityName} · {docType} · {name}. Nothing was sent — sign-in opens
						order submission.
						<a href={resolve('/(customer)/login')}>Continue to {t.loginLabel}</a>
					</p>
				</Notice>
				<p>
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink"
						onclick={() => (done = false)}>Back to review</button
					>
				</p>
			{:else if step === 1}
				<form
					class="mt-8 flex max-w-[32rem] flex-col gap-2"
					onsubmit={(e) => {
						e.preventDefault();
						if (canNext1) step = 2;
					}}
				>
					<span class="mt-2 font-bold" id="dest-label">Destination authority</span>
					<Popover.Root bind:open={comboOpen}>
						<Popover.Trigger
							role="combobox"
							aria-labelledby="dest-label"
							aria-expanded={comboOpen}
							class="field flex cursor-pointer items-center justify-between gap-2 text-left"
						>
							<span class={authority ? 'text-ink' : 'text-ink-muted'}>
								{authorityName || 'Choose an authority…'}
							</span>
							<ChevronDownIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
						</Popover.Trigger>
						<Popover.Content class="w-80 max-w-[calc(100vw-2rem)] p-0" align="start">
							<Command.Root>
								<Command.Input placeholder="Search destinations…" />
								<Command.List>
									<Command.Empty>No destination matches.</Command.Empty>
									{#each grouped as g (g.lane)}
										<Command.Group heading={LANE_LABEL[g.lane]}>
											{#each g.rows as a (a.slug)}
												<Command.Item
													value={`${a.name} ${a.country} ${a.slug}`}
													onSelect={() => pick(a.slug)}
												>
													{#if FLAG[a.slug]}
														<Flag code={FLAG[a.slug]} label={`${a.country} flag`} />
													{:else}
														<LandmarkIcon
															class="size-5 flex-none text-ink-muted"
															aria-hidden="true"
														/>
													{/if}
													<span class="truncate">{a.name}</span>
												</Command.Item>
											{/each}
										</Command.Group>
									{/each}
								</Command.List>
							</Command.Root>
						</Popover.Content>
					</Popover.Root>
					<p>
						<button
							class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed"
							type="submit"
							disabled={!canNext1}>Continue</button
						>
					</p>
					{#if missing1}
						<p id="missing-1" class="m-0 text-sm text-ink-muted">{missing1}</p>
					{/if}
				</form>
			{:else if step === 2}
				<form
					class="mt-8 flex max-w-[32rem] flex-col gap-2"
					onsubmit={(e) => {
						e.preventDefault();
						if (canNext2) step = 3;
					}}
				>
					<fieldset class="mx-0 mt-2 mb-0 rounded-md border border-line px-4 py-3">
						<legend class="px-1 font-bold">Document type — {authorityName}</legend>
						{#each DOCS as d (d)}
							<label class="flex min-h-11 items-center gap-2">
								<input
									class="h-5 w-5 accent-accent"
									type="radio"
									name="doc_type"
									value={d}
									bind:group={docType}
									required
								/>
								{d}
							</label>
						{/each}
					</fieldset>
					<p class="flex gap-3">
						<button
							class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink"
							type="button"
							onclick={() => (step = 1)}>Back</button
						>
						<button
							class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed"
							type="submit"
							disabled={!canNext2}>Continue</button
						>
					</p>
					{#if missing2}
						<p id="missing-2" class="m-0 text-sm text-ink-muted">{missing2}</p>
					{/if}
				</form>
			{:else if step === 3}
				<form
					class="mt-8 flex max-w-[32rem] flex-col gap-2"
					onsubmit={(e) => {
						e.preventDefault();
						if (canNext3) step = 4;
					}}
				>
					<label class="mt-2 font-bold" for="name">Full name</label>
					<input class="field" id="name" type="text" bind:value={name} required autocomplete="name" />
					<label class="mt-2 font-bold" for="email">Email</label>
					<input
						class="field"
						id="email"
						type="email"
						bind:value={email}
						required
						autocomplete="email"
					/>
					<label class="mt-2 font-bold" for="street">Street address</label>
					<input
						class="field"
						id="street"
						type="text"
						bind:value={street}
						required
						autocomplete="street-address"
					/>
					<div class="grid grid-cols-2 gap-x-4 gap-y-0">
						<p class="flex flex-col gap-2">
							<label for="city">City</label>
							<input
								class="field"
								id="city"
								type="text"
								bind:value={city}
								autocomplete="address-level2"
							/>
						</p>
						<p class="flex flex-col gap-2">
							<label for="postal">Postal code</label>
							<input
								class="field"
								id="postal"
								type="text"
								bind:value={postal}
								autocomplete="postal-code"
							/>
						</p>
					</div>
					<p class="flex gap-3">
						<button
							class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink"
							type="button"
							onclick={() => (step = 2)}>Back</button
						>
						<button
							class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink disabled:cursor-not-allowed"
							type="submit"
							disabled={!canNext3}>Review order</button
						>
					</p>
					{#if missing3}
						<p id="missing-3" class="m-0 text-sm text-ink-muted">{missing3}</p>
					{/if}
				</form>
			{:else}
				<dl
					class="mt-8 grid max-w-[32rem] grid-cols-[auto_1fr] gap-x-4 gap-y-2 rounded-md border border-line bg-surface p-4"
				>
					<dt class="font-bold">Destination</dt>
					<dd class="m-0">{authorityName}</dd>
					<dt class="font-bold">Document</dt>
					<dd class="m-0">{docType}</dd>
					<dt class="font-bold">Name</dt>
					<dd class="m-0">{name}</dd>
					<dt class="font-bold">Email</dt>
					<dd class="m-0">{email}</dd>
					<dt class="font-bold">Address</dt>
					<dd class="m-0">{street}{city ? `, ${city}` : ''}{postal ? ` ${postal}` : ''}</dd>
				</dl>
				<p class="flex gap-3">
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-5 font-bold text-ink"
						onclick={() => (step = 3)}>Back</button
					>
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink"
						onclick={() => (done = true)}>Draft order</button
					>
				</p>
				{#if templates.length > 0 && !done}
					<div class="mt-4 max-w-[32rem] rounded-md border border-line bg-surface p-4">
						{#if saveState === 'saved' && savedId !== null}
							<p class="m-0 font-bold">Draft saved as order #{savedId}.</p>
							<p class="mt-2 mb-0 text-sm">
								<a href={resolve('/(customer)/orders')}>See it in your orders</a>.
							</p>
						{:else}
							<p class="m-0 text-sm text-ink-muted">
								{templates[0].title} is available for {authorityName} — keep this
								draft in your account.
							</p>
							<p class="mt-3 mb-0 flex gap-3">
								<button
									class="min-h-11 cursor-pointer rounded-sm border border-line bg-bg px-5 font-bold text-ink disabled:cursor-not-allowed"
									disabled={saveState === 'saving'}
									onclick={saveDraft}
								>
									{saveState === 'saving' ? 'Saving…' : 'Save draft to my orders'}
								</button>
							</p>
							{#if saveState === 'login'}
								<p class="mt-2 mb-0 text-sm">
									Sign-in opens order submission.
									<a href={resolve('/(customer)/login')}>Continue to {t.loginLabel}</a>
								</p>
							{:else if saveState === 'error'}
								<p class="mt-2 mb-0 text-sm" role="alert">
									Could not save the draft. Check your connection and try again.
								</p>
							{/if}
						{/if}
					</div>
				{/if}
			{/if}
		</div>

		<div class="lg:col-span-5">
			<aside
				class="rounded-xl border border-line bg-bg p-5 lg:sticky lg:top-20"
				aria-label="Your apostille"
			>
				<h2 class="text-h3 m-0 font-semibold">Your apostille</h2>
				<svg
					viewBox="0 0 460 140"
					class="mt-4 w-full"
					role="img"
					aria-label={authority
						? `Route from Washington to ${authorityName}`
						: 'Route from Washington — choose a destination'}
					data-graphic="route"
				>
					<g>
						<path d={origin.d} style="fill: var(--surface-2);" transform="translate(0,0)" />
						<text x="100" y="132" text-anchor="middle" font-size="11" style="fill: var(--color-ink-muted); font-family: var(--font-sans);">
							Washington
						</text>
					</g>
					<path
						d="M205,60 Q230,20 255,60"
						fill="none"
						style="stroke: var(--color-brand-500);"
						stroke-width="1.5"
						stroke-dasharray="4 3"
					/>
					{#if dest}
						<g>
							<path d={dest.d} style="fill: var(--color-brand-300);" transform="translate(260,0)" />
							<text x="360" y="132" text-anchor="middle" font-size="11" style="fill: var(--color-ink-muted); font-family: var(--font-sans);">
								{authorityName.slice(0, 24)}
							</text>
						</g>
					{:else}
						<g>
							<rect
								x="280"
								y="20"
								width="160"
								height="100"
								rx="12"
								fill="none"
								style="stroke: var(--border);"
								stroke-width="1.5"
								stroke-dasharray="5 4"
							/>
							<text x="360" y="132" text-anchor="middle" font-size="11" style="fill: var(--color-ink-muted); font-family: var(--font-sans);">
								Choose a destination
							</text>
						</g>
					{/if}
				</svg>
				{#if !authority}
					<div role="status" aria-label="Waiting for your choices" class="mt-4 flex flex-col gap-2">
						<span class="sr-only">Choose a destination to preview fees and turnaround</span>
						<Skeleton class="h-4 w-2/3" />
						<Skeleton class="h-16 w-full" />
						<Skeleton class="h-4 w-1/2" />
					</div>
				{:else}
					<div class="mt-4 flex items-center gap-3 rounded-lg border border-line p-3">
						<svg
							viewBox="0 0 48 60"
							class="h-14 w-auto flex-none"
							role="img"
							aria-label={docType || 'Document'}
							data-graphic="document"
						>
							<path
								d="M8,2 H30 L42,14 V58 H8 Z"
								style="fill: var(--surface); stroke: var(--border);"
								stroke-width="1.5"
							/>
							<path d="M30,2 V14 H42" style="fill: none; stroke: var(--border);" stroke-width="1.5" />
							<g style="stroke: var(--border);" stroke-width="2">
								<line x1="14" y1="24" x2="36" y2="24" />
								<line x1="14" y1="30" x2="36" y2="30" />
								<line x1="14" y1="36" x2="30" y2="36" />
							</g>
							<circle cx="32" cy="46" r="7" style="fill: none; stroke: var(--color-brand-500);" stroke-width="1.5" />
						</svg>
						<p class="m-0 text-sm text-ink">
							{docType || 'Choose a document type in step 2.'}
						</p>
					</div>
					<div class="mt-4">
						<FeeReceipt facts={destFacts ?? []} />
					</div>
					<p class="mt-4 mb-0 flex min-h-11 items-center gap-2 text-sm text-ink-muted">
						<ClockIcon class="size-4 flex-none" aria-hidden="true" />
						<span>Estimated turnaround: <strong class="text-ink">{eta}</strong></span>
					</p>
				{/if}
			</aside>
		</div>
	</div>
{/if}

<noscript>
	<p class="rounded-md border border-line bg-surface px-4 py-3">
		Placing an order needs JavaScript for now; field validation above still works without it.
	</p>
</noscript>
