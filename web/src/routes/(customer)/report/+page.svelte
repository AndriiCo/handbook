<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import Heading from '../../../lib/components/brand/Heading.svelte';

	// Walk-in report form (SA-21): signed-in visitors report a change
	// at an issuing office — text plus up to 3 photos. Anonymous
	// visitors get the sign-in prompt, never the form. Photos are
	// pre-checked client-side by magic bytes; the server re-checks,
	// strips EXIF, and holds everything unmasked ("awaiting masking")
	// until the masking pass lands.

	type Picked = { name: string; dataBase64: string };

	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;
	const loginHref = resolve('/(customer)/login');

	const MAX_PHOTOS = 3;
	const MAX_PHOTO_BYTES = 8 * 1024 * 1024;
	const MAX_BODY_CHARS = 2000;

	const MAGIC: Array<{ kind: string; test: (b: Uint8Array) => boolean }> = [
		{ kind: 'jpeg', test: (b) => b.length >= 3 && b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff },
		{
			kind: 'png',
			test: (b) =>
				b.length >= 8 &&
				b[0] === 0x89 &&
				b[1] === 0x50 &&
				b[2] === 0x4e &&
				b[3] === 0x47 &&
				b[4] === 0x0d &&
				b[5] === 0x0a &&
				b[6] === 0x1a &&
				b[7] === 0x0a
		},
		{
			kind: 'webp',
			test: (b) =>
				b.length >= 12 &&
				b[0] === 0x52 &&
				b[1] === 0x49 &&
				b[2] === 0x46 &&
				b[3] === 0x46 &&
				b[8] === 0x57 &&
				b[9] === 0x45 &&
				b[10] === 0x42 &&
				b[11] === 0x50
		},
		{
			kind: 'gif',
			test: (b) =>
				b.length >= 6 &&
				b[0] === 0x47 &&
				b[1] === 0x49 &&
				b[2] === 0x46 &&
				b[3] === 0x38 &&
				(b[4] === 0x37 || b[4] === 0x39) &&
				b[5] === 0x61
		}
	];

	let authority = $state(page.url.searchParams.get('destination') ?? '');
	let body = $state('');
	let picked = $state<Picked[]>([]);
	let notice = $state<{ variant: 'info' | 'error' | 'success'; heading: string; text: string } | null>(
		null
	);
	let needsLogin = $state(false);
	let submittedId = $state<number | null>(null);
	let busy = $state(false);

	const toBase64 = (bytes: Uint8Array): string => {
		let bin = '';
		for (let i = 0; i < bytes.length; i += 0x8000) {
			bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
		}
		return btoa(bin);
	};

	async function onFiles(files: FileList | null) {
		notice = null;
		if (!files) return;
		for (const file of Array.from(files)) {
			if (picked.length >= MAX_PHOTOS) {
				notice = { variant: 'error', heading: 'At most 3 photos.', text: 'Extra photos were skipped.' };
				break;
			}
			if (file.size > MAX_PHOTO_BYTES) {
				notice = {
					variant: 'error',
					heading: 'Photo too large.',
					text: `${file.name} is over 8 MiB and was skipped.`
				};
				continue;
			}
			const bytes = new Uint8Array(await file.arrayBuffer());
			if (!MAGIC.some((m) => m.test(bytes))) {
				notice = {
					variant: 'error',
					heading: 'Photo type not allowed.',
					text: `${file.name} is not a JPEG, PNG, WebP or GIF photo and was skipped.`
				};
				continue;
			}
			picked = [...picked, { name: file.name, dataBase64: toBase64(bytes) }];
		}
	}

	async function onSubmit(event: SubmitEvent) {
		event.preventDefault();
		notice = null;
		submittedId = null;
		if (!authority.trim()) {
			notice = { variant: 'error', heading: 'Pick an office.', text: 'Tell us which issuing office this is about.' };
			return;
		}
		if (!body.trim()) {
			notice = { variant: 'error', heading: 'Describe the change.', text: 'A short note is required.' };
			return;
		}
		busy = true;
		try {
			const res = await fetch(api('/reports'), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					authority: authority.trim(),
					body: body.trim(),
					photos: picked.map((p) => ({ filename: p.name, data_base64: p.dataBase64 }))
				})
			});
			if (res.status === 401) {
				needsLogin = true;
				return;
			}
			if (res.status === 429) {
				notice = {
					variant: 'error',
					heading: 'Slow down.',
					text: 'Too many reports just now — try again in a bit.'
				};
				return;
			}
			const data = await res.json().catch(() => ({}));
			if (!res.ok) {
				notice = {
					variant: 'error',
					heading: 'Report not saved.',
					text: typeof data?.error === 'string' ? data.error : 'Try again in a bit.'
				};
				return;
			}
			submittedId = typeof data?.id === 'number' ? data.id : null;
			notice = {
				variant: 'success',
				heading: 'Report received.',
				text: 'It stays private while we remove personal details — nothing you wrote shows publicly until then.'
			};
			body = '';
			picked = [];
		} catch {
			notice = {
				variant: 'error',
				heading: 'Report not saved.',
				text: 'The service is unreachable. Try again in a bit.'
			};
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head>
	<title>Report a change — Apostille Handbook</title>
</svelte:head>

<Heading level={1}>Report a change</Heading>
<p class="mt-2 max-w-[40rem] text-ink-muted">
	Spotted a changed fee, address, or step at an issuing office? File a walk-in report. Photos are
	checked on your device and stripped of camera metadata on our end; your note stays private while
	personal details are removed.
</p>

{#if needsLogin}
	<Notice variant="info" heading="Sign in to file a report.">
		<p>
			Walk-in reports live in your account.
			<a href={loginHref}>Sign in</a>, then come back here.
		</p>
	</Notice>
{:else}
	{#if notice}
		<Notice variant={notice.variant} heading={notice.heading}><p>{notice.text}</p></Notice>
	{/if}
	<form class="mt-6 flex max-w-[40rem] flex-col gap-4" onsubmit={onSubmit}>
		<label class="flex flex-col gap-1 text-sm font-medium">
			Issuing office (slug, e.g. usa-wa-sos)
			<input
				class="min-h-11 rounded-sm border border-line bg-bg px-3 font-normal"
				name="authority"
				bind:value={authority}
				autocomplete="off"
				maxlength="128"
				required
			/>
		</label>
		<label class="flex flex-col gap-1 text-sm font-medium">
			What changed? ({body.length}/{MAX_BODY_CHARS})
			<textarea
				class="min-h-28 rounded-sm border border-line bg-bg px-3 py-2 font-normal"
				name="body"
				bind:value={body}
				maxlength={MAX_BODY_CHARS}
				required
			></textarea>
		</label>
		<div class="flex flex-col gap-1 text-sm font-medium">
			Photos (up to {MAX_PHOTOS}; JPEG, PNG, WebP or GIF, 8 MiB each)
			<input
				class="font-normal"
				type="file"
				accept="image/jpeg,image/png,image/webp,image/gif"
				multiple
				onchange={(e) => onFiles(e.currentTarget.files)}
			/>
			{#if picked.length > 0}
				<ul class="mt-1 list-disc pl-5 font-normal text-ink-muted">
					{#each picked as p, i (p.name + i)}
						<li>
							{p.name}
							<button
								type="button"
								class="underline"
								onclick={() => (picked = picked.filter((_, j) => j !== i))}>remove</button
							>
						</li>
					{/each}
				</ul>
			{/if}
		</div>
		<button
			type="submit"
			disabled={busy}
			class="inline-flex min-h-11 items-center justify-center rounded-sm bg-accent-fill px-5 font-medium text-accent-ink transition-colors hover:bg-accent-hover disabled:opacity-60"
		>
			{busy ? 'Sending…' : 'Send report'}
		</button>
		{#if submittedId !== null}
			<p class="text-sm text-ink-muted">Saved as report #{submittedId}.</p>
		{/if}
	</form>
{/if}
