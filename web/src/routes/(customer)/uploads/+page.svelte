<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { formatBytes, validateFiles } from '../../../lib/customer/uploads.js';
	import { site } from '../../../lib/locale.svelte.js';
	import { UPLOADS_COPY } from '../../../lib/customer/uploads-copy.js';

	// Upload shell: validates locally, then POSTs to /api/v1/uploads.
	// The endpoint does not exist yet — failures stay honest and the
	// file picker is preserved.
	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;

	const c = $derived(UPLOADS_COPY[site.locale]);

	// What went wrong, kept as data so a language switch re-words it:
	// the local checks re-run on the same files in the new language.
	type Problem = { kind: 'invalid' } | { kind: 'not_found' } | { kind: 'http'; status: number } | { kind: 'unreachable' };

	let files = $state<File[]>([]);
	let problem = $state<Problem | null>(null);
	let uploaded = $state<number | null>(null);
	let busy = $state(false);

	const errors = $derived.by((): string[] => {
		if (!problem) return [];
		if (problem.kind === 'invalid') return validateFiles(files, site.locale).errors;
		if (problem.kind === 'not_found') return [c.notFound];
		if (problem.kind === 'http') return [c.failed(problem.status)];
		return [c.unreachable];
	});

	function onChange(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		files = Array.from(input.files ?? []);
		problem = null;
		uploaded = null;
	}

	async function upload(event: SubmitEvent) {
		event.preventDefault();
		const check = validateFiles(files);
		problem = check.ok ? null : { kind: 'invalid' };
		uploaded = null;
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
				problem = { kind: 'not_found' };
			} else if (!res.ok) {
				problem = { kind: 'http', status: res.status };
			} else {
				uploaded = files.length;
				files = [];
			}
		} catch {
			problem = { kind: 'unreachable' };
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>{c.title} — Apostille Handbook</title>
</svelte:head>

<h1>{c.title}</h1>
<p class="max-w-[40rem] text-ink-muted">{c.intro}</p>

<form class="flex max-w-[32rem] flex-col gap-3" onsubmit={upload}>
	<label class="font-bold" for="files">{c.choose}</label>
	<input
		class="min-h-11 py-[0.6rem]"
		id="files"
		name="files"
		type="file"
		multiple
		onchange={onChange}
	/>
	{#if files.length > 0}
		<ul class="m-0 list-none rounded-sm border border-line p-0">
			{#each files as file (`${file.name}-${file.size}-${file.lastModified}`)}
				<li class="flex justify-between gap-4 border-b border-line px-[0.8rem] py-2 last:border-b-0">
					<span>{file.name}</span> <span class="font-mono text-sm text-ink-muted">{formatBytes(file.size, site.locale)}</span>
				</li>
			{/each}
		</ul>
	{/if}
	{#each errors as error (error)}
		<Notice variant="error" heading={c.cannotUpload}><p>{error}</p></Notice>
	{/each}
	{#if uploaded !== null}
		<Notice variant="success" heading={c.uploadedHeading}><p>{c.uploaded(uploaded)}</p></Notice>
	{/if}
	<button
		class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink disabled:cursor-not-allowed disabled:opacity-[0.55]"
		type="submit"
		disabled={busy || files.length === 0}
	>
		{busy ? c.uploading : c.upload}
	</button>
</form>
