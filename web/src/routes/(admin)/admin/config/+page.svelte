<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { fetchConfig, saveConfigKey } from '../../../../lib/admin/config-client.js';
	import {
		fetchSetupStatus,
		fetchSetupSteps,
		saveSetupKey,
		setupKeyHeaders
	} from '../../../../lib/admin/setup-client.js';
	import { STEPS, REQUIRED_KEYS } from '../../../../lib/admin/setup-guides.js';

	// Typed config keys per secure-config.md §1.5. SECRET rows are
	// write-only: the page shows Set/Unset and a new-value box only —
	// values are never fetched, rendered, or kept after save.
	// The admin API keeps the /apostille prefix on the admin origin
	// (spec §1.5), so the root is literal — the base-deriving trick
	// other pages use would resolve to '' on the admin host.
	//
	// Dual-mode page (secure-config.md §9.4): while the server
	// reports setup_required, this is the first-boot wizard (setup
	// key only, no session); otherwise the normal staff key list.
	const api = () => `/apostille/api/v1`;

	interface Row {
		key: string;
		type: 'bool' | 'text' | 'secret';
		value?: boolean | string;
		set?: boolean;
		draft?: string;
		dirty: boolean;
		saving: boolean;
		error: string | null;
		saved: boolean;
		// False for wizard fields the server list omits (never set):
		// completion counts only server-present rows.
		present?: boolean;
	}

	interface GuideField {
		key: string;
		type: 'bool' | 'text' | 'secret';
		label: string;
		hint: string;
		placeholder?: string;
	}

	interface GuideStep {
		id: string;
		title: string;
		blurb: string;
		keys: GuideField[];
		guide: { intro: string; steps: string[] };
	}

	const steps = STEPS as GuideStep[];
	const requiredKeys = REQUIRED_KEYS as string[];

	let mode = $state<'checking' | 'setup' | 'normal'>('checking');
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let rows = $state<Row[]>([]);

	let setupKey = $state('');
	let keyInput = $state('');
	let keyError = $state<string | null>(null);
	let setupRows = $state<Row[]>([]);
	let setupState = $state<'idle' | 'loading' | 'ready' | 'failed'>('idle');
	let setupError = $state<string | null>(null);
	let stepIdx = $state(0);

	async function load() {
		loadState = 'loading';
		loadError = null;
		try {
			const list = await fetchConfig(api());
			rows = list.map((row) => ({ ...row, saved: false }));
			loadState = 'ready';
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Config load failed.';
			loadState = 'failed';
		}
	}

	async function loadSteps() {
		setupState = 'loading';
		setupError = null;
		try {
			const list = await fetchSetupSteps(api(), setupKey);
			// Left-join every wizard field with the server list so
			// ALL rows live in $state: previously unset keys got
			// fresh plain objects per render, whose edits Svelte
			// never tracked — their Save buttons stayed disabled
			// forever. In-flight edits survive the refetch; the
			// "Saved." flag carries across it.
			const prev = new Map(setupRows.map((row) => [row.key, row]));
			const server = new Map(list.map((row) => [row.key, row]));
			setupRows = steps.flatMap((step) => step.keys).map((field) => {
				const old = prev.get(field.key);
				if (old && (old.dirty || old.saving)) return old;
				const row = server.get(field.key);
				const fresh: Row = row
					? { ...row, saved: false, present: true }
					: { ...synthRow(field), present: false };
				fresh.saved = old?.saved ?? false;
				return fresh;
			});
			setupState = 'ready';
		} catch (err) {
			setupError = err instanceof Error ? err.message : 'Setup steps failed.';
			setupState = 'failed';
		}
	}

	function startSetup(event: SubmitEvent) {
		event.preventDefault();
		keyError = null;
		if (!setupKeyHeaders(keyInput.trim())) {
			keyError =
				'That key is malformed — `setup_` + 64 hex chars, as printed by `xtask setup-key-mint`.';
			return;
		}
		setupKey = keyInput.trim();
		keyInput = '';
		void loadSteps();
	}

	function forgetKey() {
		setupKey = '';
		setupRows = [];
		setupState = 'idle';
		setupError = null;
		stepIdx = 0;
	}

	onMount(async () => {
		try {
			const status = await fetchSetupStatus(api());
			if (status.setup_required) {
				mode = 'setup';
				return;
			}
		} catch {
			// Status unreachable: fall through to the normal list,
			// which surfaces the real error.
		}
		mode = 'normal';
		await load();
	});

	function markDirty(row: Row) {
		row.dirty = true;
		row.saved = false;
		row.error = null;
	}

	async function save(row: Row) {
		row.saving = true;
		row.error = null;
		try {
			await saveConfigKey(api(), row);
			row.dirty = false;
			row.saved = true;
			if (row.type === 'secret') {
				row.set = true;
				row.draft = '';
			}
		} catch (err) {
			row.error = err instanceof Error ? err.message : 'Save failed.';
		} finally {
			row.saving = false;
		}
	}

	async function saveSetup(row: Row): Promise<boolean> {
		row.saving = true;
		row.error = null;
		try {
			await saveSetupKey(api(), setupKey, row);
			row.dirty = false;
			row.saved = true;
			if (row.type === 'secret') {
				row.set = true;
				row.draft = '';
			}
			await loadSteps();
			return true;
		} catch (err) {
			const message = err instanceof Error ? err.message : 'Save failed.';
			if (/window already closed/.test(message)) {
				setupError = message;
				setupState = 'failed';
				return false;
			}
			row.error = message;
			return false;
		} finally {
			row.saving = false;
		}
	}

	// Step change with auto-save: dirty fields on the current step
	// save first (sequentially, showing row errors); a failure
	// keeps the operator on the step instead of losing edits.
	let navSaving = $state(false);

	async function goStep(i: number) {
		if (i === stepIdx || navSaving) return;
		const dirty = steps[stepIdx].keys.map(setupRowFor).filter((row) => row.dirty);
		if (dirty.length > 0) {
			navSaving = true;
			try {
				for (const row of dirty) {
					if (!(await saveSetup(row))) return;
				}
			} finally {
				navSaving = false;
			}
		}
		stepIdx = i;
	}

	function synthRow(field: GuideField): Row {
		if (field.type === 'bool') {
			return { key: field.key, type: 'bool', value: false, dirty: false, saving: false, error: null, saved: false };
		}
		if (field.type === 'secret') {
			return { key: field.key, type: 'secret', set: false, draft: '', dirty: false, saving: false, error: null, saved: false };
		}
		return { key: field.key, type: 'text', value: '', dirty: false, saving: false, error: null, saved: false };
	}

	function setupRowFor(field: GuideField): Row {
		// loadSteps left-joins every field, so the find always hits;
		// the synth fallback only guards a future divergence.
		return setupRows.find((row) => row.key === field.key) ?? synthRow(field);
	}

	function textValue(row: Row): string {
		return typeof row.value === 'string' ? row.value : '';
	}

	function requiredMet(key: string): boolean {
		const row = setupRows.find((r) => r.key === key);
		if (!row) return false;
		if (row.type === 'bool') return row.value === true;
		if (row.type === 'secret') return row.set === true;
		return textValue(row).trim().length > 0;
	}

	const missingRequired = $derived(requiredKeys.filter((key) => !requiredMet(key)));

	function stepDoneCount(step: GuideStep): number {
		return step.keys.filter((field) => {
			const row = setupRows.find((r) => r.key === field.key);
			if (!row) return false;
			if (row.type === 'bool') return row.present === true;
			if (row.type === 'secret') return row.set === true;
			return textValue(row).trim().length > 0;
		}).length;
	}

	function stepFor(key: string): number {
		const idx = steps.findIndex((step) => step.keys.some((field) => field.key === key));
		return idx < 0 ? 0 : idx;
	}

	// Full class strings per state (not stacked class: directives):
	// the active fill must never co-occur with the idle border in
	// the cascade — and the tailwind checker only reads literals.
	function stepBtnCls(active: boolean): string {
		const base = 'min-h-11 cursor-pointer rounded-sm border px-3 font-bold [font:inherit] disabled:cursor-default';
		return active ? `${base} border-accent-fill bg-accent-fill text-accent-ink` : `${base} border-line bg-bg`;
	}
</script>

<svelte:head>
	<title>{mode === 'setup' ? 'First-time setup' : 'Configuration'} — Admin — Apostille Handbook</title>
</svelte:head>

{#if mode === 'checking'}
	<h1>Configuration</h1>
	<p aria-live="polite">Loading…</p>
{:else if mode === 'setup'}
	<h1>First-time setup</h1>

	<Notice variant="warning" heading="One key opens this wizard.">
		<p>
			Paste the setup key printed once by <code>xtask setup-key-mint</code> on the
			server. It works from any browser until the first staff passkey enrolment
			burns it — then this wizard closes permanently and the normal config list
			takes over. The key is never stored in the browser.
		</p>
	</Notice>

	{#if !setupKey}
		<form
			class="mt-4 grid max-w-[28rem] gap-3"
			onsubmit={(e) => {
				e.preventDefault();
				startSetup(e);
			}}
		>
			<label class="flex min-h-11 flex-col gap-[0.35rem] font-bold" for="setup-key">
				Setup key
				<input
					class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-mono font-normal text-ink [font:inherit]"
					id="setup-key"
					type="password"
					bind:value={keyInput}
					placeholder="setup_…"
					autocomplete="off"
					spellcheck={false}
				/>
			</label>
			{#if keyError}
				<Notice variant="error" heading="Bad key."><p>{keyError}</p></Notice>
			{/if}
			<p>
				<button
					class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
					type="submit">Start / resume setup</button
				>
			</p>
		</form>
	{:else if setupState === 'loading' || setupState === 'idle'}
		<p aria-live="polite">Loading setup progress…</p>
	{:else if setupState === 'failed'}
		<Notice variant="error" heading="Setup unavailable."><p>{setupError}</p></Notice>
		<p class="flex gap-2">
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
				type="button"
				onclick={loadSteps}>Retry</button
			>
			<button
				class="min-h-11 cursor-pointer rounded-sm border border-line bg-bg px-5 font-bold [font:inherit]"
				type="button"
				onclick={forgetKey}>Use a different key</button
			>
		</p>
	{:else}
		<nav aria-label="Setup steps">
			<ol class="m-0 flex list-none flex-wrap gap-2 p-0">
				{#each steps as step, i (step.id)}
					<li>
						<button
							class={stepBtnCls(i === stepIdx)}
							type="button"
							disabled={i === stepIdx || navSaving}
							onclick={() => goStep(i)}
							aria-current={i === stepIdx ? 'step' : undefined}
						>
							{i + 1}. {step.title} ({stepDoneCount(step)}/{step.keys.length})
						</button>
					</li>
				{/each}
			</ol>
		</nav>

		{@const step = steps[stepIdx]}
		<section aria-labelledby="setup-step">
			<h2 id="setup-step">{stepIdx + 1}. {step.title}</h2>
			<p class="max-w-[44rem]">{step.blurb}</p>

			<div class="my-3 max-w-[44rem] rounded-sm border border-line bg-surface-2 p-4" role="note" aria-label="Retrieval guide">
				<p class="m-0 mb-1 text-sm font-bold">How to get these values</p>
				<p class="m-0 mb-1 text-sm">{step.guide.intro}</p>
				<ol class="m-0 text-sm">
					{#each step.guide.steps as text (text)}
						<li class="mb-1">{text}</li>
					{/each}
				</ol>
			</div>

			<ul class="m-0 mt-4 grid max-w-[44rem] list-none gap-4 p-0 pb-8">
				{#each step.keys as field (field.key)}
					{@const row = setupRowFor(field)}
					<li class="rounded-sm border border-line px-4 py-[0.9rem]">
						<div class="mb-2 flex flex-wrap items-baseline gap-x-3 gap-y-1">
							<code class="font-mono font-semibold break-all">{field.label}</code>
							<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">{field.type}</span>
							{#if requiredKeys.includes(field.key)}
								<span class="text-[0.8rem] font-bold tracking-[0.08em] text-error uppercase">required</span>
							{/if}
						</div>
						<p class="m-0 mb-2 font-mono text-sm break-all">{field.key}</p>
						<p class="m-0 mb-2 text-sm text-ink-muted">{field.hint}</p>
						{#if field.type === 'bool'}
							<label class="min-h-11">
								<input
									class="mr-2 size-5 accent-accent align-middle"
									type="checkbox"
									checked={row.value === true}
									onchange={(e) => {
										row.value = e.currentTarget.checked;
										markDirty(row);
									}}
								/>
								Enabled
							</label>
						{:else if field.type === 'text'}
							<label class="flex min-h-11 flex-col gap-[0.35rem] font-bold" for={`setup-${field.key}`}>
								Value
								<input
									class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink [font:inherit]"
									id={`setup-${field.key}`}
									type="text"
									value={textValue(row)}
									placeholder={field.placeholder ?? ''}
									maxlength={4096}
									oninput={(e) => {
										row.value = e.currentTarget.value;
										markDirty(row);
										}}
								/>
							</label>
						{:else}
							<p class="mt-1 mb-2 font-bold" aria-live="polite">
								{row.set ? 'Set (value hidden)' : 'Not set'}
							</p>
							<label class="flex min-h-11 flex-col gap-[0.35rem] font-bold" for={`setup-${field.key}`}>
								New value (write-only — overwrites)
								<input
									class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink [font:inherit]"
									id={`setup-${field.key}`}
									type="password"
									autocomplete="new-password"
									maxlength={8192}
										value={row.draft ?? ''}
										oninput={(e) => {
										row.draft = e.currentTarget.value;
										markDirty(row);
										}}
								/>
							</label>
						{/if}
						{#if row.error}
							<Notice variant="error" heading="Save failed."><p>{row.error}</p></Notice>
						{/if}
						{#if row.saved}
							<Notice variant="success" heading="Saved.">
								<p>“{row.key}” updated and audited.</p>
							</Notice>
						{/if}
						<p>
							<button
								class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
								onclick={() => saveSetup(row)}
								disabled={!row.dirty || row.saving || (row.type === 'secret' && !row.draft)}
							>
								{row.saving ? 'Saving…' : 'Save'}
							</button>
						</p>
					</li>
				{/each}
			</ul>

			<div class="flex gap-2 pb-8">
				{#if stepIdx > 0}
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-line bg-bg px-5 font-bold [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
						type="button"
						disabled={navSaving}
						onclick={() => goStep(stepIdx - 1)}>← {steps[stepIdx - 1].title}</button
					>
				{/if}
				{#if stepIdx < steps.length - 1}
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
						type="button"
						disabled={navSaving}
						onclick={() => goStep(stepIdx + 1)}>{navSaving ? 'Saving…' : `${steps[stepIdx + 1].title} →`}</button
					>
				{/if}
			</div>
		</section>

		<section aria-labelledby="setup-review">
			<h2 id="setup-review">Review</h2>
			<p>
				{requiredKeys.length - missingRequired.length} of {requiredKeys.length} required keys set.
			</p>
			{#if missingRequired.length === 0}
				<Notice variant="success" heading="Setup complete.">
					<p>
						Every required key is set. Enrol the first staff passkey to burn the setup
						key and close this wizard — the normal config list takes over after.
					</p>
				</Notice>
			{:else}
				<p>Still missing:</p>
				<ul class="m-0 list-none p-0 pb-4">
					{#each missingRequired as key (key)}
						<li>
							<button
								class="min-h-11 cursor-pointer font-mono [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
								type="button"
								disabled={navSaving}
								onclick={() => goStep(stepFor(key))}
							>
								{key}
							</button>
						</li>
					{/each}
				</ul>
			{/if}
			<p class="flex flex-wrap gap-2 pb-8">
				<a
					class="inline-flex min-h-11 items-center rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink"
					href={`${resolve('/(admin)/admin/enrol')}?setup=1`}
					>Enrol the first passkey (burns the setup key)</a
				>
			</p>
		</section>
	{/if}
{:else}


<h1>Configuration</h1>
<p class="max-w-[44rem] text-ink-muted">
	Encrypted server config. Secrets are set-new-only: this page can never
	display one. Every save writes an audit row.
</p>

{#if loadState === 'loading'}
	<p aria-live="polite">Loading config keys…</p>
{:else if loadState === 'failed'}
	<Notice variant="error" heading="Config unavailable.">
		<p>{loadError}</p>
	</Notice>
	<p>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
			onclick={load}>Retry</button
		>
	</p>
{:else if rows.length === 0}
	<Notice variant="info" heading="No config keys.">
		<p>The server returned an empty key list. Seed keys first, then reload.</p>
	</Notice>
	<p>
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
			onclick={load}>Reload</button
		>
	</p>
{:else}
	<ul class="mt-4 mb-0 grid max-w-[44rem] list-none gap-4 p-0 pb-8">
		{#each rows as row (row.key)}
			<li class="rounded-sm border border-line px-4 py-[0.9rem]">
				<div class="mb-2 flex flex-wrap items-baseline gap-x-3 gap-y-1">
					<code class="font-mono font-semibold break-all">{row.key}</code>
					<span class="text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">{row.type}</span>
				</div>
				{#if row.type === 'bool'}
					<label class="min-h-11">
						<input
							class="mr-2 size-5 accent-accent align-middle"
							type="checkbox"
							checked={row.value === true}
							onchange={(e) => {
								row.value = e.currentTarget.checked;
								markDirty(row);
							}}
						/>
						Enabled
					</label>
				{:else if row.type === 'text'}
					<label class="flex min-h-11 flex-col gap-[0.35rem] font-bold" for={`cfg-${row.key}`}>
						Value
						<input
							class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink [font:inherit]"
							id={`cfg-${row.key}`}
							type="text"
							value={typeof row.value === 'string' ? row.value : ''}
							maxlength={4096}
							oninput={(e) => {
								row.value = e.currentTarget.value;
								markDirty(row);
							}}
						/>
					</label>
				{:else}
					<p class="mt-1 mb-2 font-bold" aria-live="polite">
						{row.set ? 'Set (value hidden)' : 'Not set'}
					</p>
					<label class="flex min-h-11 flex-col gap-[0.35rem] font-bold" for={`cfg-${row.key}`}>
						New value (write-only — overwrites)
						<input
							class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] font-normal text-ink [font:inherit]"
							id={`cfg-${row.key}`}
							type="password"
							autocomplete="new-password"
							maxlength={8192}
							value={row.draft ?? ''}
							oninput={(e) => {
								row.draft = e.currentTarget.value;
								markDirty(row);
							}}
						/>
					</label>
				{/if}
				{#if row.error}
					<Notice variant="error" heading="Save failed."><p>{row.error}</p></Notice>
				{/if}
				{#if row.saved}
					<Notice variant="success" heading="Saved.">
						<p>“{row.key}” updated and audited.</p>
					</Notice>
				{/if}
				<p>
					<button
						class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill px-5 font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
						onclick={() => save(row)}
						disabled={!row.dirty || row.saving || (row.type === 'secret' && !row.draft)}
					>
						{row.saving ? 'Saving…' : 'Save'}
					</button>
				</p>
			</li>
		{/each}
	</ul>
{/if}
{/if}
