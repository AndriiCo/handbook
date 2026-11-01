<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	type LabelSummary = {
		direction: string;
		carrier: string;
		service_name: string;
		status: string;
		tracking_no: string | null;
		tracking_url: string | null;
		label_url: string | null;
	};

	type LabelView = {
		id: number;
		purchase_option: string;
		label_format: string;
		content_type: string;
		summary: LabelSummary;
	};

	type Address = {
		name: string;
		street1: string;
		street2: string;
		city: string;
		state_province: string;
		postal: string;
		country: string;
		phone: string;
	};

	const FORMATS: Array<[string, string]> = [
		['four_by_six', '4×6 label'],
		['half_letter', 'Half-letter (two per sheet)'],
		['letter', 'US Letter'],
		['a4', 'A4'],
		['window_envelope', 'Window envelope (#10)']
	];
	const PAPERS: Array<[string, string]> = [
		['four_by_six', '4×6'],
		['letter', 'US Letter'],
		['a4', 'A4']
	];
	const SERVICES: Array<[string, string]> = [
		['overnight', 'Overnight'],
		['two_day', '2-day'],
		['ground', 'Ground']
	];
	const PARCELS: Array<[string, string]> = [
		['letter_envelope', 'Letter envelope'],
		['flat_envelope', 'Flat envelope']
	];

	const emptyAddress = (country: string): Address => ({
		name: '',
		street1: '',
		street2: '',
		city: '',
		state_province: '',
		postal: '',
		country,
		phone: ''
	});

	let authorityId = $state('1');
	let from = $state<Address>(emptyAddress('US'));
	let to = $state<Address>(emptyAddress('US'));
	let parcel = $state('letter_envelope');
	let service = $state('two_day');

	let shipmentId = $state<number | null>(null);
	let shipmentStatus = $state('');
	let labels = $state<LabelView[]>([]);

	let direction = $state('outbound');
	let carrier = $state('usps');
	let kind = $state<'printable' | 'prepaid'>('printable');
	let format = $state('four_by_six');
	let paper = $state('four_by_six');

	let notice = $state('');
	let errors = $state<Array<{ field?: string; message?: string }>>([]);
	let busy = $state(false);
	let pasteTracking = $state<Record<number, string>>({});
	let pasteFiles = $state<Record<number, File | null>>({});

	function cleanAddress(addr: Address): Record<string, string> {
		const out: Record<string, string> = {
			name: addr.name.trim(),
			street1: addr.street1.trim(),
			city: addr.city.trim(),
			state_province: addr.state_province.trim(),
			postal: addr.postal.trim(),
			country: addr.country.trim().toUpperCase()
		};
		if (addr.street2.trim()) out.street2 = addr.street2.trim();
		if (addr.phone.trim()) out.phone = addr.phone.trim();
		return out;
	}

	async function readError(res: Response): Promise<void> {
		errors = [];
		notice = '';
		try {
			const data = await res.json();
			if (typeof data?.error === 'string') {
				notice = data.error;
			} else if (Array.isArray(data?.errors)) {
				errors = data.errors;
			} else {
				notice = `Request failed (${res.status}).`;
			}
		} catch {
			notice = `Request failed (${res.status}).`;
		}
	}

	async function loadDefaults(): Promise<void> {
		try {
			const res = await fetch(api('/p2/profile/label-defaults'), {
				credentials: 'same-origin'
			});
			if (!res.ok) return;
			const data = await res.json();
			if (typeof data?.option_a_format === 'string') format = data.option_a_format;
			if (typeof data?.option_b_paper === 'string') paper = data.option_b_paper;
		} catch {
			// Defaults are a convenience; the form works without them.
		}
	}

	async function saveDefaults(): Promise<void> {
		busy = true;
		notice = '';
		errors = [];
		try {
			const res = await fetch(api('/p2/profile/label-defaults'), {
				method: 'PUT',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({ option_a_format: format, option_b_paper: paper })
			});
			if (!res.ok) {
				await readError(res);
				return;
			}
			notice = 'Defaults saved.';
		} catch {
			notice = 'Could not save defaults.';
		} finally {
			busy = false;
		}
	}

	async function createShipment(): Promise<void> {
		busy = true;
		notice = '';
		errors = [];
		try {
			const res = await fetch(api('/p2/shipments'), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					authority_id: Number(authorityId),
					from: cleanAddress(from),
					to: cleanAddress(to),
					parcel: { preset: parcel },
					service
				})
			});
			if (!res.ok) {
				await readError(res);
				return;
			}
			const data = await res.json();
			shipmentId = data.id;
			shipmentStatus = data.status;
			labels = [];
			notice = `Shipment ${shipmentId} created.`;
		} catch {
			notice = 'Could not create the shipment.';
		} finally {
			busy = false;
		}
	}

	async function refresh(): Promise<void> {
		if (shipmentId === null) return;
		try {
			const res = await fetch(api(`/p2/shipments/${shipmentId}`), {
				credentials: 'same-origin'
			});
			if (!res.ok) {
				await readError(res);
				return;
			}
			const data = await res.json();
			shipmentStatus = data.status;
			labels = Array.isArray(data.labels) ? data.labels : [];
		} catch {
			notice = 'Could not refresh the shipment.';
		}
	}

	async function purchase(): Promise<void> {
		if (shipmentId === null) return;
		busy = true;
		notice = '';
		errors = [];
		try {
			const res = await fetch(api(`/p2/shipments/${shipmentId}/purchase`), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					direction,
					carrier: kind === 'prepaid' ? 'usps' : carrier,
					option: kind === 'printable' ? { kind, format } : { kind, paper }
				})
			});
			if (!res.ok) {
				await readError(res);
				return;
			}
			const data = await res.json();
			if (data?.queued === true) {
				notice =
					'Queued for staff purchase. The label appears below once staff paste back the tracking number and file.';
			} else {
				notice = 'Label issued.';
			}
			await refresh();
		} catch {
			notice = 'Could not purchase the label.';
		} finally {
			busy = false;
		}
	}

	function pickFile(id: number, event: Event): void {
		const input = event.currentTarget as HTMLInputElement | null;
		pasteFiles[id] = input?.files?.[0] ?? null;
	}

	function fileToBase64(file: File): Promise<string> {
		return new Promise((resolvePromise, reject) => {
			const reader = new FileReader();
			reader.onload = () => {
				const text = typeof reader.result === 'string' ? reader.result : '';
				resolvePromise(text.split(',').pop() ?? '');
			};
			reader.onerror = () => reject(new Error('read failed'));
			reader.readAsDataURL(file);
		});
	}

	async function pasteBack(id: number): Promise<void> {
		if (shipmentId === null) return;
		const file = pasteFiles[id];
		if (!file) {
			notice = 'Choose the bought label PDF first.';
			return;
		}
		busy = true;
		notice = '';
		errors = [];
		try {
			const res = await fetch(api(`/p2/shipments/${shipmentId}/labels/${id}/paste-back`), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					tracking_no: (pasteTracking[id] ?? '').trim(),
					label_pdf_base64: await fileToBase64(file)
				})
			});
			if (!res.ok) {
				await readError(res);
				return;
			}
			notice = 'Tracking pasted back; label issued.';
			pasteTracking[id] = '';
			pasteFiles[id] = null;
			await refresh();
		} catch {
			notice = 'Could not paste back.';
		} finally {
			busy = false;
		}
	}

	async function voidLabel(id: number): Promise<void> {
		if (shipmentId === null) return;
		busy = true;
		notice = '';
		errors = [];
		try {
			const res = await fetch(api(`/p2/shipments/${shipmentId}/labels/${id}/void`), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({})
			});
			if (!res.ok) {
				await readError(res);
				return;
			}
			notice = 'Label voided.';
			await refresh();
		} catch {
			notice = 'Could not void the label.';
		} finally {
			busy = false;
		}
	}

	function downloadHref(id: number): string {
		return api(`/p2/shipments/${shipmentId}/labels/${id}/download`);
	}

	function formatName(value: string): string {
		return FORMATS.find(([v]) => v === value)?.[1] ?? value;
	}

	onMount(loadDefaults);
</script>

<svelte:head>
	<title>Shipping labels — Apostille Handbook</title>
</svelte:head>

<h1>Shipping labels</h1>

{#if notice}
	<p class="rounded-sm border border-line bg-surface px-4 py-3" role="status">{notice}</p>
{/if}
{#if errors.length}
	<ul class="rounded-sm border border-[var(--danger,#b3261e)] bg-surface py-3 pr-4 pl-8" role="alert">
		{#each errors as err}
			<li>{err.field ? `${err.field}: ` : ''}{err.message}</li>
		{/each}
	</ul>
{/if}

<section aria-labelledby="shipment-h">
	<h2 id="shipment-h">1. Shipment</h2>
	<form
		class="flex max-w-[44rem] flex-col gap-2"
		onsubmit={(e) => {
			e.preventDefault();
			createShipment();
		}}
	>
		<label class="font-bold" for="authority">Authority ID</label>
		<input
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
			id="authority"
			type="number"
			min="1"
			bind:value={authorityId}
			required
		/>

		<div class="grid grid-cols-1 gap-4 min-[48rem]:grid-cols-2">
			<fieldset class="rounded-sm border border-line px-4 py-3">
				<legend class="px-1 font-bold">From</legend>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="from-name">Name</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="from-name"
					type="text"
					bind:value={from.name}
					required
					autocomplete="name"
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="from-street">Street</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="from-street"
					type="text"
					bind:value={from.street1}
					required
					autocomplete="street-address"
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="from-street2">
					Street, line 2 (optional)
				</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="from-street2"
					type="text"
					bind:value={from.street2}
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="from-city">City</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="from-city"
					type="text"
					bind:value={from.city}
					required
					autocomplete="address-level2"
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="from-state">State</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="from-state"
					type="text"
					bind:value={from.state_province}
					required
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="from-postal">
					Postal code
				</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="from-postal"
					type="text"
					bind:value={from.postal}
					required
					autocomplete="postal-code"
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="from-country">
					Country (ISO-2)
				</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="from-country"
					type="text"
					bind:value={from.country}
					required
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="from-phone">
					Phone (optional)
				</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="from-phone"
					type="tel"
					bind:value={from.phone}
					autocomplete="tel"
				/>
			</fieldset>
			<fieldset class="rounded-sm border border-line px-4 py-3">
				<legend class="px-1 font-bold">To</legend>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="to-name">Name</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="to-name"
					type="text"
					bind:value={to.name}
					required
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="to-street">Street</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="to-street"
					type="text"
					bind:value={to.street1}
					required
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="to-street2">
					Street, line 2 (optional)
				</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="to-street2"
					type="text"
					bind:value={to.street2}
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="to-city">City</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="to-city"
					type="text"
					bind:value={to.city}
					required
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="to-state">State</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="to-state"
					type="text"
					bind:value={to.state_province}
					required
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="to-postal">
					Postal code
				</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="to-postal"
					type="text"
					bind:value={to.postal}
					required
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="to-country">
					Country (ISO-2)
				</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="to-country"
					type="text"
					bind:value={to.country}
					required
				/>
				<label class="mt-2 flex min-h-0 items-center gap-[0.6rem] font-bold" for="to-phone">
					Phone (optional)
				</label>
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="to-phone"
					type="tel"
					bind:value={to.phone}
				/>
			</fieldset>
		</div>

		<div class="flex flex-wrap gap-4">
			<div class="flex flex-col gap-1">
				<label class="font-bold" for="parcel">Parcel</label>
				<select
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="parcel"
					bind:value={parcel}
				>
					{#each PARCELS as [value, label]}
						<option {value}>{label}</option>
					{/each}
				</select>
			</div>
			<div class="flex flex-col gap-1">
				<label class="font-bold" for="service">Service</label>
				<select
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
					id="service"
					bind:value={service}
				>
					{#each SERVICES as [value, label]}
						<option {value}>{label}</option>
					{/each}
				</select>
			</div>
		</div>

		<button
			class="mt-3 min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-[1.2rem] py-[0.6rem] font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
			type="submit"
			disabled={busy}>Create shipment</button
		>
	</form>
</section>

<section aria-labelledby="purchase-h">
	<h2 id="purchase-h">2. Purchase a label</h2>
	{#if shipmentId === null}
		<p class="rounded-sm border border-line bg-surface px-4 py-3">Create a shipment first.</p>
	{:else}
		<p>
			Shipment {shipmentId} · status: <strong>{shipmentStatus || 'draft'}</strong>
			<button
				type="button"
				class="m-0 ml-2 min-h-0 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-[0.6rem] py-1 font-bold text-accent-ink"
				onclick={refresh}>Refresh</button
			>
		</p>
		<form
			class="flex max-w-[44rem] flex-col gap-2"
			onsubmit={(e) => {
				e.preventDefault();
				purchase();
			}}
		>
			<fieldset class="rounded-sm border border-line px-4 py-3">
				<legend class="px-1 font-bold">Label option</legend>
				<label class="flex min-h-11 items-center gap-[0.6rem] font-bold">
					<input type="radio" name="kind" value="printable" bind:group={kind} />
					A — Printable address label (you affix postage)
				</label>
				<label class="flex min-h-11 items-center gap-[0.6rem] font-bold">
					<input type="radio" name="kind" value="prepaid" bind:group={kind} />
					B — Prepaid USPS label (demo purchase, no live postage)
				</label>
			</fieldset>

			<div class="flex flex-wrap gap-4">
				<div class="flex flex-col gap-1">
					<label class="font-bold" for="direction">Direction</label>
					<select
						class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
						id="direction"
						bind:value={direction}
					>
						<option value="outbound">Outbound (you → authority)</option>
						<option value="return">Return (authority → you)</option>
					</select>
				</div>
				{#if kind === 'printable'}
					<div class="flex flex-col gap-1">
						<label class="font-bold" for="carrier">Carrier</label>
						<select
							class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
							id="carrier"
							bind:value={carrier}
						>
							<option value="usps">USPS</option>
							<option value="fed_ex">FedEx</option>
						</select>
					</div>
					<div class="flex flex-col gap-1">
						<label class="font-bold" for="format">Layout</label>
						<select
							class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
							id="format"
							bind:value={format}
						>
							{#each FORMATS as [value, label]}
								<option {value}>{label}</option>
							{/each}
						</select>
					</div>
				{:else}
					<div class="flex flex-col gap-1">
						<span class="font-bold">Carrier</span>
						<p class="mt-1 mb-0">USPS (prepaid is USPS-only)</p>
					</div>
					<div class="flex flex-col gap-1">
						<label class="font-bold" for="paper">Paper size</label>
						<select
							class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
							id="paper"
							bind:value={paper}
						>
							{#each PAPERS as [value, label]}
								<option {value}>{label}</option>
							{/each}
						</select>
					</div>
				{/if}
			</div>

			<div class="flex flex-wrap gap-3">
				<button
					class="mt-3 min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-[1.2rem] py-[0.6rem] font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
					type="submit"
					disabled={busy}>Purchase label</button
				>
				<button
					class="mt-3 min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-[1.2rem] py-[0.6rem] font-bold text-ink disabled:cursor-wait disabled:opacity-60"
					type="button"
					disabled={busy}
					onclick={saveDefaults}
				>
					Save choices as my defaults
				</button>
			</div>
		</form>
	{/if}
</section>

<section aria-labelledby="labels-h">
	<h2 id="labels-h">3. Labels</h2>
	{#if labels.length === 0}
		<p class="rounded-sm border border-line bg-surface px-4 py-3">No labels yet for this shipment.</p>
	{:else}
		<ul class="m-0 flex list-none flex-col gap-4 p-0">
			{#each labels as label (label.id)}
				<li class="rounded-sm border border-line px-4 py-3">
					<p>
						<strong>#{label.id}</strong>
						{label.summary.service_name} ·
						{label.summary.direction} ·
						{label.purchase_option === 'printable'
							? `printable, ${formatName(label.label_format)}`
							: `prepaid, ${label.label_format}`} ·
						<strong>{label.summary.status}</strong>
					</p>
					<p>
						{#if label.summary.tracking_no}
							Tracking: {label.summary.tracking_no}
							{#if label.summary.tracking_url}
								<a class="ml-3" href={label.summary.tracking_url} target="_blank" rel="noreferrer">
									Track
								</a>
							{/if}
						{:else}
							No tracking yet.
						{/if}
						{#if label.summary.label_url}
							<a class="ml-3" href={downloadHref(label.id)} download>Download</a>
						{/if}
					</p>
					{#if label.summary.status === 'pending' || label.summary.status === 'voided'}
						<form
							class="mt-2 flex max-w-[28rem] flex-col gap-2"
							onsubmit={(e) => {
								e.preventDefault();
								pasteBack(label.id);
							}}
						>
							<label class="font-bold" for="track-{label.id}">Tracking number</label>
							<input
								class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
								id="track-{label.id}"
								type="text"
								bind:value={pasteTracking[label.id]}
								placeholder="Pasted back by staff"
							/>
							<label class="font-bold" for="file-{label.id}">Label PDF</label>
							<input
								class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
								id="file-{label.id}"
								type="file"
								accept="application/pdf"
								onchange={(e) => pickFile(label.id, e)}
							/>
							<button
								class="mt-3 min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-[1.2rem] py-[0.6rem] font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
								type="submit"
								disabled={busy}>Paste back</button
							>
						</form>
					{/if}
					{#if label.summary.status === 'issued'}
						<button
							type="button"
							class="mt-3 min-h-11 cursor-pointer rounded-sm border border-line bg-surface px-[1.2rem] py-[0.6rem] font-bold text-ink disabled:cursor-wait disabled:opacity-60"
							disabled={busy}
							onclick={() => voidLabel(label.id)}
						>
							Void label
						</button>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</section>

