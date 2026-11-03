<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { csrfHeaders } from '../../../lib/admin/config-client.js';

	// Staff-only live FedEx purchase (SA-19). This route ships in the
	// admin build only (never the public build). The staff API keeps
	// the /apostille prefix on the admin origin, so the root is
	// literal (same as the config/calls pages). Quote and purchase
	// post the full carrier payload: a live buy needs real
	// ship-from/ship-to addresses, never a ZIP alone. FedEx Ship
	// requires a contact phone on both ends, so purchase waits for
	// both; the bought label PDF stays downloadable from the store.
	const api = () => `/apostille/api/v1/fedex-live`;

	interface Caps {
		enabled: boolean;
		daily_cap: number;
		bought_today: number;
		max_usd: number;
	}

	interface Address {
		name: string;
		street1: string;
		city: string;
		state_province: string;
		postal: string;
		country: string;
		phone: string;
	}

	const blankAddress = (): Address => ({
		name: '',
		street1: '',
		city: '',
		state_province: '',
		postal: '',
		country: 'US',
		phone: ''
	});

	let caps = $state<Caps | null>(null);
	let capsState = $state<'working' | 'ready' | 'failed'>('working');
	let capsError = $state<string | null>(null);

	let orderId = $state('');
	let leg = $state<'outbound' | 'return'>('outbound');
	let service = $state<'overnight' | 'two_day' | 'ground'>('two_day');
	let parcel = $state<'letter_envelope' | 'flat_envelope'>('flat_envelope');
	let from = $state<Address>(blankAddress());
	let to = $state<Address>(blankAddress());

	let quoteCents = $state<number | null>(null);
	let quoteState = $state<'idle' | 'working' | 'done' | 'failed'>('idle');
	let quoteError = $state<string | null>(null);

	let tracking = $state<string | null>(null);
	let labelUrl = $state<string | null>(null);
	let buyState = $state<'idle' | 'working' | 'done' | 'failed'>('idle');
	let buyError = $state<string | null>(null);

	let voidState = $state<'idle' | 'working' | 'done' | 'failed'>('idle');
	let voidError = $state<string | null>(null);
	let voidMsg = $state<string | null>(null);

	const idemKey = $derived(
		orderId.trim() ? `${orderId.trim()}:${leg}` : ''
	);

	// An empty phone is left out (the server treats a present phone
	// as one to validate).
	const wire = (a: Address) => {
		const { phone, ...rest } = a;
		return phone.trim() ? { ...rest, phone: phone.trim() } : rest;
	};

	function payload() {
		return {
			from: wire(from),
			to: wire(to),
			parcel: { preset: parcel },
			service,
			pairing_id: orderId.trim()
		};
	}

	const addressReady = (a: Address) =>
		a.name.trim() && a.street1.trim() && a.city.trim() && a.state_province.trim() && a.postal.trim();

	const formReady = $derived(
		orderId.trim().length > 0 && !!addressReady(from) && !!addressReady(to)
	);

	// 10 digits for US numbers; a leading 1 / +1 is dropped server-side.
	const phoneReady = (a: Address) => {
		const digits = a.phone.replace(/\D/g, '');
		return digits.length === 10 || (digits.length === 11 && digits.startsWith('1'));
	};
	const buyReady = $derived(formReady && phoneReady(from) && phoneReady(to));

	async function loadCaps() {
		capsState = 'working';
		capsError = null;
		try {
			const res = await fetch(`${api()}/caps`);
			if (!res.ok) throw new Error(`caps request failed (${res.status})`);
			caps = (await res.json()) as Caps;
			capsState = 'ready';
		} catch (err) {
			capsError = err instanceof Error ? err.message : 'Caps request failed.';
			capsState = 'failed';
		}
	}

	onMount(loadCaps);

	async function quote() {
		quoteState = 'working';
		quoteError = null;
		quoteCents = null;
		try {
			const res = await fetch(`${api()}/quote`, {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				body: JSON.stringify({ payload: payload() })
			});
			if (!res.ok) throw new Error(`quote failed (${res.status})`);
			const data = (await res.json()) as { quoted_cents: number };
			quoteCents = data.quoted_cents;
			quoteState = 'done';
		} catch (err) {
			quoteError = err instanceof Error ? err.message : 'Quote failed.';
			quoteState = 'failed';
		}
	}

	async function buy() {
		buyState = 'working';
		buyError = null;
		tracking = null;
		labelUrl = null;
		try {
			const res = await fetch(`${api()}/purchase`, {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				body: JSON.stringify({
					order_id: orderId.trim(),
					leg,
					payload: payload(),
					quoted_cents: quoteCents
				})
			});
			if (!res.ok) throw new Error(`purchase failed (${res.status})`);
			const data = (await res.json()) as { tracking_no: string; label_url?: string | null };
			tracking = data.tracking_no;
			labelUrl = data.label_url ?? null;
			buyState = 'done';
			voidState = 'idle';
			voidMsg = null;
			await loadCaps();
		} catch (err) {
			buyError = err instanceof Error ? err.message : 'Purchase failed.';
			buyState = 'failed';
		}
	}

	async function voidLabel() {
		voidState = 'working';
		voidError = null;
		voidMsg = null;
		try {
			const res = await fetch(`${api()}/void`, {
				method: 'POST',
				headers: { 'content-type': 'application/json', ...csrfHeaders() },
				body: JSON.stringify({ order_id: orderId.trim(), leg })
			});
			if (!res.ok) throw new Error(`void failed (${res.status})`);
			voidMsg = `Voided ${idemKey}.`;
			voidState = 'done';
		} catch (err) {
			voidError = err instanceof Error ? err.message : 'Void failed.';
			voidState = 'failed';
		}
	}
</script>

<svelte:head>
	<title>FedEx live purchase — staff</title>
</svelte:head>

<main class="mx-auto max-w-2xl space-y-6 p-6">
	<h1 class="text-xl font-bold">FedEx live purchase</h1>
	<p class="text-sm">
		Staff-only live label purchase. Caps: 2 labels per day, USD 60 max per label,
		allowlisted destination ZIPs only. One purchase per order leg.
	</p>

	<section aria-label="Live caps" class="rounded border border-line p-4">
		<h2 class="font-bold">Live caps</h2>
		{#if capsState === 'working'}
			<div role="status" aria-label="Working" class="space-y-2">
				<div class="h-4 animate-pulse rounded bg-surface-2"></div>
				<div class="h-4 w-2/3 animate-pulse rounded bg-surface-2"></div>
			</div>
		{:else if capsState === 'ready' && caps}
			<ul class="list-disc pl-5 text-sm">
				<li>Live buys: {caps.enabled ? 'on' : 'off'}</li>
				<li>Bought today: {caps.bought_today} of {caps.daily_cap}</li>
				<li>Per-label max: USD {caps.max_usd}</li>
			</ul>
		{:else}
			<Notice variant="error" heading="Caps unavailable"><p>{capsError ?? 'Unknown error.'}</p></Notice>
			<button type="button" class="mt-2 rounded border border-accent-fill bg-accent-fill px-4 py-2 text-accent-ink" onclick={loadCaps}>
				Retry
			</button>
		{/if}
	</section>

	<section aria-label="Shipment" class="space-y-3 rounded border border-line p-4">
		<h2 class="font-bold">Shipment</h2>
		<div>
			<label for="fedex-order" class="block text-sm font-bold">Order ID</label>
			<input
				id="fedex-order"
				class="mt-1 w-full rounded border border-line bg-bg p-2 text-ink"
				type="text"
				autocomplete="off"
				bind:value={orderId}
			/>
		</div>
		<div class="flex gap-4">
			<div>
				<label for="fedex-leg" class="block text-sm font-bold">Leg</label>
				<select id="fedex-leg" class="mt-1 rounded border border-line bg-bg p-2 text-ink" bind:value={leg}>
					<option value="outbound">Outbound</option>
					<option value="return">Return</option>
				</select>
			</div>
			<div>
				<label for="fedex-service" class="block text-sm font-bold">Service</label>
				<select id="fedex-service" class="mt-1 rounded border border-line bg-bg p-2 text-ink" bind:value={service}>
					<option value="overnight">Overnight</option>
					<option value="two_day">2Day</option>
					<option value="ground">Ground</option>
				</select>
			</div>
			<div>
				<label for="fedex-parcel" class="block text-sm font-bold">Parcel</label>
				<select id="fedex-parcel" class="mt-1 rounded border border-line bg-bg p-2 text-ink" bind:value={parcel}>
					<option value="letter_envelope">Letter envelope</option>
					<option value="flat_envelope">Flat envelope</option>
				</select>
			</div>
		</div>
		{#if idemKey}
			<p class="text-sm">Idempotency key: <code>{idemKey}</code></p>
		{/if}
	</section>

	{#snippet addressFields(id: string, title: string, a: Address)}
		<fieldset class="space-y-3 rounded border border-line p-4">
			<legend class="px-1 font-bold">{title}</legend>
			<div>
				<label for={`${id}-name`} class="block text-sm font-bold">Name</label>
				<input id={`${id}-name`} class="mt-1 w-full rounded border border-line bg-bg p-2 text-ink" type="text" autocomplete="off" bind:value={a.name} />
			</div>
			<div>
				<label for={`${id}-street`} class="block text-sm font-bold">Street</label>
				<input id={`${id}-street`} class="mt-1 w-full rounded border border-line bg-bg p-2 text-ink" type="text" autocomplete="off" bind:value={a.street1} />
			</div>
			<div>
				<label for={`${id}-phone`} class="block text-sm font-bold">Contact phone</label>
				<input
					id={`${id}-phone`}
					class="mt-1 w-full rounded border border-line bg-bg p-2 text-ink"
					type="tel"
					inputmode="tel"
					autocomplete="off"
					placeholder="360 555 0100"
					bind:value={a.phone}
				/>
			</div>
			<div class="flex gap-4">
				<div class="flex-1">
					<label for={`${id}-city`} class="block text-sm font-bold">City</label>
					<input id={`${id}-city`} class="mt-1 w-full rounded border border-line bg-bg p-2 text-ink" type="text" autocomplete="off" bind:value={a.city} />
				</div>
				<div>
					<label for={`${id}-state`} class="block text-sm font-bold">State</label>
					<input id={`${id}-state`} class="mt-1 w-24 rounded border border-line bg-bg p-2 text-ink" type="text" autocomplete="off" bind:value={a.state_province} />
				</div>
				<div>
					<label for={`${id}-zip`} class="block text-sm font-bold">ZIP</label>
					<input id={`${id}-zip`} class="mt-1 w-32 rounded border border-line bg-bg p-2 text-ink" type="text" inputmode="numeric" autocomplete="off" bind:value={a.postal} />
				</div>
			</div>
		</fieldset>
	{/snippet}

	{@render addressFields('fedex-from', 'Ship from', from)}
	{@render addressFields('fedex-to', 'Ship to', to)}

	<section aria-label="Quote" class="space-y-3 rounded border border-line p-4">
		<h2 class="font-bold">Rate quote</h2>
		<button
			type="button"
			class="rounded border border-accent-fill bg-accent-fill px-4 py-2 text-accent-ink disabled:opacity-50"
			disabled={!formReady || quoteState === 'working'}
			onclick={quote}
		>
			Get quote
		</button>
		{#if quoteState === 'working'}
			<div role="status" aria-label="Working">
				<div class="h-4 w-1/2 animate-pulse rounded bg-surface-2"></div>
			</div>
		{:else if quoteState === 'done' && quoteCents !== null}
			<Notice variant="info" heading="Quote"
				><p>Quoted {(quoteCents! / 100).toFixed(2)} USD. Cap check happens at purchase.</p></Notice
			>
		{:else if quoteState === 'failed'}
			<Notice variant="error" heading="Quote failed"><p>{quoteError ?? 'Unknown error.'}</p></Notice>
		{/if}
	</section>

	<section aria-label="Purchase" class="space-y-3 rounded border border-line p-4">
		<h2 class="font-bold">Purchase</h2>
		{#if formReady && !buyReady}
			<p class="text-sm">FedEx needs a 10-digit contact phone for both addresses.</p>
		{/if}
		<button
			type="button"
			class="rounded border border-accent-fill bg-accent-fill px-4 py-2 text-accent-ink disabled:opacity-50"
			disabled={!buyReady || buyState === 'working'}
			onclick={buy}
		>
			Buy label
		</button>
		{#if buyState === 'working'}
			<div role="status" aria-label="Working">
				<div class="h-4 w-1/2 animate-pulse rounded bg-surface-2"></div>
			</div>
		{:else if buyState === 'done' && tracking}
			<Notice variant="success" heading="Label bought">
				<p>
					Tracking: {tracking}
					{#if labelUrl}
						· <a class="font-bold underline" href={labelUrl} download>Label PDF</a>
					{:else}
						· Label PDF not stored; reprint it from FedEx Ship Manager.
					{/if}
				</p>
			</Notice>
		{:else if buyState === 'failed'}
			<Notice variant="error" heading="Purchase failed"><p>{buyError ?? 'Unknown error.'}</p></Notice>
		{/if}
	</section>

	<section aria-label="Void" class="space-y-3 rounded border border-line p-4">
		<h2 class="font-bold">Void</h2>
		<button
			type="button"
			class="rounded border border-error bg-error-bg px-4 py-2 text-error disabled:opacity-50"
			disabled={!formReady || buyState !== 'done' || voidState === 'working'}
			onclick={voidLabel}
		>
			Void label
		</button>
		{#if voidState === 'working'}
			<div role="status" aria-label="Working">
				<div class="h-4 w-1/2 animate-pulse rounded bg-surface-2"></div>
			</div>
		{:else if voidState === 'done' && voidMsg}
			<Notice variant="success" heading="Voided"><p>{voidMsg}</p></Notice>
		{:else if voidState === 'failed'}
			<Notice variant="error" heading="Void failed"><p>{voidError ?? 'Unknown error.'}</p></Notice>
		{/if}
	</section>
</main>
