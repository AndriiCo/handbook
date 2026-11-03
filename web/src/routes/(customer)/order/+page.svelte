<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { fly } from 'svelte/transition';
	import { prefersReducedMotion } from 'svelte/motion';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { toast } from 'svelte-sonner';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Skeleton } from '../../../lib/components/ui/skeleton/index.js';
	import { Button } from '../../../lib/components/ui/button/index.js';
	import { Input } from '../../../lib/components/ui/input/index.js';
	import { Badge } from '../../../lib/components/ui/badge/index.js';
	import * as Popover from '../../../lib/components/ui/popover/index.js';
	import * as Command from '../../../lib/components/ui/command/index.js';
	import Heading from '../../../lib/components/brand/Heading.svelte';
	import OrderStepper from '../../../lib/customer/OrderStepper.svelte';
	import DeliveryCard from '../../../lib/customer/DeliveryCard.svelte';
	import FactChip from '../../../lib/customer/FactChip.svelte';
	import JourneyGraphic from '../../../lib/customer/JourneyGraphic.svelte';
	import {
		carrierName,
		getJson,
		humanize,
		isPaused,
		loginHref,
		money,
		postJson,
		probeSignedIn,
		safeHref,
		serviceName,
		fmtTime,
		type DeliveryOption,
		type Fact,
		type FormTemplate,
		type LegRole,
		type Order,
		type OrderKind,
		type OrderOptions,
		type PlanBundle,
		type PlanPath,
		type Quote
	} from '../../../lib/customer/orders';
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';
	import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
	import LandmarkIcon from '@lucide/svelte/icons/landmark';
	import FileTextIcon from '@lucide/svelte/icons/file-text';
	import LogInIcon from '@lucide/svelte/icons/log-in';
	import LockIcon from '@lucide/svelte/icons/lock';
	import ArrowRightIcon from '@lucide/svelte/icons/arrow-right';
	import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';
	import LoaderIcon from '@lucide/svelte/icons/loader-circle';
	import ExternalLinkIcon from '@lucide/svelte/icons/external-link';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

	// W3a: Destination → Document → Form → Delivery → Pay. Paths come
	// from the accepted plan, the form fields from the accepted form
	// map, delivery options and prices from /p2/orders/options, the
	// quote from the server. Sign-in is required at Delivery.
	const STEPS = [
		{ id: 'destination', label: 'Destination' },
		{ id: 'document', label: 'Document' },
		{ id: 'form', label: 'Form' },
		{ id: 'delivery', label: 'Delivery' },
		{ id: 'pay', label: 'Pay' }
	] as const;
	type StepId = (typeof STEPS)[number]['id'];
	const GENERAL = 'general';
	const KINDS: OrderKind[] = ['label_fedex', 'label_usps', 'mail_kit', 'address_label'];
	const RESUMABLE = ['draft', 'quoted', 'awaiting_payment'];

	type Authority = { slug: string; name: string; country: string; region?: string };

	let locale = $state<Locale>(resolveLocale());
	const t = $derived(STRINGS[locale]);

	let step = $state(0);
	let slug = $state('');
	let pathId = $state<string | null>(null);
	let kind = $state<OrderKind | null>(null);
	let values = $state<Record<string, string | boolean>>({});
	let formSkipped = $state(false);
	let address = $state({
		name: '',
		line1: '',
		line2: '',
		city: '',
		region: '',
		postal_code: '',
		country: ''
	});

	let index = $state<Authority[] | null>(null);
	let loadError = $state<string | null>(null);
	let signedIn = $state<boolean | null>(null);
	let bundle = $state<PlanBundle | null>(null);
	let bundleReady = $state(false);
	let facts = $state<Fact[]>([]);
	let options = $state<OrderOptions | null>(null);
	let optionsState = $state<'idle' | 'loading' | 'ready' | 'error'>('idle');
	let comboOpen = $state(false);

	let order = $state<Order | null>(null);
	let quote = $state<Quote | null>(null);
	let busy = $state<null | 'order' | 'quote' | 'checkout' | 'free'>(null);
	let paused = $state<string | null>(null);
	let payError = $state<string | null>(null);
	let stepHeading = $state<HTMLElement | null>(null);
	let stepperEl = $state<HTMLElement | null>(null);

	const enc = encodeURIComponent;
	const motion = (n: number) => (prefersReducedMotion.current ? 0 : n);

	// ------------------------------------------------------------ derived

	const authorityName = $derived(index?.find((a) => a.slug === slug)?.name ?? slug);
	const plan = $derived(bundle?.plan ?? null);
	const planPaths = $derived<PlanPath[]>(Array.isArray(plan?.paths) ? plan.paths : []);
	const docOptions = $derived<PlanPath[]>(
		planPaths.length > 0 ? planPaths : [{ id: GENERAL, label: 'Public document', fact_ids: [] }]
	);
	const pathLabel = $derived(docOptions.find((p) => p.id === pathId)?.label ?? '');
	const allFacts = $derived<Fact[]>([...(bundle?.facts ?? []), ...facts]);
	const factById = (id: number | null | undefined): Fact | null =>
		id === null || id === undefined ? null : (allFacts.find((f) => f.id === id) ?? null);
	const topics = $derived.by(() => {
		const seen = new Map<string, number>();
		for (const f of facts) {
			if (f.status && f.status !== 'accepted') continue;
			seen.set(f.topic, (seen.get(f.topic) ?? 0) + 1);
		}
		return [...seen.entries()];
	});

	const shows = (showFor: string[] | undefined) =>
		!Array.isArray(showFor) || showFor.length === 0 || (pathId !== null && showFor.includes(pathId));
	const callouts = (id: string) =>
		(plan?.blocks ?? []).filter(
			(b) => b.kind === 'callout' && Array.isArray(b.show_for) && b.show_for.includes(id)
		);
	const feeLines = $derived(
		(plan?.blocks ?? [])
			.filter((b) => b.kind === 'fees' && shows(b.show_for))
			.flatMap((b) => b.lines ?? [])
	);
	const timing = $derived((plan?.blocks ?? []).find((b) => b.kind === 'timing' && shows(b.show_for)) ?? null);

	const form = $derived.by((): FormTemplate | null => {
		const forms = (bundle?.forms ?? []).filter((f) => f.fillable && Array.isArray(f.fields));
		if (forms.length === 0) return null;
		const wanted = (plan?.blocks ?? [])
			.filter((b) => b.kind === 'forms' && shows(b.show_for))
			.flatMap((b) => b.form_ids ?? []);
		return forms.find((f) => wanted.includes(f.id)) ?? forms[0];
	});
	const filled = (v: string | boolean | undefined) =>
		typeof v === 'boolean' ? v : typeof v === 'string' && v.trim() !== '';
	const missingFields = $derived(
		form ? form.fields.filter((f) => f.required && f.type !== 'checkbox' && !filled(values[f.name])) : []
	);
	const filledCount = $derived(form ? form.fields.filter((f) => filled(values[f.name])).length : 0);

	const currency = $derived(quote?.currency ?? options?.currency ?? order?.amount?.currency ?? null);
	const selected = $derived<DeliveryOption | null>(
		options?.options.find((o) => o.kind === kind && o.enabled) ?? null
	);
	const addressOk = $derived(
		address.name.trim() !== '' &&
			address.line1.trim() !== '' &&
			address.city.trim() !== '' &&
			address.postal_code.trim() !== '' &&
			/^[A-Za-z]{2}$/.test(address.country.trim())
	);
	const carriers = $derived.by(() => {
		const out: Partial<Record<LegRole, string>> = {};
		for (const q of quote?.carrier_quotes ?? []) out[q.leg] = carrierName(q.carrier);
		for (const l of order?.legs ?? []) out[l.role] ??= carrierName(l.carrier);
		return out;
	});
	const payLines = $derived(quote?.lines ?? order?.amount?.lines ?? []);
	const payTotal = $derived(quote?.total_cents ?? order?.amount?.total_cents ?? null);
	const mode = $derived(quote?.mode ?? order?.mode ?? options?.mode ?? null);

	const nextHref = $derived(
		loginHref(
			`${resolve('/(customer)/order')}?${new URLSearchParams({
				destination: slug,
				...(pathId ? { path: pathId } : {}),
				...(kind ? { kind } : {}),
				step: 'delivery'
			}).toString()}`
		)
	);

	type Destination = Authority;
	// One group per country, so the combobox scans like an atlas index.
	const grouped = $derived.by(() => {
		const out = new Map<string, Destination[]>();
		for (const a of index ?? []) {
			const key = a.country || 'Other';
			out.set(key, [...(out.get(key) ?? []), a]);
		}
		return [...out.entries()].sort((x, y) => x[0].localeCompare(y[0]));
	});

	// ------------------------------------------------------------ loading

	async function loadDestination(s: string) {
		bundleReady = false;
		bundle = null;
		facts = [];
		const [p, f] = await Promise.all([
			getJson(`/authorities/${enc(s)}/plan`),
			getJson(`/authorities/${enc(s)}/facts`)
		]);
		if (slug !== s) return;
		const body = p.ok ? p.body : null;
		bundle = body
			? {
					slug: s,
					plan: body.plan && typeof body.plan === 'object' ? body.plan : null,
					facts: Array.isArray(body.facts) ? body.facts : [],
					forms: Array.isArray(body.forms) ? body.forms : []
				}
			: null;
		facts = f.ok && Array.isArray(f.body?.facts) ? f.body.facts : [];
		bundleReady = true;
		restoreValues();
	}

	async function loadOptions(s: string) {
		if (optionsState === 'loading' || (optionsState === 'ready' && options?.slug === s)) return;
		optionsState = 'loading';
		const r = await getJson<OrderOptions>(`/p2/orders/options?slug=${enc(s)}`);
		if (slug !== s) return;
		if (r.ok && Array.isArray(r.body?.options)) {
			options = r.body;
			optionsState = 'ready';
			if (kind && !r.body.options.some((o) => o.kind === kind && o.enabled)) kind = null;
		} else {
			options = null;
			optionsState = 'error';
		}
	}

	const storeKey = () => (form ? `hb_order_form:${slug}:${form.id}` : null);
	function restoreValues() {
		const key = storeKey();
		if (!key) return;
		try {
			const raw = sessionStorage.getItem(key);
			if (raw) values = { ...JSON.parse(raw), ...values };
		} catch {
			// Private mode or blocked storage: the form still works.
		}
	}
	$effect(() => {
		const key = storeKey();
		const snapshot = JSON.stringify(values);
		if (!key || snapshot === '{}') return;
		try {
			sessionStorage.setItem(key, snapshot);
		} catch {
			// Best effort only.
		}
	});

	onMount(() => {
		const params = page.url.searchParams;
		void (async () => {
			const [idx, who] = await Promise.all([getJson('/authorities'), probeSignedIn()]);
			signedIn = who;
			if (!idx.ok) {
				loadError = 'The authority index is unavailable right now. Try again in a minute.';
				index = [];
				return;
			}
			index = Array.isArray(idx.body?.authorities) ? idx.body.authorities : [];

			let preset = params.get('destination') ?? '';
			const orderId = params.get('order');
			if (orderId && who) {
				const r = await getJson<Order>(`/p2/orders/${enc(orderId)}`);
				if (r.ok && typeof r.body?.id === 'string') {
					if (!RESUMABLE.includes(r.body.status)) {
						await goto(resolve('/(customer)/orders/[id]', { id: r.body.id }));
						return;
					}
					order = r.body;
					kind = r.body.kind;
					preset = r.body.authority?.slug ?? preset;
					pathId = r.body.path_id ?? GENERAL;
				}
			}
			if (params.get('checkout') === 'cancel') {
				toast.info('Checkout closed. Your order is saved; pay when you are ready.');
			}
			if (!preset || !(index ?? []).some((a) => a.slug === preset)) return;
			slug = preset;
			await loadDestination(preset);
			const p = params.get('path');
			if (!pathId && p && docOptions.some((o) => o.id === p)) pathId = p;
			const k = params.get('kind') as OrderKind | null;
			if (!kind && k && KINDS.includes(k)) kind = k;
			const want = STEPS.findIndex((s) => s.id === (params.get('step') as StepId));
			const limit = order ? 4 : pathId ? 3 : 1;
			step = Math.max(0, Math.min(want < 0 ? 1 : want, limit));
			if (step >= 3) void loadOptions(preset);
		})();
	});

	// ------------------------------------------------------------ actions

	async function go(i: number) {
		step = i;
		if (i >= 3 && slug) void loadOptions(slug);
		const u = new URL(window.location.href);
		u.search = '';
		if (slug) u.searchParams.set('destination', slug);
		if (pathId && i >= 2) u.searchParams.set('path', pathId);
		if (kind && i >= 3) u.searchParams.set('kind', kind);
		if (order && i === 4) u.searchParams.set('order', order.id);
		u.searchParams.set('step', STEPS[i].id);
		try {
			await goto(u, { shallow: true, replace: true });
		} catch {
			// URL mirroring is a convenience; the step already moved.
		}
		await tick();
		// Focus the new step for screen readers; scroll only when the
		// stepper went above the sticky header (long steps on 390 px).
		stepHeading?.focus({ preventScroll: true });
		if ((stepperEl?.getBoundingClientRect().top ?? 0) < 56) {
			stepperEl?.scrollIntoView({ block: 'start', behavior: prefersReducedMotion.current ? 'auto' : 'smooth' });
		}
	}

	function pick(s: string) {
		comboOpen = false;
		if (s === slug) return;
		slug = s;
		pathId = null;
		kind = null;
		order = null;
		quote = null;
		paused = null;
		payError = null;
		values = {};
		formSkipped = false;
		options = null;
		optionsState = 'idle';
		void loadDestination(s);
	}

	function choosePath(id: string) {
		if (pathId !== id) {
			pathId = id;
			order = null;
			quote = null;
		}
	}

	function chooseKind(k: OrderKind) {
		if (kind !== k) {
			kind = k;
			order = null;
			quote = null;
			paused = null;
			payError = null;
		}
	}

	function returnTo() {
		const a = address;
		return {
			name: a.name.trim(),
			line1: a.line1.trim(),
			line2: a.line2.trim() || null,
			city: a.city.trim(),
			region: a.region.trim(),
			postal_code: a.postal_code.trim(),
			country: a.country.trim().toUpperCase()
		};
	}

	async function placeOrder(k: OrderKind) {
		payError = null;
		paused = null;
		busy = k === 'address_label' ? 'free' : 'order';
		let fillId: string | null = null;
		if (form && !formSkipped && filledCount > 0) {
			const f = await postJson(`/p2/forms/${form.id}/fill`, { values: { ...values } });
			if (f.ok && typeof f.body?.fill_id === 'string') fillId = f.body.fill_id;
		}
		const r = await postJson<Order>('/p2/orders', {
			slug,
			kind: k,
			path_id: pathId === GENERAL ? null : pathId,
			form_fill_id: fillId,
			return_to: returnTo()
		});
		if (r.status === 401) {
			signedIn = false;
			busy = null;
			return;
		}
		if (isPaused(r)) {
			paused = String((r.body as { reason?: string })?.reason ?? '');
			busy = null;
			await go(4);
			return;
		}
		if (!r.ok || typeof r.body?.id !== 'string') {
			payError = 'We could not create the order. Check your connection and try again.';
			busy = null;
			return;
		}
		order = r.body;
		if (k === 'address_label') {
			toast.success('Free address label created.');
			await goto(resolve('/(customer)/orders/[id]', { id: r.body.id }));
			return;
		}
		await requote();
	}

	async function requote() {
		if (!order) return;
		busy = 'quote';
		payError = null;
		const q = await postJson<Quote>(`/p2/orders/${enc(order.id)}/quote`, {});
		busy = null;
		if (isPaused(q)) {
			paused = String((q.body as { reason?: string })?.reason ?? '');
		} else if (q.ok && Array.isArray(q.body?.lines)) {
			quote = q.body;
		} else {
			payError = 'The quote is unavailable right now. Try again in a moment.';
		}
		await go(4);
	}

	async function pay() {
		if (!order) return;
		busy = 'checkout';
		payError = null;
		const r = await postJson(`/p2/orders/${enc(order.id)}/checkout`, {});
		if (isPaused(r)) {
			paused = String((r.body as { reason?: string })?.reason ?? '');
			busy = null;
			return;
		}
		const url = safeHref(r.body?.checkout_url);
		if (r.ok && url) {
			// Keep the button busy: the browser is leaving for Checkout.
			window.location.assign(url);
			return;
		}
		busy = null;
		payError = 'Checkout did not open. Your order is saved; try again in a moment.';
	}

	async function useFree() {
		chooseKind('address_label');
		await placeOrder('address_label');
	}

	const freeOption = $derived(
		options?.options.find((o) => o.kind === 'address_label' && o.enabled) ?? null
	);
	const fieldId = (name: string) => `field-${name.replace(/[^A-Za-z0-9_-]/g, '-')}`;
</script>

<svelte:head>
	<title>{t.orderLabel} — Apostille Handbook</title>
</svelte:head>

<div class="flex flex-col gap-2">
	<Heading level={1} eyebrow="Apostille order">{t.orderLabel}</Heading>
	<p class="m-0 max-w-[60ch] text-body-lg text-ink-muted">
		Pick where the document is going and how it travels. Every option and price below comes from the
		authority's accepted facts.
	</p>
</div>

{#if loadError}
	<div class="mt-8">
		<Notice variant="error" heading="The order form is unavailable."><p>{loadError}</p></Notice>
	</div>
{:else if index === null}
	<!-- The stepper renders before the index arrives, so a slow first
	     read never hides where the visitor is (GP4 reads aria-current). -->
	<div class="pt-10 lg:w-7/12">
		<OrderStepper steps={STEPS} current={step} onpick={() => {}} />
	</div>
	<div role="status" aria-label="Loading order form" class="mt-8 flex max-w-lg flex-col gap-3">
		<span class="sr-only">Loading order form</span>
		<Skeleton class="h-7 w-full" />
		<Skeleton class="h-11 w-full" />
		<Skeleton class="h-11 w-2/3" />
	</div>
{:else if index.length === 0}
	<div class="mt-8">
		<Notice variant="info" heading="No destinations yet.">
			<p>
				The handbook has no published authorities, so there is nothing to order yet.
				<a class="underline underline-offset-4" href={resolve('/(public)')}>Back to the authority index</a>.
			</p>
		</Notice>
	</div>
{:else}
	<div class="grid gap-8 py-10 lg:grid-cols-12 xl:py-16">
		<div class="min-w-0 lg:col-span-7">
			<div bind:this={stepperEl} class="scroll-mt-20">
				<OrderStepper steps={STEPS} current={step} onpick={(i) => go(i)} />
			</div>

			{#key step}
				<div class="mt-8" in:fly={{ y: 8, duration: motion(220) }}>
					{#if step === 0}
						<h2
							bind:this={stepHeading}
							tabindex="-1"
							class="m-0 text-h3 font-semibold outline-none"
						>
							Where will the document be used?
						</h2>
						<form
							class="mt-4 flex max-w-lg flex-col gap-3"
							onsubmit={(e) => {
								e.preventDefault();
								if (slug) void go(1);
							}}
						>
							<span class="text-sm font-semibold" id="dest-label">Destination authority</span>
							<Popover.Root bind:open={comboOpen}>
								<Popover.Trigger
									role="combobox"
									aria-labelledby="dest-label"
									aria-expanded={comboOpen}
									class="field flex cursor-pointer items-center justify-between gap-2 text-left shadow-sm"
								>
									<span class={`truncate ${slug ? 'text-ink' : 'text-ink-muted'}`}>
										{slug ? authorityName : 'Choose an authority…'}
									</span>
									<ChevronDownIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
								</Popover.Trigger>
								<Popover.Content class="w-80 max-w-[calc(100vw-2rem)] p-0" align="start">
									<Command.Root>
										<Command.Input placeholder="Search destinations…" />
										<Command.List>
											<Command.Empty>No destination matches.</Command.Empty>
											{#each grouped as [country, rows] (country)}
												<Command.Group heading={country}>
													{#each rows as a (a.slug)}
														<Command.Item
															value={`${a.name} ${a.country} ${a.slug}`}
															onSelect={() => pick(a.slug)}
														>
															<LandmarkIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
															<span class="truncate">{a.name}</span>
														</Command.Item>
													{/each}
												</Command.Group>
											{/each}
										</Command.List>
									</Command.Root>
								</Popover.Content>
							</Popover.Root>
							{#if slug && plan?.headline}
								<p class="m-0 rounded-md border border-line bg-surface px-4 py-3 text-sm text-ink">
									<span class="font-semibold">From the accepted plan:</span>
									{plan.headline}
								</p>
							{/if}
							<div class="mt-2 flex items-center gap-3">
								<Button type="submit" class="h-11 px-5" disabled={!slug}>
									Continue <ArrowRightIcon aria-hidden="true" />
								</Button>
								{#if !slug}
									<span class="text-sm text-ink-muted">Choose a destination to continue.</span>
								{/if}
							</div>
						</form>
					{:else if step === 1}
						<h2
							bind:this={stepHeading}
							tabindex="-1"
							class="m-0 text-h3 font-semibold outline-none"
						>
							Which document are you sending?
						</h2>
						{#if !bundleReady}
							<div role="status" aria-label="Loading document types" class="mt-4 grid gap-3">
								<span class="sr-only">Loading document types</span>
								<Skeleton class="h-16 w-full" />
								<Skeleton class="h-16 w-full" />
							</div>
						{:else}
							<fieldset class="m-0 mt-4 flex flex-col gap-3 border-0 p-0">
								<legend class="sr-only">Document type for {authorityName}</legend>
								{#each docOptions as p (p.id)}
									{@const on = pathId === p.id}
									<div
										class={`rounded-lg border transition-[box-shadow,border-color] duration-200 has-[input:focus-visible]:ring-3 has-[input:focus-visible]:ring-focus/40 ${
											on
												? 'border-accent-fill bg-bg shadow-md ring-1 ring-accent-fill'
												: 'border-line bg-bg shadow-sm hover:border-brand-300'
										}`}
									>
										<label class="flex min-h-16 cursor-pointer items-center gap-3 p-4" for={`path-${p.id}`}>
											<input
												class="sr-only"
												type="radio"
												name="path"
												id={`path-${p.id}`}
												value={p.id}
												checked={on}
												onchange={() => choosePath(p.id)}
											/>
											<span
												aria-hidden="true"
												class="flex size-10 flex-none items-center justify-center rounded-md bg-surface-2 text-accent-strong"
											>
												<FileTextIcon class="size-5" />
											</span>
											<span class="flex min-w-0 flex-1 flex-col">
												<span class="font-semibold text-ink">{p.label}</span>
												{#if p.fact_ids.length > 0}
													<span class="text-sm text-ink-muted">
														Plan cites {p.fact_ids.length}
														{p.fact_ids.length === 1 ? 'accepted fact' : 'accepted facts'}
													</span>
												{:else}
													<span class="text-sm text-ink-muted">No document-specific plan yet</span>
												{/if}
											</span>
										</label>
										{#each callouts(p.id) as c (c.id)}
											<div
												class="flex flex-wrap items-center gap-x-2 border-t border-line bg-changed-bg px-4 py-2 text-sm text-ink"
											>
												<TriangleAlertIcon class="size-4 flex-none text-changed" aria-hidden="true" />
												<span class="min-w-0 flex-1 basis-48">
													<span class="font-semibold">{c.title}</span>
													{#if c.body}· {c.body}{/if}
												</span>
												{#if c.fact_ids[0] !== undefined}
													<FactChip factId={c.fact_ids[0]} fact={factById(c.fact_ids[0])} />
												{/if}
											</div>
										{/each}
									</div>
								{/each}
							</fieldset>
							{#if planPaths.length === 0 && topics.length > 0}
								<div class="mt-4 rounded-md border border-line bg-surface px-4 py-3 text-sm">
									<p class="m-0 font-semibold text-ink">
										{authorityName} has no page plan yet. Its accepted facts cover:
									</p>
									<ul class="m-0 mt-2 flex list-none flex-wrap gap-2 p-0">
										{#each topics as [topic, count] (topic)}
											<li>
												<Badge variant="outline" class="h-7 px-3 text-sm">
													{humanize(topic)} · {count}
												</Badge>
											</li>
										{/each}
									</ul>
								</div>
							{/if}
						{/if}
						<div class="mt-6 flex flex-wrap items-center gap-3">
							<Button variant="outline" class="h-11 px-5" onclick={() => go(0)}>
								<ArrowLeftIcon aria-hidden="true" /> Back
							</Button>
							<Button class="h-11 px-5" disabled={!pathId} onclick={() => go(2)}>
								Continue <ArrowRightIcon aria-hidden="true" />
							</Button>
							{#if !pathId && bundleReady}
								<span class="text-sm text-ink-muted">Choose a document type to continue.</span>
							{/if}
						</div>
					{:else if step === 2}
						<h2
							bind:this={stepHeading}
							tabindex="-1"
							class="m-0 text-h3 font-semibold outline-none"
						>
							Fill in the request form
						</h2>
						{#if !bundleReady}
							<div role="status" aria-label="Loading the form" class="mt-4 grid gap-3">
								<span class="sr-only">Loading the form</span>
								<Skeleton class="h-11 w-full" />
								<Skeleton class="h-11 w-full" />
							</div>
						{:else if form}
							<div class="mt-4 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-ink-muted">
								<span class="font-semibold text-ink">{form.title}</span>
								{#if safeHref(form.official_url)}
									<a
										class="inline-flex min-h-11 items-center gap-1"
										href={safeHref(form.official_url)}
										target="_blank"
										rel="noopener noreferrer"
									>
										Official form <ExternalLinkIcon class="size-3.5" aria-hidden="true" />
									</a>
								{/if}
								{#if form.source_fact_id}
									<FactChip factId={form.source_fact_id} fact={factById(form.source_fact_id)} />
								{/if}
							</div>
							<form
								class="mt-2 flex max-w-lg flex-col gap-5"
								onsubmit={(e) => {
									e.preventDefault();
									if (missingFields.length === 0) {
										formSkipped = false;
										void go(3);
									}
								}}
							>
								{#each form.fields as f (f.name)}
									<div class="flex flex-col gap-1.5">
										{#if f.type === 'checkbox'}
											<label class="flex min-h-11 cursor-pointer items-center gap-3" for={fieldId(f.name)}>
												<input
													id={fieldId(f.name)}
													type="checkbox"
													class="size-5 accent-accent-fill"
													checked={values[f.name] === true}
													onchange={(e) => (values[f.name] = e.currentTarget.checked)}
												/>
												<span class="font-semibold text-ink">{f.label}</span>
											</label>
										{:else if f.options.length > 0}
											<span class="font-semibold text-ink" id={`${fieldId(f.name)}-label`}>
												{f.label}{#if f.required}{' '}<span class="text-ink-muted">(required)</span>{/if}
											</span>
											<div class="flex flex-wrap gap-2" role="radiogroup" aria-labelledby={`${fieldId(f.name)}-label`}>
												{#each f.options as opt (opt)}
													{@const on = values[f.name] === opt}
													<label
														class={`inline-flex min-h-11 cursor-pointer items-center rounded-full border px-4 text-sm has-[input:focus-visible]:ring-3 has-[input:focus-visible]:ring-focus/40 ${
															on
																? 'border-accent-fill bg-accent-fill text-accent-ink'
																: 'border-line bg-bg text-ink'
														}`}
													>
														<input
															class="sr-only"
															type="radio"
															name={f.name}
															value={opt}
															checked={values[f.name] === opt}
															onchange={() => (values[f.name] = opt)}
														/>
														{opt}
													</label>
												{/each}
											</div>
										{:else}
											<label class="font-semibold text-ink" for={fieldId(f.name)}>
												{f.label}{#if f.required}{' '}<span class="text-ink-muted">(required)</span>{/if}
											</label>
											<Input
												id={fieldId(f.name)}
												class="h-11 bg-bg"
												type={f.type === 'number' ? 'number' : f.type === 'date' ? 'date' : 'text'}
												min={f.type === 'number' ? 1 : undefined}
												required={f.required}
												value={typeof values[f.name] === 'string' ? (values[f.name] as string) : ''}
												oninput={(e) => (values[f.name] = e.currentTarget.value)}
												aria-describedby={f.help ? `${fieldId(f.name)}-help` : undefined}
											/>
										{/if}
										{#if f.help}
											<p class="m-0 text-sm text-ink-muted" id={`${fieldId(f.name)}-help`}>{f.help}</p>
										{/if}
										{#if f.label_quote}
											<p class="m-0 text-xs text-ink-muted">
												On the official form:
												<span class="font-quote italic">“{f.label_quote}”</span>
											</p>
										{/if}
									</div>
								{/each}
								<div class="flex flex-wrap items-center gap-3">
									<Button variant="outline" class="h-11 px-5" onclick={() => go(1)}>
										<ArrowLeftIcon aria-hidden="true" /> Back
									</Button>
									<Button type="submit" class="h-11 px-5" disabled={missingFields.length > 0}>
										Continue <ArrowRightIcon aria-hidden="true" />
									</Button>
									<Button
										variant="ghost"
										class="h-11 px-3"
										onclick={() => {
											formSkipped = true;
											void go(3);
										}}
									>
										Fill the paper form instead
									</Button>
								</div>
								{#if missingFields.length > 0}
									<p class="m-0 text-sm text-ink-muted">
										Still needed: {missingFields.map((f) => f.label).join(', ')}.
									</p>
								{/if}
							</form>
						{:else}
							<div class="mt-4">
								<Notice variant="info" heading="No online form for this authority.">
									<p>
										There is no fillable form on file for {authorityName}. Continue to delivery; the
										authority's dossier lists what to send.
									</p>
								</Notice>
							</div>
							<div class="mt-6 flex flex-wrap gap-3">
								<Button variant="outline" class="h-11 px-5" onclick={() => go(1)}>
									<ArrowLeftIcon aria-hidden="true" /> Back
								</Button>
								<Button class="h-11 px-5" onclick={() => go(3)}>
									Continue <ArrowRightIcon aria-hidden="true" />
								</Button>
							</div>
						{/if}
					{:else if step === 3}
						<div class="flex flex-wrap items-center justify-between gap-2">
							<h2
								bind:this={stepHeading}
								tabindex="-1"
								class="m-0 text-h3 font-semibold outline-none"
							>
								How should it travel?
							</h2>
							{#if options?.mode === 'test'}
								<Badge variant="outline" class="h-7 border-changed px-3 text-sm text-changed">
									Test mode · no real charges
								</Badge>
							{/if}
						</div>

						{#if signedIn === false}
							<div
								class="mt-4 flex flex-col gap-3 rounded-lg border border-line bg-surface p-4 text-ink sm:flex-row sm:items-center"
							>
								<LockIcon class="size-5 flex-none" aria-hidden="true" />
								<p class="m-0 flex-1 text-sm">
									<span class="font-semibold">Sign in to choose delivery.</span>
									Orders, labels and tracking live in your account. Your choices so far are kept.
								</p>
								<Button href={nextHref} class="h-11 px-5">
									<LogInIcon aria-hidden="true" /> Sign in
								</Button>
							</div>
						{/if}

						{#if optionsState === 'loading' || optionsState === 'idle'}
							<div role="status" aria-label="Loading delivery options" class="mt-4 grid gap-3 sm:grid-cols-2">
								<span class="sr-only">Loading delivery options</span>
								<Skeleton class="h-24 w-full" />
								<Skeleton class="h-24 w-full" />
								<Skeleton class="h-24 w-full" />
								<Skeleton class="h-24 w-full" />
							</div>
						{:else if optionsState === 'error' || !options}
							<div class="mt-4">
								<Notice variant="error" heading="Delivery options are unavailable right now.">
									<p>Try again in a moment; your choices so far are kept.</p>
								</Notice>
								<Button
									variant="outline"
									class="mt-3 h-11 px-5"
									onclick={() => {
										optionsState = 'idle';
										void loadOptions(slug);
									}}>Try again</Button
								>
							</div>
						{:else}
							{#if !options.paid_enabled}
								<div class="mt-4">
									<Notice variant="warning" heading="Paid options are paused right now.">
										<p>The free address label still works.</p>
									</Notice>
								</div>
							{/if}
							<fieldset class="m-0 mt-4 grid gap-3 border-0 p-0 sm:grid-cols-2">
								<legend class="sr-only">Delivery option</legend>
								{#each options.options as o (o.kind)}
									<DeliveryCard
										option={o}
										currency={options.currency}
										selected={kind === o.kind}
										fact={factById(o.fact_id)}
										onselect={chooseKind}
									/>
								{/each}
							</fieldset>

							{#if signedIn && selected}
								<fieldset class="m-0 mt-6 flex max-w-lg flex-col gap-3 rounded-lg border border-line p-4">
									<legend class="px-1 font-semibold text-ink">Your return address</legend>
									<p class="m-0 text-sm text-ink-muted">
										Printed on your labels, and where the apostilled document comes back to.
									</p>
									<label class="text-sm font-semibold" for="ret-name">Full name</label>
									<Input id="ret-name" class="h-11 bg-bg" autocomplete="name" bind:value={address.name} />
									<label class="text-sm font-semibold" for="ret-line1">Street address</label>
									<Input id="ret-line1" class="h-11 bg-bg" autocomplete="address-line1" bind:value={address.line1} />
									<label class="text-sm font-semibold" for="ret-line2">Apartment, suite (optional)</label>
									<Input id="ret-line2" class="h-11 bg-bg" autocomplete="address-line2" bind:value={address.line2} />
									<div class="grid grid-cols-2 gap-3">
										<div class="flex flex-col gap-1.5">
											<label class="text-sm font-semibold" for="ret-city">City</label>
											<Input id="ret-city" class="h-11 bg-bg" autocomplete="address-level2" bind:value={address.city} />
										</div>
										<div class="flex flex-col gap-1.5">
											<label class="text-sm font-semibold" for="ret-region">State or region</label>
											<Input id="ret-region" class="h-11 bg-bg" autocomplete="address-level1" bind:value={address.region} />
										</div>
										<div class="flex flex-col gap-1.5">
											<label class="text-sm font-semibold" for="ret-postal">Postal code</label>
											<Input id="ret-postal" class="h-11 bg-bg" autocomplete="postal-code" bind:value={address.postal_code} />
										</div>
										<div class="flex flex-col gap-1.5">
											<label class="text-sm font-semibold" for="ret-country">Country code</label>
											<Input
												id="ret-country"
												class="h-11 bg-bg uppercase"
												autocomplete="country"
												maxlength={2}
												bind:value={address.country}
												aria-describedby="ret-country-help"
											/>
										</div>
									</div>
									<p class="m-0 text-xs text-ink-muted" id="ret-country-help">Two letters, for example GB.</p>
								</fieldset>
							{/if}
						{/if}

						{#if payError}
							<p class="mt-4 mb-0 text-sm text-error" role="alert">{payError}</p>
						{/if}
						<div class="mt-6 flex flex-wrap items-center gap-3">
							<Button variant="outline" class="h-11 px-5" onclick={() => go(2)}>
								<ArrowLeftIcon aria-hidden="true" /> Back
							</Button>
							{#if signedIn === false}
								<Button href={nextHref} class="h-11 px-5">
									<LogInIcon aria-hidden="true" /> Sign in to continue
								</Button>
							{:else if selected?.kind === 'address_label'}
								<Button
									class="h-11 px-5"
									disabled={!addressOk || busy !== null}
									onclick={() => placeOrder('address_label')}
								>
									{#if busy === 'free'}<LoaderIcon class="animate-spin" aria-hidden="true" />{/if}
									Create free label
								</Button>
							{:else}
								<Button
									class="h-11 px-5"
									disabled={!selected || !addressOk || busy !== null}
									onclick={() => (kind ? placeOrder(kind) : undefined)}
								>
									{#if busy === 'order' || busy === 'quote'}<LoaderIcon class="animate-spin" aria-hidden="true" />{/if}
									{busy === 'quote' ? 'Getting your quote' : 'Get quote'}
									{#if busy === null}<ArrowRightIcon aria-hidden="true" />{/if}
								</Button>
							{/if}
							{#if signedIn && optionsState === 'ready' && (!selected || !addressOk)}
								<span class="text-sm text-ink-muted">
									{!selected ? 'Choose a delivery option.' : 'Add your return address to continue.'}
								</span>
							{/if}
						</div>
					{:else}
						<h2
							bind:this={stepHeading}
							tabindex="-1"
							class="m-0 text-h3 font-semibold outline-none"
						>
							{paused !== null ? 'Paid options are paused' : 'Review and pay'}
						</h2>
						{#if signedIn === false}
							<div class="mt-4">
								<Notice variant="info" heading="Sign in to pay.">
									<p>
										Your order lives in your account. <a class="underline underline-offset-4" href={nextHref}>Sign in</a> and you will come
										back to delivery.
									</p>
								</Notice>
							</div>
						{:else if paused !== null}
							<div class="mt-4 flex flex-col gap-4">
								<Notice variant="warning" heading="Checkout is paused.">
									<p>{paused || 'Paid options are paused right now.'}</p>
								</Notice>
								{#if freeOption || options === null}
									<div class="flex flex-col gap-3 rounded-lg border border-line bg-bg p-4 shadow-sm sm:flex-row sm:items-center">
										<p class="m-0 flex-1 text-sm text-ink">
											<span class="font-semibold">{freeOption?.title ?? 'Free address label'}</span>
											· print it at home and add your own postage.
										</p>
										<Button class="h-11 px-5" disabled={busy !== null || !addressOk} onclick={useFree}>
											{#if busy === 'free'}<LoaderIcon class="animate-spin" aria-hidden="true" />{/if}
											Use the free label
										</Button>
									</div>
									{#if !addressOk}
										<p class="m-0 text-sm text-ink-muted">
											Add your return address in <button
												type="button"
												class="cursor-pointer border-0 bg-transparent p-0 text-accent-strong underline"
												onclick={() => go(3)}>Delivery</button
											> first.
										</p>
									{/if}
								{/if}
							</div>
						{:else if order && payTotal !== null && currency}
							<div class="mt-4 overflow-hidden rounded-lg border border-line bg-bg shadow-sm">
								<div class="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-surface px-4 py-3">
									<span class="font-semibold text-ink">{selected?.title ?? 'Your order'}</span>
									<span class="font-mono text-xs text-ink-muted">{order.id}</span>
								</div>
								<ul class="m-0 list-none p-0">
									{#each payLines as line, i (`${i}-${line.label}`)}
										<li class="flex items-baseline justify-between gap-4 border-b border-line px-4 py-3 text-sm">
											<span class="text-ink">{line.label}</span>
											<span class="font-mono text-ink">{money(line.cents, currency)}</span>
										</li>
									{/each}
								</ul>
								<div class="flex items-baseline justify-between gap-4 px-4 py-4">
									<span class="font-semibold text-ink">Total</span>
									<span class="font-mono text-h3 font-semibold text-ink">{money(payTotal, currency)}</span>
								</div>
								{#if quote && quote.carrier_quotes.length > 0}
									<ul class="m-0 flex list-none flex-col gap-1 border-t border-line bg-surface px-4 py-3 text-sm text-ink-muted">
										{#each quote.carrier_quotes as cq (cq.leg)}
											<li>
												{cq.leg === 'outbound' ? 'To the authority' : cq.leg === 'return' ? 'Back to you' : 'Kit to you'}:
												{serviceName(cq.service, cq.carrier)}{#if cq.delivery_days}, about {cq.delivery_days}
													{cq.delivery_days === 1 ? 'day' : 'days'}{/if}
											</li>
										{/each}
									</ul>
								{/if}
							</div>
							<div class="mt-3 flex flex-wrap items-center gap-2 text-sm text-ink-muted">
								{#if mode === 'test'}
									<Badge variant="outline" class="h-7 border-changed px-3 text-sm text-changed">Test mode</Badge>
								{/if}
								{#if quote?.expires_at}
									<span>Quote held until {fmtTime(quote.expires_at)}.</span>
								{/if}
								<span>Payment opens on Stripe's secure checkout.</span>
							</div>
							{#if payError}
								<p class="mt-3 mb-0 text-sm text-error" role="alert">{payError}</p>
							{/if}
							<div class="mt-6 flex flex-wrap items-center gap-3">
								<Button variant="outline" class="h-11 px-5" onclick={() => go(3)}>
									<ArrowLeftIcon aria-hidden="true" /> Back
								</Button>
								<Button class="h-11 px-6" disabled={busy !== null} onclick={pay}>
									{#if busy === 'checkout'}
										<LoaderIcon class="animate-spin" aria-hidden="true" /> Opening checkout
									{:else}
										<LockIcon aria-hidden="true" /> Pay {money(payTotal, currency)}
									{/if}
								</Button>
							</div>
						{:else if order}
							<div class="mt-4">
								<Notice variant="info" heading="No quote yet.">
									<p>{payError ?? 'Get a fresh quote for this order.'}</p>
								</Notice>
								<Button class="mt-3 h-11 px-5" disabled={busy !== null} onclick={requote}>
									{#if busy === 'quote'}<LoaderIcon class="animate-spin" aria-hidden="true" />{/if}
									Get quote
								</Button>
							</div>
						{:else}
							<div class="mt-4">
								<Notice variant="info" heading="Choose delivery first.">
									<p>Pick how the document travels, then the quote appears here.</p>
								</Notice>
								<Button variant="outline" class="mt-3 h-11 px-5" onclick={() => go(3)}>
									<ArrowLeftIcon aria-hidden="true" /> Back to delivery
								</Button>
							</div>
						{/if}
					{/if}
				</div>
			{/key}
		</div>

		<div class="min-w-0 lg:col-span-5">
			<aside
				class="rounded-xl border border-line bg-bg p-5 shadow-sm lg:sticky lg:top-20"
				aria-label="Your apostille"
			>
				<h2 class="m-0 text-h3 font-semibold">Your apostille</h2>
				<div class="mt-3">
					<JourneyGraphic authority={slug ? authorityName : null} {kind} {carriers} />
				</div>
				{#if !slug}
					<div role="status" aria-label="Waiting for your choices" class="mt-4 flex flex-col gap-2">
						<span class="sr-only">Choose a destination to see your order summary</span>
						<Skeleton class="h-4 w-2/3" />
						<Skeleton class="h-12 w-full" />
						<Skeleton class="h-4 w-1/2" />
					</div>
				{:else}
					<dl class="m-0 mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
						<dt class="text-ink-muted">Destination</dt>
						<dd class="m-0 text-ink">{authorityName}</dd>
						<dt class="text-ink-muted">Document</dt>
						<dd class="m-0 text-ink">{pathLabel || 'Not chosen yet'}</dd>
						<dt class="text-ink-muted">Form</dt>
						<dd class="m-0 text-ink">
							{#if !form}
								None on file
							{:else if formSkipped}
								Paper form
							{:else}
								{filledCount} of {form.fields.length} fields filled
							{/if}
						</dd>
						<dt class="text-ink-muted">Delivery</dt>
						<dd class="m-0 text-ink">
							{#if selected}
								{selected.title}
								{#if selected.from_cents !== null && currency}
									<span class="text-ink-muted">
										· {selected.from_cents === 0 ? 'Free' : `from ${money(selected.from_cents, currency, { compact: true })}`}
									</span>
								{/if}
							{:else}
								Not chosen yet
							{/if}
						</dd>
						{#if payTotal !== null && currency}
							<dt class="text-ink-muted">Total</dt>
							<dd class="m-0 font-mono font-semibold text-ink">{money(payTotal, currency)}</dd>
						{/if}
					</dl>
					{#if feeLines.length > 0 || timing}
						<div class="mt-4 border-t border-line pt-4">
							<p class="m-0 text-eyebrow font-semibold tracking-[0.06em] text-ink-muted uppercase">
								From the authority
							</p>
							<ul class="m-0 mt-2 flex list-none flex-col gap-1 p-0 text-sm">
								{#each feeLines as fee, i (`${i}-${fee.label}`)}
									<li class="flex flex-wrap items-center gap-x-2">
										<span class="text-ink">
											{fee.label}{#if currency}: {money(fee.amount_cents, currency)}{/if}{#if fee.per}{' '}per
												{fee.per}{/if}
										</span>
										{#if fee.fact_id}<FactChip factId={fee.fact_id} fact={factById(fee.fact_id)} />{/if}
									</li>
								{/each}
								{#if timing}
									<li class="flex flex-wrap items-center gap-x-2">
										<span class="text-ink">{timing.title}</span>
										{#if timing.fact_ids[0] !== undefined}
											<FactChip factId={timing.fact_ids[0]} fact={factById(timing.fact_ids[0])} />
										{/if}
									</li>
								{/if}
							</ul>
						</div>
					{/if}
				{/if}
			</aside>
		</div>
	</div>
{/if}

<noscript>
	<p class="rounded-md border border-line bg-surface px-4 py-3">Placing an order needs JavaScript.</p>
</noscript>
