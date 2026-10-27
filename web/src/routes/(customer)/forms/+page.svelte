<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { validateField, validateSubmission } from '../../../lib/customer/forms-validate.js';

	// Guided in-browser fill (P2-U1): pick authority + form, fill with
	// inline validation (TS port; the server re-validates), review,
	// submit. Stale/conflict/correction states inline (U2/U4/U5).
	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	interface FieldDef {
		key: string;
		label: string;
		help?: string;
		type: string;
		required: boolean;
		max_len?: number;
		pattern?: string;
		options?: string[];
		prefill_topic?: string;
	}

	interface Withheld {
		key: string;
		topic: string;
		fact_ids: number[];
		reason: string;
	}

	interface FormData {
		id: number;
		authority: string;
		code: string;
		title: string;
		version: number;
		authority_revision: string;
		fields: FieldDef[];
		accepts_replica: string;
		prefill: Record<string, { value: unknown; fact_id: number }>;
		withheld: Withheld[];
		replica_notice: string | null;
		replica_unconfirmed: boolean;
	}

	interface StaleInfo {
		message: string;
		new_version: number;
		added: string[];
		removed: string[];
		changed: string[];
	}

	type Step = 'pick' | 'fill' | 'review';

	let step = $state<Step>('pick');
	let authority = $state('usa-wa-sos');
	let code = $state('wa-apostille-request');
	let form = $state<FormData | null>(null);
	let values = $state<Record<string, unknown>>({});
	let prefilled = $state<Record<string, number>>({});
	let errors = $state<Record<string, string>>({});
	let loadError = $state<string | null>(null);
	let submitError = $state<string | null>(null);
	let stale = $state<StaleInfo | null>(null);
	let draftId = $state<number | null>(null);
	let busy = $state(false);
	let correctionFor = $state<string | null>(null);
	let correctionNote = $state('');
	let correctionDone = $state<string | null>(null);

	const withheldByKey = $derived.by(() => {
		const map: Record<string, Withheld> = {};
		for (const w of form?.withheld ?? []) map[w.key] = w;
		return map;
	});

	const customerOverride = $derived.by(() => {
		for (const key of Object.keys(prefilled)) {
			const current = values[key];
			const original = form?.prefill[key]?.value;
			if (JSON.stringify(current) !== JSON.stringify(original)) return true;
		}
		return Object.keys(withheldByKey).some(
			(key) => values[key] !== undefined && values[key] !== ''
		);
	});

	function fid(field: FieldDef, suffix = '') {
		return `f-${field.key}${suffix}`;
	}

	function strVal(key: string): string {
		const v = values[key];
		return typeof v === 'string' ? v : ((v ?? '') as object).toString();
	}

	function addrVal(key: string): Record<string, string> {
		const v = values[key];
		if (v && typeof v === 'object' && !Array.isArray(v)) {
			return v as Record<string, string>;
		}
		return { name: '', street: '', city: '', state: '', zip: '' };
	}

	function revalidate(key: string) {
		const field = form?.fields.find((f) => f.key === key);
		if (!field) return;
		const found = validateField(field, values[key]);
		if (found.length > 0) errors[key] = found[0].message;
		else delete errors[key];
		errors = { ...errors };
	}

	function setValue(key: string, value: unknown) {
		values[key] = value;
		values = { ...values };
		revalidate(key);
	}

	function addrAutocomplete(part: string): "name" | "street-address" | "address-level2" | "address-level1" | "postal-code" {
		switch (part) {
			case "street":
				return "street-address";
			case "city":
				return "address-level2";
			case "state":
				return "address-level1";
			case "zip":
				return "postal-code";
			default:
				return "name";
		}
	}

	function setAddr(key: string, part: string, value: string) {
		values[key] = { ...addrVal(key), [part]: value };
		values = { ...values };
		revalidate(key);
	}

	async function loadForm(carry: Record<string, unknown> = {}) {
		loadError = null;
		submitError = null;
		stale = null;
		draftId = null;
		busy = true;
		try {
			const res = await fetch(api(`/p2/forms/${authority}/${code}`), {
				credentials: 'same-origin'
			});
			if (res.status === 404) {
				loadError = 'No active form for that authority and code.';
				return;
			}
			if (res.status === 409) {
				const data = await res.json();
				loadError = data?.error ?? 'This form is under re-verification. Please try again later.';
				return;
			}
			if (!res.ok) throw new Error(String(res.status));
			const data = (await res.json()) as FormData;
			form = data;
			const next: Record<string, unknown> = {};
			const marks: Record<string, number> = {};
			for (const [key, p] of Object.entries(data.prefill)) {
				next[key] = structuredClone(p.value);
				marks[key] = p.fact_id;
			}
			// U2 carry-over: keep entered values where keys still match.
			for (const field of data.fields) {
				if (carry[field.key] !== undefined) next[field.key] = carry[field.key];
			}
			values = next;
			prefilled = marks;
			errors = {};
			step = 'fill';
		} catch {
			loadError = 'Form service unreachable — is the API server running?';
		} finally {
			busy = false;
		}
	}

	function backToReview(): void {
		submitError = null;
		if (!form) return;
		const found = validateSubmission(form.fields, values);
		const map: Record<string, string> = {};
		for (const e of found) {
			if (!map[e.key]) map[e.key] = e.message;
		}
		errors = map;
		if (found.length > 0) {
			submitError = 'Some fields need attention before review.';
			return;
		}
		step = 'review';
	}

	async function saveDraft() {
		if (!form) return;
		submitError = null;
		busy = true;
		try {
			const res = await fetch(api('/p2/submissions'), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					template_id: form.id,
					draft: true,
					values,
					customer_override: customerOverride
				})
			});
			if (!res.ok) throw new Error(String(res.status));
			const data = await res.json();
			draftId = data.id;
		} catch {
			submitError = 'Could not save the draft. Nothing was lost — your entries are still above.';
		} finally {
			busy = false;
		}
	}

	async function submit() {
		if (!form) return;
		submitError = null;
		stale = null;
		busy = true;
		try {
			const res = await fetch(api('/p2/submissions'), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					template_id: form.id,
					values,
					customer_override: customerOverride
				})
			});
			if (res.status === 201) {
				const data = await res.json();
				window.location.assign(`${apiRoot}/forms/${data.id}`);
				return;
			}
			if (res.status === 422) {
				const data = await res.json();
				const map: Record<string, string> = {};
				for (const e of data.errors ?? []) {
					if (!map[e.key]) map[e.key] = e.message;
				}
				errors = map;
				step = 'fill';
				submitError = 'The server rejected some fields. Your entries are preserved.';
				return;
			}
			if (res.status === 409) {
				const data = await res.json();
				stale = {
					message: data?.error ?? 'This form edition was superseded.',
					new_version: data?.new_version ?? 0,
					added: data?.changed_keys?.added ?? [],
					removed: data?.changed_keys?.removed ?? [],
					changed: data?.changed_keys?.changed ?? []
				};
				step = 'fill';
				return;
			}
			throw new Error(String(res.status));
		} catch {
			submitError = 'Submission failed. Nothing was stored — please try again.';
		} finally {
			busy = false;
		}
	}

	async function reportCorrection(field: FieldDef) {
		if (!form || correctionNote.trim() === '') return;
		correctionDone = null;
		busy = true;
		try {
			const res = await fetch(api('/p2/corrections'), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					template_id: form.id,
					topic: withheldByKey[field.key]?.topic ?? field.prefill_topic ?? field.key,
					note: correctionNote.trim()
				})
			});
			if (!res.ok) throw new Error(String(res.status));
			correctionDone = 'Thanks — staff will re-check this value. The form stays usable.';
			correctionFor = null;
			correctionNote = '';
		} catch {
			correctionDone = 'Could not send the report. Please try again.';
		} finally {
			busy = false;
		}
	}

	function displayValue(field: FieldDef): string {
		const v = values[field.key];
		if (v === undefined || v === null || v === '') return '—';
		if (typeof v === 'boolean') return v ? 'yes' : 'no';
		if (typeof v === 'object') {
			return Object.entries(v as Record<string, unknown>)
				.map(([k, val]) => `${k}: ${val}`)
				.join('; ');
		}
		return String(v);
	}
</script>

<svelte:head>
	<title>Forms — Apostille Handbook</title>
</svelte:head>

<h1>Apostille request form</h1>

{#if step === 'pick'}
	<p class="lede">
		Pick the issuing authority and the form. Verified addresses and fees
		come prefilled; the server re-checks everything on submit.
	</p>
	<form
		class="pick"
		onsubmit={(e) => {
			e.preventDefault();
			loadForm();
		}}
	>
		<label for="authority">Authority</label>
		<input id="authority" name="authority" type="text" bind:value={authority} required />
		<label for="code">Form code</label>
		<input id="code" name="code" type="text" bind:value={code} required />
		<button type="submit" disabled={busy}>{busy ? 'Loading…' : 'Load form'}</button>
	</form>
	{#if loadError}
		<Notice variant="error" heading="Cannot load this form."><p>{loadError}</p></Notice>
	{/if}
{/if}

{#if step !== 'pick' && form}
	<div class="formhead">
		<h2>{form.title}</h2>
		<p class="meta">
			{form.code} · edition {form.authority_revision} (v{form.version}) · wet-ink signature:
			print and sign after submitting
		</p>
		{#if form.replica_notice}
			<Notice variant={form.accepts_replica === 'no' ? 'warning' : 'info'} heading="About your printout.">
				<p>{form.replica_notice}</p>
			</Notice>
		{/if}
	</div>

	{#if stale}
		<Notice variant="warning" heading="This form was updated.">
			<p>{stale.message}</p>
			{#if stale.added.length > 0}<p>Added: {stale.added.join(', ')}</p>{/if}
			{#if stale.removed.length > 0}<p>Removed: {stale.removed.join(', ')}</p>{/if}
			{#if stale.changed.length > 0}<p>Changed: {stale.changed.join(', ')}</p>{/if}
			<button type="button" onclick={() => loadForm(values)} disabled={busy}>
				Load v{stale.new_version} and carry my entries over
			</button>
		</Notice>
	{/if}
{/if}

{#if step === 'fill' && form}
	<form
		class="fill"
		onsubmit={(e) => {
			e.preventDefault();
			backToReview();
		}}
	>
		{#each form.fields as field (field.key)}
			<div class="field">
				<label for={fid(field)}>
					{field.label}{field.required ? ' *' : ''}
				</label>
				{#if field.help}<p class="help">{field.help}</p>{/if}
				{#if prefilled[field.key] !== undefined}
					<p class="prefill">Prefilled from verified record #{prefilled[field.key]} — fix it if it is wrong.</p>
				{/if}
				{#if withheldByKey[field.key]}
					<p class="withheld">
						{withheldByKey[field.key].reason}. Type it manually, or
						<button
							type="button"
							class="link"
							onclick={() => {
								correctionFor = field.key;
								correctionNote = '';
							}}
						>
							report it
						</button>.
					</p>
					{#if correctionFor === field.key}
						<label for={fid(field, '-note')}>What looks wrong?</label>
						<textarea
							id={fid(field, '-note')}
							rows="2"
							maxlength="2000"
							bind:value={correctionNote}
						></textarea>
						<button type="button" onclick={() => reportCorrection(field)} disabled={busy}>
							Send report
						</button>
					{/if}
				{/if}

				{#if field.type === 'textarea'}
					<textarea
						id={fid(field)}
						rows="4"
						maxlength={field.max_len}
						value={strVal(field.key)}
						oninput={(e) => setValue(field.key, e.currentTarget.value)}
						aria-invalid={errors[field.key] ? 'true' : undefined}
					></textarea>
				{:else if field.type === 'select'}
					<select
						id={fid(field)}
						value={strVal(field.key)}
						onchange={(e) => setValue(field.key, e.currentTarget.value)}
						aria-invalid={errors[field.key] ? 'true' : undefined}
					>
						<option value="">Choose…</option>
						{#each field.options ?? [] as option (option)}
							<option value={option}>{option}</option>
						{/each}
					</select>
				{:else if field.type === 'radio'}
					<fieldset>
						<legend class="visually-hidden">{field.label}</legend>
						{#each field.options ?? [] as option (option)}
							<label class="choice">
								<input
									type="radio"
									name={fid(field)}
									value={option}
									checked={strVal(field.key) === option}
									onchange={() => setValue(field.key, option)}
								/>
								{option}
							</label>
						{/each}
					</fieldset>
				{:else if field.type === 'checkbox'}
					<label class="choice">
						<input
							id={fid(field)}
							type="checkbox"
							checked={values[field.key] === true}
							onchange={(e) => setValue(field.key, e.currentTarget.checked)}
						/>
						Yes
					</label>
				{:else if field.type === 'address_block'}
					<fieldset class="addr">
						<legend class="visually-hidden">{field.label} address</legend>
						{#each ['name', 'street', 'city', 'state', 'zip'] as part (part)}
							<label for={fid(field, `-${part}`)}>{part}</label>
							<input
								id={fid(field, `-${part}`)}
								type="text"
								value={addrVal(field.key)[part] ?? ''}
								oninput={(e) => setAddr(field.key, part, e.currentTarget.value)}
								autocomplete={addrAutocomplete(part)}
							/>
						{/each}
					</fieldset>
				{:else}
					<input
						id={fid(field)}
						type={field.type === 'date'
							? 'date'
							: field.type === 'number' || field.type === 'money'
								? 'text'
								: field.type}
						inputmode={field.type === 'money' || field.type === 'number' ? 'decimal' : undefined}
						maxlength={field.max_len}
						value={strVal(field.key)}
						oninput={(e) => setValue(field.key, e.currentTarget.value)}
						aria-invalid={errors[field.key] ? 'true' : undefined}
					/>
				{/if}
				{#if errors[field.key]}
					<p class="error" role="alert">{errors[field.key]}</p>
				{/if}
			</div>
		{/each}

		{#if submitError}
			<Notice variant="error" heading="Not yet."><p>{submitError}</p></Notice>
		{/if}
		{#if draftId !== null}
			<Notice variant="success" heading="Draft saved."><p>Draft #{draftId} is stored; submit it from the review step.</p></Notice>
		{/if}
		{#if correctionDone}
			<Notice variant="info" heading="Report sent."><p>{correctionDone}</p></Notice>
		{/if}
		<div class="actions">
			<button type="button" onclick={saveDraft} disabled={busy}>Save draft</button>
			<button type="submit" disabled={busy}>Review</button>
		</div>
	</form>
{/if}

{#if step === 'review' && form}
	<h2>Review</h2>
	<table>
		<tbody>
			{#each form.fields as field (field.key)}
				<tr>
					<th scope="row">{field.label}</th>
					<td>{displayValue(field)}</td>
				</tr>
			{/each}
		</tbody>
	</table>
	{#if form.replica_notice}
		<Notice variant={form.accepts_replica === 'no' ? 'warning' : 'info'} heading="About your printout.">
			<p>{form.replica_notice}</p>
		</Notice>
	{/if}
	{#if customerOverride}
		<Notice variant="info" heading="You edited a prefilled value.">
			<p>Staff will see the override flag on this submission.</p>
		</Notice>
	{/if}
	{#if submitError}
		<Notice variant="error" heading="Submit failed."><p>{submitError}</p></Notice>
	{/if}
	<div class="actions">
		<button type="button" onclick={() => (step = 'fill')} disabled={busy}>Back to edit</button>
		<button type="button" onclick={submit} disabled={busy}>
			{busy ? 'Submitting…' : 'Submit request'}
		</button>
	</div>
{/if}

<style>
	.lede {
		color: var(--text-muted);
		max-width: 40rem;
	}
	.pick,
	.fill {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		max-width: 36rem;
	}
	.formhead {
		max-width: 40rem;
		margin-bottom: 1rem;
	}
	.meta {
		color: var(--text-muted);
		font-size: 0.875rem;
	}
	.field {
		display: flex;
		flex-direction: column;
		gap: 0.35rem;
		padding-bottom: 0.75rem;
		border-bottom: 1px solid var(--border);
	}
	.field label {
		font-weight: 700;
	}
	.field input[type='text'],
	.field input[type='email'],
	.field input[type='tel'],
	.field input[type='date'],
	.field select,
	.field textarea {
		font: inherit;
		min-height: 44px;
		padding: 0.5rem 0.6rem;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		background: var(--surface);
		color: var(--text);
	}
	.help {
		margin: 0;
		font-size: 0.875rem;
		color: var(--text-muted);
	}
	.prefill,
	.withheld {
		margin: 0;
		font-size: 0.875rem;
	}
	.prefill {
		color: var(--text-muted);
	}
	.withheld {
		font-weight: 700;
	}
	.error {
		margin: 0;
		font-weight: 700;
		color: var(--error);
	}
	.choice {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		font-weight: 400;
		min-height: 44px;
	}
	fieldset {
		border: none;
		margin: 0;
		padding: 0;
	}
	.addr {
		display: grid;
		grid-template-columns: 5rem 1fr;
		gap: 0.4rem 0.6rem;
		align-items: center;
	}
	.visually-hidden {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip: rect(0 0 0 0);
	}
	.link {
		all: unset;
		text-decoration: underline;
		cursor: pointer;
		color: inherit;
	}
	table {
		border-collapse: collapse;
		max-width: 40rem;
		margin-bottom: 1rem;
	}
	th,
	td {
		border-bottom: 1px solid var(--border);
		padding: 0.5rem 0.75rem;
		text-align: left;
		vertical-align: top;
	}
	.actions {
		display: flex;
		gap: 0.75rem;
		margin-top: 0.5rem;
	}
	button {
		min-height: 44px;
		padding: 0 1rem;
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
</style>
