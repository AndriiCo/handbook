<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { validateField, validateSubmission } from '../../../lib/customer/forms-validate.js';
	import { FORMS_COPY } from '../../../lib/customer/forms-copy.js';
	import { site } from '../../../lib/locale.svelte.js';

	// Guided in-browser fill (P2-U1): pick authority + form, fill with
	// inline validation (TS port; the server re-validates), review,
	// submit. Stale/conflict/correction states inline (U2/U4/U5).
	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	// Page words follow the site language; messages are kept as
	// functions of the copy so a language switch re-words them too.
	const c = $derived(FORMS_COPY[site.locale]);
	type Msg = (copy: typeof c) => string;
	/** A field error: `code` re-words it outside English (forms-copy.js). */
	type FieldErr = { code?: string; message: string };
	const ADDR_PARTS = ['name', 'street', 'city', 'state', 'zip'] as const;

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
		message: string | null;
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
	let errors = $state<Record<string, FieldErr>>({});
	let loadError = $state<Msg | null>(null);
	let submitError = $state<Msg | null>(null);
	let stale = $state<StaleInfo | null>(null);
	let draftId = $state<number | null>(null);
	let busy = $state(false);
	let correctionFor = $state<string | null>(null);
	let correctionNote = $state('');
	let correctionDone = $state<Msg | null>(null);

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
		if (found.length > 0) errors[key] = found[0];
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
				loadError = (k) => k.noActiveForm;
				return;
			}
			if (res.status === 409) {
				const data = await res.json();
				const text: unknown = data?.error;
				loadError = typeof text === 'string' ? () => text : (k) => k.underReverification;
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
			loadError = (k) => k.unreachable;
		} finally {
			busy = false;
		}
	}

	function backToReview(): void {
		submitError = null;
		if (!form) return;
		const found = validateSubmission(form.fields, values);
		const map: Record<string, FieldErr> = {};
		for (const e of found) {
			if (!map[e.key]) map[e.key] = e;
		}
		errors = map;
		if (found.length > 0) {
			submitError = (k) => k.needsAttention;
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
			submitError = (k) => k.draftFailed;
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
				const map: Record<string, FieldErr> = {};
				for (const e of data.errors ?? []) {
					if (!map[e.key]) map[e.key] = { code: e.code, message: e.message };
				}
				errors = map;
				step = 'fill';
				submitError = (k) => k.serverRejected;
				return;
			}
			if (res.status === 409) {
				const data = await res.json();
				stale = {
					message: typeof data?.error === 'string' ? data.error : null,
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
			submitError = (k) => k.submitFailedBody;
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
			correctionDone = (k) => k.correctionThanks;
			correctionFor = null;
			correctionNote = '';
		} catch {
			correctionDone = (k) => k.correctionFailed;
		} finally {
			busy = false;
		}
	}

	function displayValue(field: FieldDef): string {
		const v = values[field.key];
		if (v === undefined || v === null || v === '') return '—';
		if (typeof v === 'boolean') return v ? c.yesValue : c.noValue;
		if (typeof v === 'object') {
			return Object.entries(v as Record<string, unknown>)
				.map(([k, val]) => `${k}: ${val}`)
				.join('; ');
		}
		return String(v);
	}
</script>

<svelte:head>
	<title>{c.title} — Apostille Handbook</title>
</svelte:head>

<h1>{c.heading}</h1>

{#if step === 'pick'}
	<p class="max-w-[40rem] text-ink-muted">{c.pickIntro}</p>
	<form
		class="flex max-w-[36rem] flex-col gap-3"
		onsubmit={(e) => {
			e.preventDefault();
			loadForm();
		}}
	>
		<label for="authority">{c.authority}</label>
		<input id="authority" name="authority" type="text" bind:value={authority} required />
		<label for="code">{c.formCode}</label>
		<input id="code" name="code" type="text" bind:value={code} required />
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
			type="submit"
			disabled={busy}>{busy ? c.loading : c.loadForm}</button
		>
	</form>
	{#if loadError}
		<Notice variant="error" heading={c.cannotLoad}><p>{loadError(c)}</p></Notice>
	{/if}
{/if}

{#if step !== 'pick' && form}
	<div class="mb-4 max-w-[40rem]">
		<h2>{form.title}</h2>
		<p class="text-sm text-ink-muted">
			{c.edition(form.code, form.authority_revision, form.version)}
		</p>
		{#if form.replica_notice}
			<Notice variant={form.accepts_replica === 'no' ? 'warning' : 'info'} heading={c.aboutPrintout}>
				<p>{form.replica_notice}</p>
			</Notice>
		{/if}
	</div>

	{#if stale}
		<Notice variant="warning" heading={c.formUpdated}>
			<p>{stale.message ?? c.superseded}</p>
			{#if stale.added.length > 0}<p>{c.added(stale.added.join(', '))}</p>{/if}
			{#if stale.removed.length > 0}<p>{c.removed(stale.removed.join(', '))}</p>{/if}
			{#if stale.changed.length > 0}<p>{c.changed(stale.changed.join(', '))}</p>{/if}
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
				type="button"
				onclick={() => loadForm(values)}
				disabled={busy}
			>
				{c.carryOver(stale.new_version)}
			</button>
		</Notice>
	{/if}
{/if}

{#if step === 'fill' && form}
	<form
		class="flex max-w-[36rem] flex-col gap-3"
		onsubmit={(e) => {
			e.preventDefault();
			backToReview();
		}}
	>
		{#each form.fields as field (field.key)}
			<div class="flex flex-col gap-[0.35rem] border-b border-line pb-3">
				<label class="font-bold" for={fid(field)}>
					{field.label}{field.required ? ' *' : ''}
				</label>
				{#if field.help}<p class="m-0 text-sm text-ink-muted">{field.help}</p>{/if}
				{#if prefilled[field.key] !== undefined}
					<p class="m-0 text-sm text-ink-muted">
						{c.prefilledFrom(prefilled[field.key])}
					</p>
				{/if}
				{#if withheldByKey[field.key]}
					<p class="m-0 text-sm font-bold">
						{withheldByKey[field.key].reason}. {c.typeManually}
						<button
							type="button"
							class="inline cursor-pointer border-0 bg-transparent p-0 text-inherit underline"
							onclick={() => {
								correctionFor = field.key;
								correctionNote = '';
							}}
						>
							{c.reportIt}
						</button>.
					</p>
					{#if correctionFor === field.key}
						<label class="font-bold" for={fid(field, '-note')}>{c.whatLooksWrong}</label>
						<textarea
							class="min-h-11 rounded-sm border border-line bg-surface px-[0.6rem] py-2 text-ink"
							id={fid(field, '-note')}
							rows="2"
							maxlength="2000"
							bind:value={correctionNote}
						></textarea>
						<button
							class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
							type="button"
							onclick={() => reportCorrection(field)}
							disabled={busy}
						>
							{c.sendReport}
						</button>
					{/if}
				{/if}

				{#if field.type === 'textarea'}
					<textarea
						class="min-h-11 rounded-sm border border-line bg-surface px-[0.6rem] py-2 text-ink"
						id={fid(field)}
						rows="4"
						maxlength={field.max_len}
						value={strVal(field.key)}
						oninput={(e) => setValue(field.key, e.currentTarget.value)}
						aria-invalid={errors[field.key] ? 'true' : undefined}
					></textarea>
				{:else if field.type === 'select'}
					<select
						class="min-h-11 rounded-sm border border-line bg-surface px-[0.6rem] py-2 text-ink"
						id={fid(field)}
						value={strVal(field.key)}
						onchange={(e) => setValue(field.key, e.currentTarget.value)}
						aria-invalid={errors[field.key] ? 'true' : undefined}
					>
						<option value="">{c.choose}</option>
						{#each field.options ?? [] as option (option)}
							<option value={option}>{option}</option>
						{/each}
					</select>
				{:else if field.type === 'radio'}
					<fieldset class="m-0 border-0 p-0">
						<legend class="sr-only">{field.label}</legend>
						{#each field.options ?? [] as option (option)}
							<label class="flex min-h-11 items-center gap-2 font-normal">
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
					<label class="flex min-h-11 items-center gap-2 font-normal">
						<input
							id={fid(field)}
							type="checkbox"
							checked={values[field.key] === true}
							onchange={(e) => setValue(field.key, e.currentTarget.checked)}
						/>
						{c.yes}
					</label>
				{:else if field.type === 'address_block'}
					<fieldset class="m-0 grid grid-cols-[5rem_1fr] items-center gap-x-[0.6rem] gap-y-[0.4rem] border-0 p-0">
						<legend class="sr-only">{c.addressLegend(field.label)}</legend>
						{#each ADDR_PARTS as part (part)}
							<label class="font-bold" for={fid(field, `-${part}`)}>{c.addrPart[part]}</label>
							<input
								class="min-h-11 rounded-sm border border-line bg-surface px-[0.6rem] py-2 text-ink"
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
						class="min-h-11 rounded-sm border border-line bg-surface px-[0.6rem] py-2 text-ink"
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
					<p class="m-0 font-bold text-error" role="alert">{c.fieldError(errors[field.key], field)}</p>
				{/if}
			</div>
		{/each}

		{#if submitError}
			<Notice variant="error" heading={c.notYet}><p>{submitError(c)}</p></Notice>
		{/if}
		{#if draftId !== null}
			<Notice variant="success" heading={c.draftSaved}><p>{c.draftStored(draftId)}</p></Notice>
		{/if}
		{#if correctionDone}
			<Notice variant="info" heading={c.reportSent}><p>{correctionDone(c)}</p></Notice>
		{/if}
		<div class="mt-2 flex gap-3">
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
				type="button"
				onclick={saveDraft}
				disabled={busy}>{c.saveDraft}</button
			>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
				type="submit"
				disabled={busy}>{c.review}</button
			>
		</div>
	</form>
{/if}

{#if step === 'review' && form}
	<h2>{c.reviewHeading}</h2>
	<table class="mb-4 max-w-[40rem] border-collapse">
		<tbody>
			{#each form.fields as field (field.key)}
				<tr>
					<th class="border-b border-line px-3 py-2 text-left align-top" scope="row">{field.label}</th>
					<td class="border-b border-line px-3 py-2 text-left align-top">{displayValue(field)}</td>
				</tr>
			{/each}
		</tbody>
	</table>
	{#if form.replica_notice}
		<Notice variant={form.accepts_replica === 'no' ? 'warning' : 'info'} heading={c.aboutPrintout}>
			<p>{form.replica_notice}</p>
		</Notice>
	{/if}
	{#if customerOverride}
		<Notice variant="info" heading={c.overrideHeading}>
			<p>{c.overrideBody}</p>
		</Notice>
	{/if}
	{#if submitError}
		<Notice variant="error" heading={c.submitFailed}><p>{submitError(c)}</p></Notice>
	{/if}
	<div class="mt-2 flex gap-3">
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
			type="button"
			onclick={() => (step = 'fill')}
			disabled={busy}>{c.backToEdit}</button
		>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-4 font-bold text-accent-ink disabled:cursor-wait disabled:opacity-60"
			type="button"
			onclick={submit}
			disabled={busy}
		>
			{busy ? c.submitting : c.submitRequest}
		</button>
	</div>
{/if}
