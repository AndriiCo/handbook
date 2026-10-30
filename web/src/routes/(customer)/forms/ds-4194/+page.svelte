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

<article class="mx-auto max-w-[8.5in] border border-line bg-surface p-6 print:max-w-none print:border-0 print:p-0">
	<header class="mb-4 pb-3 [border-bottom:3px_double_var(--border)]">
		<p class="m-0 mb-1 text-[0.8125rem] tracking-[0.04em] uppercase">
			US Department of State · draft resemblance · not the official form
		</p>
		<h1 class="m-0 mb-1 font-sans text-2xl leading-[1.2]">{template.title} (DS-4194)</h1>
		<p class="m-0 text-sm">{template.authority_revision} · template v{template.version}</p>
	</header>

	<div class="print:hidden">
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
				<section class="mt-6 border-t border-line pt-4">
					<h2 class="m-0 mb-4 text-[1.0625rem]">{section.title}</h2>
					{#each section.keys as key (key)}
						{@const field = byKey.get(key)}
						{#if field}
							<div class="mb-4">
								{#if field.type === 'address_block'}
									<span class="mb-1 block font-semibold" id={`label-${field.key}`}>
										{field.label}{#if field.required} *{/if}
									</span>
									{#if field.help}<p class="m-0 mb-2 text-sm">{field.help}</p>{/if}
									<AddressBlock
										idPrefix={field.key}
										bind:value={values[field.key]}
										{showErrors}
									/>
								{:else if field.type === 'tel'}
									<label class="mb-1 block font-semibold" for={field.key}>
										{field.label}{#if field.required} *{/if}
									</label>
									{#if field.help}<p class="m-0 mb-2 text-sm">{field.help}</p>{/if}
									<PhoneInput id={field.key} bind:value={values[field.key]} />
								{:else if field.key === 'countries_of_use'}
									<label class="mb-1 block font-semibold" for={field.key}>
										{field.label}{#if field.required} *{/if}
									</label>
									{#if field.help}<p class="m-0 mb-2 text-sm">{field.help}</p>{/if}
									<textarea
										class="w-full rounded-sm border border-line bg-surface px-3 py-2 text-inherit"
										id={field.key}
										rows="2"
										maxlength={field.max_len}
										bind:value={values[field.key]}
										placeholder="List each country; one per line"
									></textarea>
									<p class="m-0 mb-2 text-sm">Country picker for single-country use:</p>
									<CountrySelect bind:value={values[field.key]} />
								{:else if field.type === 'textarea'}
									<label class="mb-1 block font-semibold" for={field.key}>
										{field.label}{#if field.required} *{/if}
									</label>
									{#if field.help}<p class="m-0 mb-2 text-sm">{field.help}</p>{/if}
									<textarea
										class="w-full rounded-sm border border-line bg-surface px-3 py-2 text-inherit"
										id={field.key}
										rows="3"
										maxlength={field.max_len}
										bind:value={values[field.key]}
									></textarea>
								{:else if field.type === 'radio'}
									<span class="mb-1 block font-semibold" id={`label-${field.key}`}>
										{field.label}{#if field.required} *{/if}
									</span>
									{#if field.help}<p class="m-0 mb-2 text-sm">{field.help}</p>{/if}
									<div class="flex gap-4" role="radiogroup" aria-labelledby={`label-${field.key}`}>
										{#each (field.options ?? []) as option (option)}
											<label class="mb-1 flex items-center gap-2 font-normal">
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
									<label class="mb-1 flex items-center gap-2 font-normal">
										<input type="checkbox" bind:checked={values[field.key]} />
										{field.label}
									</label>
									{#if field.help}<p class="m-0 mb-2 text-sm">{field.help}</p>{/if}
								{:else}
									<label class="mb-1 block font-semibold" for={field.key}>
										{field.label}{#if field.required} *{/if}
									</label>
									{#if field.help}<p class="m-0 mb-2 text-sm">{field.help}</p>{/if}
									<input
										class="w-full rounded-sm border border-line bg-surface px-3 py-2 text-inherit"
										id={field.key}
										type={field.type === 'number' ? 'number' : field.type === 'email' ? 'email' : 'text'}
										inputmode={field.type === 'number' ? 'numeric' : undefined}
										maxlength={field.max_len}
										bind:value={values[field.key]}
									/>
								{/if}
								{#if showErrors}
									<ul class="mt-[0.375rem] mb-0 pl-5 text-sm text-error">
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

			<section class="mt-6 border-t border-line pt-4">
				<h2 class="m-0 mb-4 text-[1.0625rem]">Signature</h2>
				<p class="m-0 mb-2 text-sm">
					The paper form is signed in ink after printing. This draft keeps no
					signature — sign the printed page.
				</p>
				<div class="mt-8 grid grid-cols-[2fr_1fr] gap-8">
					<div class="flex flex-col gap-7 text-sm"><span>Signature</span><span class="border-b border-current"></span></div>
					<div class="flex flex-col gap-7 text-sm"><span>Date</span><span class="border-b border-current"></span></div>
				</div>
			</section>

			<div class="mt-6 flex gap-3 print:hidden">
				<button
					class="cursor-pointer rounded-sm border border-line bg-accent-fill px-5 py-[0.625rem] font-semibold text-accent-ink"
					type="submit">Review draft</button
				>
			</div>
		</form>
	{:else}
		{#each SECTIONS as section (section.title)}
			<section class="mt-6 border-t border-line pt-4">
				<h2 class="m-0 mb-4 text-[1.0625rem]">{section.title}</h2>
				<dl class="m-0">
					{#each section.keys as key (key)}
						{@const field = byKey.get(key)}
						{#if field}
							<div class="grid grid-cols-[minmax(10rem,35%)_1fr] gap-3 py-[0.375rem] [border-bottom:1px_dotted_var(--border)]">
								<dt class="font-semibold">{field.label}</dt>
								<dd class="m-0 wrap-anywhere">{displayValue(field, values[field.key])}</dd>
							</div>
						{/if}
					{/each}
				</dl>
			</section>
		{/each}

		<section class="mt-6 border-t border-line pt-4">
			<h2 class="m-0 mb-4 text-[1.0625rem]">Signature</h2>
			<div class="mt-8 grid grid-cols-[2fr_1fr] gap-8">
				<div class="flex flex-col gap-7 text-sm"><span>Signature</span><span class="border-b border-current"></span></div>
				<div class="flex flex-col gap-7 text-sm"><span>Date</span><span class="border-b border-current"></span></div>
			</div>
		</section>

		<div class="mt-6 flex gap-3 print:hidden">
			<button
				class="cursor-pointer rounded-sm border border-line bg-accent-fill px-5 py-[0.625rem] font-semibold text-accent-ink"
				type="button"
				onclick={() => (step = 'fill')}>Back to edit</button
			>
			<button
				class="cursor-pointer rounded-sm border border-line bg-accent-fill px-5 py-[0.625rem] font-semibold text-accent-ink"
				type="button"
				onclick={printPage}>Print / save PDF</button
			>
		</div>
	{/if}
</article>

<style>
	/* Tailwind has no @page support — the Letter page box stays. */
	@page {
		size: Letter;
		margin: 0.6in;
	}
</style>
