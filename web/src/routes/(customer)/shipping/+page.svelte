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
	<p class="notice" role="status">{notice}</p>
{/if}
{#if errors.length}
	<ul class="errors" role="alert">
		{#each errors as err}
			<li>{err.field ? `${err.field}: ` : ''}{err.message}</li>
		{/each}
	</ul>
{/if}

<section aria-labelledby="shipment-h">
	<h2 id="shipment-h">1. Shipment</h2>
	<form
		onsubmit={(e) => {
			e.preventDefault();
			createShipment();
		}}
	>
		<label for="authority">Authority ID</label>
		<input id="authority" type="number" min="1" bind:value={authorityId} required />

		<div class="addresses">
			<fieldset>
				<legend>From</legend>
				<label for="from-name">Name</label>
				<input id="from-name" type="text" bind:value={from.name} required autocomplete="name" />
				<label for="from-street">Street</label>
				<input
					id="from-street"
					type="text"
					bind:value={from.street1}
					required
					autocomplete="street-address"
				/>
				<label for="from-street2">Street, line 2 (optional)</label>
				<input id="from-street2" type="text" bind:value={from.street2} />
				<label for="from-city">City</label>
				<input
					id="from-city"
					type="text"
					bind:value={from.city}
					required
					autocomplete="address-level2"
				/>
				<label for="from-state">State</label>
				<input id="from-state" type="text" bind:value={from.state_province} required />
				<label for="from-postal">Postal code</label>
				<input
					id="from-postal"
					type="text"
					bind:value={from.postal}
					required
					autocomplete="postal-code"
				/>
				<label for="from-country">Country (ISO-2)</label>
				<input id="from-country" type="text" bind:value={from.country} required />
				<label for="from-phone">Phone (optional)</label>
				<input id="from-phone" type="tel" bind:value={from.phone} autocomplete="tel" />
			</fieldset>
			<fieldset>
				<legend>To</legend>
				<label for="to-name">Name</label>
				<input id="to-name" type="text" bind:value={to.name} required />
				<label for="to-street">Street</label>
				<input id="to-street" type="text" bind:value={to.street1} required />
				<label for="to-street2">Street, line 2 (optional)</label>
				<input id="to-street2" type="text" bind:value={to.street2} />
				<label for="to-city">City</label>
				<input id="to-city" type="text" bind:value={to.city} required />
				<label for="to-state">State</label>
				<input id="to-state" type="text" bind:value={to.state_province} required />
				<label for="to-postal">Postal code</label>
				<input id="to-postal" type="text" bind:value={to.postal} required />
				<label for="to-country">Country (ISO-2)</label>
				<input id="to-country" type="text" bind:value={to.country} required />
				<label for="to-phone">Phone (optional)</label>
				<input id="to-phone" type="tel" bind:value={to.phone} />
			</fieldset>
		</div>

		<div class="row">
			<div>
				<label for="parcel">Parcel</label>
				<select id="parcel" bind:value={parcel}>
					{#each PARCELS as [value, label]}
						<option {value}>{label}</option>
					{/each}
				</select>
			</div>
			<div>
				<label for="service">Service</label>
				<select id="service" bind:value={service}>
					{#each SERVICES as [value, label]}
						<option {value}>{label}</option>
					{/each}
				</select>
			</div>
		</div>

		<button type="submit" disabled={busy}>Create shipment</button>
	</form>
</section>

<section aria-labelledby="purchase-h">
	<h2 id="purchase-h">2. Purchase a label</h2>
	{#if shipmentId === null}
		<p class="notice">Create a shipment first.</p>
	{:else}
		<p>
			Shipment {shipmentId} · status: <strong>{shipmentStatus || 'draft'}</strong>
			<button type="button" class="linklike" onclick={refresh}>Refresh</button>
		</p>
		<form
			onsubmit={(e) => {
				e.preventDefault();
				purchase();
			}}
		>
			<fieldset>
				<legend>Label option</legend>
				<label>
					<input type="radio" name="kind" value="printable" bind:group={kind} />
					A — Printable address label (you affix postage)
				</label>
				<label>
					<input type="radio" name="kind" value="prepaid" bind:group={kind} />
					B — Prepaid USPS label (postage included)
				</label>
			</fieldset>

			<div class="row">
				<div>
					<label for="direction">Direction</label>
					<select id="direction" bind:value={direction}>
						<option value="outbound">Outbound (you → authority)</option>
						<option value="return">Return (authority → you)</option>
					</select>
				</div>
				{#if kind === 'printable'}
					<div>
						<label for="carrier">Carrier</label>
						<select id="carrier" bind:value={carrier}>
							<option value="usps">USPS</option>
							<option value="fed_ex">FedEx</option>
						</select>
					</div>
					<div>
						<label for="format">Layout</label>
						<select id="format" bind:value={format}>
							{#each FORMATS as [value, label]}
								<option {value}>{label}</option>
							{/each}
						</select>
					</div>
				{:else}
					<div>
						<span class="static-label">Carrier</span>
						<p class="static-value">USPS (prepaid is USPS-only)</p>
					</div>
					<div>
						<label for="paper">Paper size</label>
						<select id="paper" bind:value={paper}>
							{#each PAPERS as [value, label]}
								<option {value}>{label}</option>
							{/each}
						</select>
					</div>
				{/if}
			</div>

			<div class="actions">
				<button type="submit" disabled={busy}>Purchase label</button>
				<button type="button" class="secondary" disabled={busy} onclick={saveDefaults}>
					Save choices as my defaults
				</button>
			</div>
		</form>
	{/if}
</section>

<section aria-labelledby="labels-h">
	<h2 id="labels-h">3. Labels</h2>
	{#if labels.length === 0}
		<p class="notice">No labels yet for this shipment.</p>
	{:else}
		<ul class="labels">
			{#each labels as label (label.id)}
				<li>
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
								<a href={label.summary.tracking_url} target="_blank" rel="noreferrer">
									Track
								</a>
							{/if}
						{:else}
							No tracking yet.
						{/if}
						{#if label.summary.label_url}
							<a href={downloadHref(label.id)} download>Download</a>
						{/if}
					</p>
					{#if label.summary.status === 'pending' || label.summary.status === 'voided'}
						<form
							class="pasteback"
							onsubmit={(e) => {
								e.preventDefault();
								pasteBack(label.id);
							}}
						>
							<label for="track-{label.id}">Tracking number</label>
							<input
								id="track-{label.id}"
								type="text"
								bind:value={pasteTracking[label.id]}
								placeholder="Pasted back by staff"
							/>
							<label for="file-{label.id}">Label PDF</label>
							<input
								id="file-{label.id}"
								type="file"
								accept="application/pdf"
								onchange={(e) => pickFile(label.id, e)}
							/>
							<button type="submit" disabled={busy}>Paste back</button>
						</form>
					{/if}
					{#if label.summary.status === 'issued'}
						<button
							type="button"
							class="secondary"
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

<style>
	.addresses {
		display: grid;
		grid-template-columns: 1fr;
		gap: 1rem;
	}
	@media (min-width: 48rem) {
		.addresses {
			grid-template-columns: 1fr 1fr;
		}
	}
	form {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		max-width: 44rem;
	}
	fieldset {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
	fieldset label {
		display: flex;
		align-items: center;
		gap: 0.6rem;
		min-height: 44px;
	}
	fieldset > label[for] {
		font-weight: 700;
		min-height: 0;
		margin-top: 0.5rem;
	}
	legend {
		font-weight: 700;
		padding: 0 0.25rem;
	}
	label {
		font-weight: 700;
	}
	input[type='text'],
	input[type='tel'],
	input[type='number'],
	input[type='file'],
	select {
		min-height: 44px;
		padding: 0.6rem 0.8rem;
		font: inherit;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		gap: 1rem;
	}
	.row > div {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
	}
	.static-label {
		font-weight: 700;
	}
	.static-value {
		margin: 0.25rem 0 0;
	}
	button {
		min-height: 44px;
		margin-top: 0.75rem;
		padding: 0.6rem 1.2rem;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
	}
	button:disabled {
		opacity: 0.6;
		cursor: wait;
	}
	button.secondary {
		color: var(--text);
		background: var(--surface);
		border-color: var(--border);
	}
	.linklike {
		min-height: 0;
		margin: 0 0 0 0.5rem;
		padding: 0.25rem 0.6rem;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 0.75rem;
	}
	.notice {
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
	.errors {
		background: var(--surface);
		border: 1px solid var(--danger, #b3261e);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem 0.75rem 2rem;
	}
	.labels {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}
	.labels > li {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
	.labels a {
		margin-left: 0.75rem;
	}
	.pasteback {
		max-width: 28rem;
		margin-top: 0.5rem;
	}
</style>
