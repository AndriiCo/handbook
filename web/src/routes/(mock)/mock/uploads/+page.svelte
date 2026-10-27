<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { UPLOADS } from '../../../../lib/mock/fixtures';

	// Simulated picker: choosing real files lists them locally (names +
	// sizes only) and marks them ready without any network call.
	let picked = $state<{ name: string; size: string; note: string }[]>([]);
	let confirmed = $state(false);

	function onChange(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		picked = Array.from(input.files ?? []).map((f) => ({
			name: f.name,
			size: f.size > 1024 * 1024 ? `${(f.size / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(f.size / 1024))} KB`,
			note: 'Staged locally — mock upload only.'
		}));
		confirmed = false;
	}
</script>

<svelte:head>
	<title>Mock uploads — Apostille Handbook</title>
</svelte:head>

<h1>Uploads</h1>
<p class="lede">
	Document photos and scans for your orders. Originals stay private to your account and staff —
	community-visible copies are masked first.
</p>

<form
	onsubmit={(e) => {
		e.preventDefault();
		confirmed = picked.length > 0;
	}}
>
	<label for="files">Choose files (up to 10, 25 MB each)</label>
	<input id="files" name="files" type="file" multiple onchange={onChange} />
	{#if picked.length > 0}
		<ul class="picked">
			{#each picked as file (file.name)}
				<li><span>{file.name}</span> <span class="mono">{file.size}</span></li>
			{/each}
		</ul>
	{/if}
	{#if confirmed}
		<Notice variant="success" heading="Staged (mock).">
			<p>
				{picked.length} file{picked.length === 1 ? '' : 's'} staged locally. Nothing was sent.
				<a href={resolve('/(mock)/mock/precheck')}>See the precheck view</a>.
			</p>
		</Notice>
	{/if}
	<button type="submit" disabled={picked.length === 0}>Upload (mock)</button>
</form>

<h2 class="section-label">Already on this mock account</h2>
<ul class="stored">
	{#each UPLOADS as upload (upload.name)}
		<li>
			<span class="name">{upload.name}</span>
			<span class="mono">{upload.size}</span>
			<span class="pill" data-state={upload.state}>{upload.state}</span>
			<span class="note">{upload.note}</span>
		</li>
	{/each}
</ul>

<style>
	.lede {
		color: var(--text-muted);
		max-width: 40rem;
	}
	form {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		max-width: 32rem;
	}
	label {
		font-weight: 700;
	}
	input[type='file'] {
		min-height: 44px;
		padding: 0.6rem 0;
		font: inherit;
	}
	.picked {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
	}
	.picked li {
		display: flex;
		justify-content: space-between;
		gap: 1rem;
		padding: 0.5rem 0.8rem;
		border-bottom: 1px solid var(--border);
	}
	.picked li:last-child {
		border-bottom: none;
	}
	.mono {
		font-family: var(--font-mono);
		font-size: 0.875rem;
		color: var(--text-muted);
	}
	.section-label {
		font-size: 0.8rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: var(--text-muted);
		margin-top: 2rem;
	}
	.stored {
		list-style: none;
		margin: 0 0 2rem;
		padding: 0;
	}
	.stored li {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 0.25rem 0.75rem;
		padding: 0.6rem 0;
		border-bottom: 1px solid var(--border);
	}
	.name {
		font-weight: 700;
	}
	.pill {
		font-size: 0.8rem;
		font-weight: 700;
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
		padding: 0.1rem 0.5rem;
	}
	.pill[data-state='ready'] {
		background: var(--success-bg);
		color: var(--success);
		border-color: var(--success);
	}
	.pill[data-state='flagged'] {
		background: var(--error-bg);
		color: var(--error);
		border-color: var(--error);
	}
	.note {
		flex-basis: 100%;
		font-size: 0.875rem;
		color: var(--text-muted);
	}
	button {
		min-height: 44px;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
	}
	:root[data-theme='dark'] button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	button:disabled {
		opacity: 0.55;
		cursor: not-allowed;
	}
</style>
