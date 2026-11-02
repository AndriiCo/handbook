<script lang="ts">
	import { onMount } from 'svelte';
	import Notice from '../../../lib/public-ui/Notice.svelte';

	// Staff-only live FedEx purchase (SA-19). This route ships in the
	// admin build only (never the public build). The staff API keeps
	// the /apostille prefix on the admin origin, so the root is
	// literal (same as the config/calls pages). Quote and purchase
	// post the full carrier payload: a live buy needs real
	// ship-from/ship-to addresses, never a ZIP alone.
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
	}

	const blankAddress = (): Address => ({
		name: '',
		street1: '',
		city: '',
		state_province: '',
		postal: '',
		country: 'US'
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
	let buyState = $state<'idle' | 'working' | 'done' | 'failed'>('idle');
	let buyError = $state<string | null>(null);

	let voidState = $state<'idle' | 'working' | 'done' | 'failed'>('idle');
	let voidError = $state<string | null>(null);
	let voidMsg = $state<string | null>(null);

	const idemKey = $derived(
		orderId.trim() ? `${orderId.trim()}:${leg}` : ''
	);

	function payload() {
		return {
			from,
			to,
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
				headers: { 'content-type': 'application/json' },
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
		try {
			const res = await fetch(`${api()}/purchase`, {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					order_id: orderId.trim(),
					leg,
					payload: payload(),
					quoted_cents: quoteCents
				})
			});
			if (!res.ok) throw new Error(`purchase failed (${res.status})`);
			const data = (await res.json()) as { tracking_no: string };
			tracking = data.tracking_no;
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
				headers: { 'content-type': 'application/json' },
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

	<section aria-label="Live caps" class="rounded border p-4">
		<h2 class="font-bold">Live caps</h2>
		{#if capsState === 'working'}
			<div role="status" aria-label="Working" class="space-y-2">
				<div class="h-4 animate-pulse rounded bg-gray-200"></div>
				<div class="h-4 w-2/3 animate-pulse rounded bg-gray-200"></div>
			</div>
		{:else if capsState === 'ready' && caps}
			<ul class="list-disc pl-5 text-sm">
				<li>Live buys: {caps.enabled ? 'on' : 'off'}</li>
				<li>Bought today: {caps.bought_today} of {caps.daily_cap}</li>
				<li>Per-label max: USD {caps.max_usd}</li>
			</ul>
		{:else}
			<Notice variant="error" heading="Caps unavailable"><p>{capsError ?? 'Unknown error.'}</p></Notice>
			<button type="button" class="mt-2 rounded bg-blue-600 px-4 py-2 text-white" onclick={loadCaps}>
				Retry
			</button>
		{/if}
	</section>

	<section aria-label="Shipment" class="space-y-3 rounded border p-4">
		<h2 class="font-bold">Shipment</h2>
		<div>
			<label for="fedex-order" class="block text-sm font-bold">Order ID</label>
			<input
				id="fedex-order"
				class="mt-1 w-full rounded border p-2"
				type="text"
				autocomplete="off"
				bind:value={orderId}
			/>
		</div>
		<div class="flex gap-4">
			<div>
				<label for="fedex-leg" class="block text-sm font-bold">Leg</label>
				<select id="fedex-leg" class="mt-1 rounded border p-2" bind:value={leg}>
					<option value="outbound">Outbound</option>
					<option value="return">Return</option>
				</select>
			</div>
			<div>
				<label for="fedex-service" class="block text-sm font-bold">Service</label>
				<select id="fedex-service" class="mt-1 rounded border p-2" bind:value={service}>
					<option value="overnight">Overnight</option>
					<option value="two_day">2Day</option>
					<option value="ground">Ground</option>
				</select>
			</div>
			<div>
				<label for="fedex-parcel" class="block text-sm font-bold">Parcel</label>
				<select id="fedex-parcel" class="mt-1 rounded border p-2" bind:value={parcel}>
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
		<fieldset class="space-y-3 rounded border p-4">
			<legend class="px-1 font-bold">{title}</legend>
			<div>
				<label for={`${id}-name`} class="block text-sm font-bold">Name</label>
				<input id={`${id}-name`} class="mt-1 w-full rounded border p-2" type="text" autocomplete="off" bind:value={a.name} />
			</div>
			<div>
				<label for={`${id}-street`} class="block text-sm font-bold">Street</label>
				<input id={`${id}-street`} class="mt-1 w-full rounded border p-2" type="text" autocomplete="off" bind:value={a.street1} />
			</div>
			<div class="flex gap-4">
				<div class="flex-1">
					<label for={`${id}-city`} class="block text-sm font-bold">City</label>
					<input id={`${id}-city`} class="mt-1 w-full rounded border p-2" type="text" autocomplete="off" bind:value={a.city} />
				</div>
				<div>
					<label for={`${id}-state`} class="block text-sm font-bold">State</label>
					<input id={`${id}-state`} class="mt-1 w-24 rounded border p-2" type="text" autocomplete="off" bind:value={a.state_province} />
				</div>
				<div>
					<label for={`${id}-zip`} class="block text-sm font-bold">ZIP</label>
					<input id={`${id}-zip`} class="mt-1 w-32 rounded border p-2" type="text" inputmode="numeric" autocomplete="off" bind:value={a.postal} />
				</div>
			</div>
		</fieldset>
	{/snippet}

	{@render addressFields('fedex-from', 'Ship from', from)}
	{@render addressFields('fedex-to', 'Ship to', to)}

	<section aria-label="Quote" class="space-y-3 rounded border p-4">
		<h2 class="font-bold">Rate quote</h2>
		<button
			type="button"
			class="rounded bg-blue-600 px-4 py-2 text-white disabled:opacity-50"
			disabled={!formReady || quoteState === 'working'}
			onclick={quote}
		>
			Get quote
		</button>
		{#if quoteState === 'working'}
			<div role="status" aria-label="Working">
				<div class="h-4 w-1/2 animate-pulse rounded bg-gray-200"></div>
			</div>
		{:else if quoteState === 'done' && quoteCents !== null}
			<Notice variant="info" heading="Quote"
				><p>Quoted {(quoteCents! / 100).toFixed(2)} USD. Cap check happens at purchase.</p></Notice
			>
		{:else if quoteState === 'failed'}
			<Notice variant="error" heading="Quote failed"><p>{quoteError ?? 'Unknown error.'}</p></Notice>
		{/if}
	</section>

	<section aria-label="Purchase" class="space-y-3 rounded border p-4">
		<h2 class="font-bold">Purchase</h2>
		<button
			type="button"
			class="rounded bg-blue-600 px-4 py-2 text-white disabled:opacity-50"
			disabled={!formReady || buyState === 'working'}
			onclick={buy}
		>
			Buy label
		</button>
		{#if buyState === 'working'}
			<div role="status" aria-label="Working">
				<div class="h-4 w-1/2 animate-pulse rounded bg-gray-200"></div>
			</div>
		{:else if buyState === 'done' && tracking}
			<Notice variant="success" heading="Label bought"><p>Tracking: {tracking}</p></Notice>
		{:else if buyState === 'failed'}
			<Notice variant="error" heading="Purchase failed"><p>{buyError ?? 'Unknown error.'}</p></Notice>
		{/if}
	</section>

	<section aria-label="Void" class="space-y-3 rounded border p-4">
		<h2 class="font-bold">Void</h2>
		<button
			type="button"
			class="rounded bg-red-700 px-4 py-2 text-white disabled:opacity-50"
			disabled={!formReady || buyState !== 'done' || voidState === 'working'}
			onclick={voidLabel}
		>
			Void label
		</button>
		{#if voidState === 'working'}
			<div role="status" aria-label="Working">
				<div class="h-4 w-1/2 animate-pulse rounded bg-gray-200"></div>
			</div>
		{:else if voidState === 'done' && voidMsg}
			<Notice variant="success" heading="Voided"><p>{voidMsg}</p></Notice>
		{:else if voidState === 'failed'}
			<Notice variant="error" heading="Void failed"><p>{voidError ?? 'Unknown error.'}</p></Notice>
		{/if}
	</section>
</main>
