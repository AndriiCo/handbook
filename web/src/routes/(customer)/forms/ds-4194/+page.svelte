<script lang="ts">
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import AddressBlock from '../../../../lib/forms/AddressBlock.svelte';
	import CountrySelect from '../../../../lib/forms/CountrySelect.svelte';
	import PhoneInput from '../../../../lib/forms/PhoneInput.svelte';
	import {
		errorsForKey,
		isValid,
		validateSubmission
	} from '../../../../lib/forms/validation.js';
	import templateData from '../../../../lib/forms/generated/us-dos__ds-4194.json';

	// DS-4194 resemblance pilot (form-framework.md §6): fill with
	// inline framework validation, review, US-Letter print. The
	// template is a static generated copy — the route works offline
	// and posts nothing: DOS submission semantics are unverified
	// (REQUIREMENTS.md TBD), so printing the filled page is the
	// only output. Wet-ink signature lines render for print only.

	interface TemplateField {
		key: string;
		label: string;
		help?: string;
		type: string;
		required: boolean;
		max_len?: number;
		pattern?: string;
		options?: string[];
	}

	interface Template {
		authority: string;
		code: string;
		title: string;
		version: number;
		authority_revision: string;
		accepts_replica: string;
		fields: TemplateField[];
	}

	interface Section {
		title: string;
		note?: string;
		keys: string[];
	}

	const template = templateData as Template;

	// Per-form section map (REQUIREMENTS.md §"Section map").
	// Checked against the template keys below; unknown keys warn.
	const SECTIONS: Section[] = [
		{
			title: 'Section 1 — Requester contact',
			keys: ['requester_name', 'requester_address', 'requester_phone', 'requester_email']
		},
		{
			title: 'Section 2 — Courier / representative',
			keys: ['on_behalf', 'rep_name', 'rep_phone']
		},
		{
			title: 'Section 3 — Return mailing address',
			keys: ['same_as_above', 'return_address']
		},
		{
			title: 'Section 4 — Document information',
			keys: ['countries_of_use', 'number_of_documents', 'document_type']
		}
	];

	const byKey = new Map(template.fields.map((field) => [field.key, field]));
	for (const section of SECTIONS) {
		for (const key of section.keys) {
			if (!byKey.has(key)) console.warn(`ds-4194: section map key missing from template: ${key}`);
		}
	}
	for (const field of template.fields) {
		if (!SECTIONS.some((section) => section.keys.includes(field.key))) {
			console.warn(`ds-4194: template field missing from section map: ${field.key}`);
		}
	}

	type Step = 'fill' | 'review';
	let step = $state<Step>('fill');
	let showErrors = $state(false);

	function initialValue(field: TemplateField): unknown {
		if (field.type === 'address_block') return {};
		if (field.type === 'checkbox') return false;
		return '';
	}

	// Dynamic per-field state: `any` lets the generic section loop
	// bind each key to its typed component. Validation still runs
	// through the typed validateSubmission over this record.
	let values = $state<Record<string, any>>(
		Object.fromEntries(template.fields.map((field) => [field.key, initialValue(field)]))
	);

	const errors = $derived(validateSubmission(template.fields, values));

	function goReview() {
		showErrors = true;
		if (isValid(errors)) step = 'review';
	}

	function displayValue(field: TemplateField, value: unknown): string {
		if (value === undefined || value === null || value === '') return '—';
		if (typeof value === 'boolean') return value ? 'yes' : 'no';
		if (typeof value === 'object') {
			const parts = Object.entries(value as Record<string, unknown>)
				.filter(([, v]) => v !== undefined && v !== null && v !== '')
				.map(([k, v]) => `${k}: ${v}`);
			return parts.length > 0 ? parts.join('; ') : '—';
		}
		return String(value);
	}

	function printPage() {
		window.print();
	}
</script>

<svelte:head>
	<title>DS-4194 draft resemblance — {template.title}</title>
</svelte:head>

<article class="sheet">
	<header class="masthead">
		<p class="kicker">US Department of State · draft resemblance · not the official form</p>
		<h1>{template.title} (DS-4194)</h1>
		<p class="edition">{template.authority_revision} · template v{template.version}</p>
	</header>

	<div class="no-print">
		<Notice variant="warning" heading="Draft resemblance — verify against the official form">
			<p>
				This page mirrors the paper form's sections for drafting only. It submits
				nothing, asserts no fee, timeline, or acceptance rule, and is not the
				U.S. Department of State. Check every value against the official DS-4194
				before use.
			</p>
		</Notice>
	</div>

	{#if step === 'fill'}
		<form
			onsubmit={(event) => {
				event.preventDefault();
				goReview();
			}}
		>
			{#each SECTIONS as section (section.title)}
				<section>
					<h2>{section.title}</h2>
					{#each section.keys as key (key)}
						{@const field = byKey.get(key)}
						{#if field}
							<div class="field">
								{#if field.type === 'address_block'}
									<span class="label" id={`label-${field.key}`}>
										{field.label}{#if field.required} *{/if}
									</span>
									{#if field.help}<p class="help">{field.help}</p>{/if}
									<AddressBlock
										idPrefix={field.key}
										bind:value={values[field.key]}
										{showErrors}
									/>
								{:else if field.type === 'tel'}
									<label for={field.key}>
										{field.label}{#if field.required} *{/if}
									</label>
									{#if field.help}<p class="help">{field.help}</p>{/if}
									<PhoneInput id={field.key} bind:value={values[field.key]} />
								{:else if field.key === 'countries_of_use'}
									<label for={field.key}>
										{field.label}{#if field.required} *{/if}
									</label>
									{#if field.help}<p class="help">{field.help}</p>{/if}
									<textarea
										id={field.key}
										rows="2"
										maxlength={field.max_len}
										bind:value={values[field.key]}
										placeholder="List each country; one per line"
									></textarea>
									<p class="help">Country picker for single-country use:</p>
									<CountrySelect bind:value={values[field.key]} />
								{:else if field.type === 'textarea'}
									<label for={field.key}>
										{field.label}{#if field.required} *{/if}
									</label>
									{#if field.help}<p class="help">{field.help}</p>{/if}
									<textarea
										id={field.key}
										rows="3"
										maxlength={field.max_len}
										bind:value={values[field.key]}
									></textarea>
								{:else if field.type === 'radio'}
									<span class="label" id={`label-${field.key}`}>
										{field.label}{#if field.required} *{/if}
									</span>
									{#if field.help}<p class="help">{field.help}</p>{/if}
									<div class="choices" role="radiogroup" aria-labelledby={`label-${field.key}`}>
										{#each (field.options ?? []) as option (option)}
											<label class="choice">
												<input
													type="radio"
													name={field.key}
													value={option}
													bind:group={values[field.key]}
												/>
												{option}
											</label>
										{/each}
									</div>
								{:else if field.type === 'checkbox'}
									<label class="choice">
										<input type="checkbox" bind:checked={values[field.key]} />
										{field.label}
									</label>
									{#if field.help}<p class="help">{field.help}</p>{/if}
								{:else}
									<label for={field.key}>
										{field.label}{#if field.required} *{/if}
									</label>
									{#if field.help}<p class="help">{field.help}</p>{/if}
									<input
										id={field.key}
										type={field.type === 'number' ? 'number' : field.type === 'email' ? 'email' : 'text'}
										inputmode={field.type === 'number' ? 'numeric' : undefined}
										maxlength={field.max_len}
										bind:value={values[field.key]}
									/>
								{/if}
								{#if showErrors}
									<ul class="errors">
										{#each errorsForKey(errors, field.key) as error (error.code)}
											<li>{error.message}</li>
										{/each}
									</ul>
								{/if}
							</div>
						{/if}
					{/each}
				</section>
			{/each}

			<section>
				<h2>Signature</h2>
				<p class="help">
					The paper form is signed in ink after printing. This draft keeps no
					signature — sign the printed page.
				</p>
				<div class="sign-lines">
					<div><span>Signature</span><span class="line"></span></div>
					<div><span>Date</span><span class="line short"></span></div>
				</div>
			</section>

			<div class="actions no-print">
				<button type="submit">Review draft</button>
			</div>
		</form>
	{:else}
		{#each SECTIONS as section (section.title)}
			<section>
				<h2>{section.title}</h2>
				<dl class="review">
					{#each section.keys as key (key)}
						{@const field = byKey.get(key)}
						{#if field}
							<div class="row">
								<dt>{field.label}</dt>
								<dd>{displayValue(field, values[field.key])}</dd>
							</div>
						{/if}
					{/each}
				</dl>
			</section>
		{/each}

		<section>
			<h2>Signature</h2>
			<div class="sign-lines">
				<div><span>Signature</span><span class="line"></span></div>
				<div><span>Date</span><span class="line short"></span></div>
			</div>
		</section>

		<div class="actions no-print">
			<button type="button" onclick={() => (step = 'fill')}>Back to edit</button>
			<button type="button" onclick={printPage}>Print / save PDF</button>
		</div>
	{/if}
</article>

<style>
	.sheet {
		max-width: 8.5in;
		margin: 0 auto;
		padding: 1.5rem;
		background: var(--surface);
		border: 1px solid var(--border);
	}
	.masthead {
		border-bottom: 3px double var(--border);
		margin-bottom: 1rem;
		padding-bottom: 0.75rem;
	}
	.kicker {
		margin: 0 0 0.25rem;
		font-size: 0.8125rem;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}
	h1 {
		margin: 0 0 0.25rem;
		font-size: 1.5rem;
		line-height: 1.2;
	}
	.edition {
		margin: 0;
		font-size: 0.875rem;
	}
	section {
		margin-top: 1.5rem;
		padding-top: 1rem;
		border-top: 1px solid var(--border);
	}
	h2 {
		margin: 0 0 1rem;
		font-size: 1.0625rem;
	}
	.field {
		margin-bottom: 1rem;
	}
	.label,
	label {
		display: block;
		font-weight: 600;
		margin-bottom: 0.25rem;
	}
	.help {
		margin: 0 0 0.5rem;
		font-size: 0.875rem;
	}
	input[type='text'],
	input[type='email'],
	input[type='number'],
	textarea {
		width: 100%;
		box-sizing: border-box;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.5rem 0.75rem;
		background: var(--surface);
		color: inherit;
		font: inherit;
	}
	.choices {
		display: flex;
		gap: 1rem;
	}
	.choice {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		font-weight: 400;
	}
	.errors {
		margin: 0.375rem 0 0;
		padding-left: 1.25rem;
		color: var(--error);
		font-size: 0.875rem;
	}
	.review {
		margin: 0;
	}
	.review .row {
		display: grid;
		grid-template-columns: minmax(10rem, 35%) 1fr;
		gap: 0.75rem;
		padding: 0.375rem 0;
		border-bottom: 1px dotted var(--border);
	}
	.review dt {
		font-weight: 600;
	}
	.review dd {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.sign-lines {
		display: grid;
		grid-template-columns: 2fr 1fr;
		gap: 2rem;
		margin-top: 2rem;
	}
	.sign-lines > div {
		display: flex;
		flex-direction: column;
		gap: 1.75rem;
		font-size: 0.875rem;
	}
	.sign-lines .line {
		border-bottom: 1px solid currentColor;
	}
	.actions {
		display: flex;
		gap: 0.75rem;
		margin-top: 1.5rem;
	}
	.actions button {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.625rem 1.25rem;
		background: var(--accent);
		color: var(--accent-ink);
		font: inherit;
		font-weight: 600;
		cursor: pointer;
	}
	@media print {
		.no-print {
			display: none;
		}
		.sheet {
			max-width: none;
			border: 0;
			padding: 0;
		}
		@page {
			size: Letter;
			margin: 0.6in;
		}
	}
</style>
