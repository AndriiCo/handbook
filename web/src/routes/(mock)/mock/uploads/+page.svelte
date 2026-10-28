<script lang="ts">
	import { resolve } from '$app/paths';
	import Notice from '../../../../lib/public-ui/Notice.svelte';
	import { UPLOADS } from '../../../../lib/mock/fixtures';

	// Simulated picker: choosing real files lists them locally (names +
	// sizes only) and marks them ready without any network call.
	let picked = $state<{ name: string; size: string; note: string }[]>([]);
	let confirmed = $state(false);

	// Full class strings per state: exactly one color set wins.
	const pillCls = (state: string) =>
		`rounded-xs border px-2 py-[0.1rem] text-[0.8rem] font-bold ${
			state === 'ready'
				? 'border-success bg-success-bg text-success'
				: state === 'flagged'
					? 'border-error bg-error-bg text-error'
					: 'border-line'
		}`;

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
<p class="max-w-[40rem] text-ink-muted">
	Document photos and scans for your orders. Originals stay private to your account and staff —
	community-visible copies are masked first.
</p>

<form
	class="flex max-w-[32rem] flex-col gap-3"
	onsubmit={(e) => {
		e.preventDefault();
		confirmed = picked.length > 0;
	}}
>
	<label class="font-bold" for="files">Choose files (up to 10, 25 MB each)</label>
	<input
		class="min-h-11 py-[0.6rem] [font:inherit]"
		id="files"
		name="files"
		type="file"
		multiple
		onchange={onChange}
	/>
	{#if picked.length > 0}
		<ul class="m-0 list-none rounded-sm border border-line p-0">
			{#each picked as file (file.name)}
				<li class="flex justify-between gap-4 border-b border-line px-[0.8rem] py-2 last:border-b-0">
					<span>{file.name}</span>
					<span class="font-mono text-sm text-ink-muted">{file.size}</span>
				</li>
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
	<button
		class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink [font:inherit] disabled:cursor-not-allowed disabled:opacity-55"
		type="submit"
		disabled={picked.length === 0}>Upload (mock)</button
	>
</form>

<h2 class="mt-8 text-[0.8rem] tracking-[0.08em] text-ink-muted uppercase">
	Already on this mock account
</h2>
<ul class="m-0 mb-8 list-none p-0">
	{#each UPLOADS as upload (upload.name)}
		<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b border-line py-[0.6rem]">
			<span class="font-bold">{upload.name}</span>
			<span class="font-mono text-sm text-ink-muted">{upload.size}</span>
			<span class={pillCls(upload.state)} data-state={upload.state}>{upload.state}</span>
			<span class="basis-full text-sm text-ink-muted">{upload.note}</span>
		</li>
	{/each}
</ul>

