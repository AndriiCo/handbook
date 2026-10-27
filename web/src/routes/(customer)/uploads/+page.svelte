<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { formatBytes, validateFiles } from '../../../lib/customer/uploads.js';

	// Upload shell: validates locally, then POSTs to /api/v1/uploads.
	// The endpoint does not exist yet — failures stay honest and the
	// file picker is preserved.
	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	let files = $state<File[]>([]);
	let errors = $state<string[]>([]);
	let result = $state<string | null>(null);
	let busy = $state(false);

	function onChange(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		files = Array.from(input.files ?? []);
		errors = [];
		result = null;
	}

	async function upload(event: SubmitEvent) {
		event.preventDefault();
		const check = validateFiles(files);
		errors = check.errors;
		result = null;
		if (!check.ok) return;
		busy = true;
		try {
			const body = new FormData();
			for (const file of files) body.append('files', file, file.name);
			const res = await fetch(api('/uploads'), {
				method: 'POST',
				credentials: 'same-origin',
				body
			});
			if (res.status === 404) {
				errors = ['Upload endpoint not found (404) — the server lane has not landed yet.'];
			} else if (!res.ok) {
				errors = [`Upload failed: HTTP ${res.status}. Nothing was stored.`];
			} else {
				result = `${files.length} file${files.length === 1 ? '' : 's'} uploaded.`;
				files = [];
			}
		} catch {
			errors = ['Upload service unreachable — is the API server running? Nothing was sent.'];
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>Uploads — Apostille Handbook</title>
</svelte:head>

<h1>Uploads</h1>
<p class="lede">
	Document photos and scans for your orders. Originals stay private to
	your account and staff — community-visible copies are masked first.
</p>

<form onsubmit={upload}>
	<label for="files">Choose files (up to 10, 25 MB each)</label>
	<input id="files" name="files" type="file" multiple onchange={onChange} />
	{#if files.length > 0}
		<ul class="picked">
			{#each files as file (`${file.name}-${file.size}-${file.lastModified}`)}
				<li><span>{file.name}</span> <span class="mono">{formatBytes(file.size)}</span></li>
			{/each}
		</ul>
	{/if}
	{#each errors as error (error)}
		<Notice variant="error" heading="Cannot upload."><p>{error}</p></Notice>
	{/each}
	{#if result}
		<Notice variant="success" heading="Uploaded."><p>{result}</p></Notice>
	{/if}
	<button type="submit" disabled={busy || files.length === 0}>
		{busy ? 'Uploading…' : 'Upload'}
	</button>
</form>

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
