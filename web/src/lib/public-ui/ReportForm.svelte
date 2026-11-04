<script lang="ts">
	import { onMount } from 'svelte';
	import ImagePlus from '@lucide/svelte/icons/image-plus';
	import XIcon from '@lucide/svelte/icons/x';
	import LoaderCircle from '@lucide/svelte/icons/loader-circle';
	import CheckIcon from '@lucide/svelte/icons/check';
	import RotateCw from '@lucide/svelte/icons/rotate-cw';
	import TextQuote from '@lucide/svelte/icons/text-quote';
	import Notice from './Notice.svelte';
	import {
		AUTOSAVE_MS,
		clearDraft,
		draftKey,
		loadDraft,
		photoKind,
		quoteLine,
		saveDraft
	} from './report-draft.js';

	// Walk-in report form (SA-21), shared by the dossier's docked panel
	// and the bare /report page. Text plus up to 3 photos; each photo
	// uploads the moment it is picked (POST /reports/uploads, staged
	// under the visitor's account) and the submit names the staged ids.
	// The note, office and staged ids live in localStorage
	// (report-draft.js) until the report is sent: saved every minute
	// while something changed, and whenever the window loses focus, the
	// tab hides or the page unloads. Signed-out visitors can write; the
	// draft waits on this device through the sign-in round trip.
	//
	// Props:
	// - slug: the office this report is about; null shows a field for it.
	// - officeName: shown instead of the slug when known.
	// - apiBase: prefix before /api/v1 ('' on the public host).
	// - loginHref: the sign-in page; `?next=` returns here.
	// - quoteFrom: element whose selected text "Quote" may insert (the page).
	let {
		slug = null,
		officeName = '',
		apiBase = '',
		loginHref,
		quoteFrom = null
	}: {
		slug?: string | null;
		officeName?: string;
		apiBase?: string;
		loginHref: string;
		quoteFrom?: HTMLElement | null;
	} = $props();

	type Upload = {
		key: number;
		id: string | null;
		name: string;
		size: number;
		at: number;
		status: 'uploading' | 'done' | 'error';
		progress: number;
		error: string;
		preview: string;
		file: File | null;
	};

	const MAX_PHOTOS = 3;
	const MAX_PHOTO_BYTES = 8 * 1024 * 1024;
	const MAX_BODY_CHARS = 2000;
	const api = (path: string) => `${apiBase}/api/v1${path}`;

	const key = $derived(draftKey(slug));
	const store = (): Storage | null => {
		try {
			return typeof localStorage === 'undefined' ? null : localStorage;
		} catch {
			return null;
		}
	};

	let authority = $state('');
	let body = $state('');
	let uploads = $state<Upload[]>([]);
	let signedIn = $state<boolean | null>(null);
	let savedAt = $state<number | null>(null);
	let savedSnap = $state('');
	let busy = $state(false);
	let dragging = $state(false);
	let selection = $state('');
	let sentId = $state<number | null>(null);
	let notice = $state<{ variant: 'info' | 'error' | 'success'; heading: string; text: string } | null>(
		null
	);
	let nextKey = 0;
	let picker = $state<HTMLInputElement | null>(null);
	let note = $state<HTMLTextAreaElement | null>(null);
	let formEl = $state<HTMLFormElement | null>(null);

	const xhrs = new Map<number, XMLHttpRequest>();

	const draft = () => ({
		authority: (slug ?? authority).trim(),
		body,
		uploads: uploads
			.filter((u) => u.status === 'done' && u.id)
			.map((u) => ({ id: u.id as string, name: u.name, size: u.size, at: u.at }))
	});
	const snap = $derived(JSON.stringify(draft()));
	const dirty = $derived(snap !== savedSnap);
	const uploading = $derived(uploads.some((u) => u.status === 'uploading'));

	function save() {
		if (!dirty) return;
		const at = saveDraft(store(), key, draft());
		savedSnap = snap;
		savedAt = at;
	}

	const loginWithNext = $derived.by(() => {
		if (typeof window === 'undefined') return loginHref;
		const here = window.location.pathname + window.location.search + '#report';
		return `${loginHref}?next=${encodeURIComponent(here)}`;
	});

	onMount(() => {
		const d = loadDraft(store(), key);
		if (d) {
			authority = slug ? slug : d.authority;
			body = d.body;
			uploads = d.uploads.map((u) => ({
				...u,
				key: nextKey++,
				status: 'done' as const,
				progress: 1,
				error: '',
				preview: '',
				file: null
			}));
			savedAt = d.savedAt || null;
		} else if (slug) {
			authority = slug;
		}
		savedSnap = JSON.stringify(draft());

		fetch(api('/auth/me'), { credentials: 'same-origin' })
			.then(async (r) => {
				const me = r.ok ? await r.json().catch(() => ({})) : {};
				signedIn = typeof me?.user_id === 'string' && me.user_id !== '';
			})
			.catch(() => (signedIn = null));

		const timer = setInterval(save, AUTOSAVE_MS);
		const onHide = () => {
			if (document.visibilityState === 'hidden') save();
		};
		const onSelect = () => {
			const sel = window.getSelection();
			if (!sel || sel.isCollapsed || !quoteFrom) return;
			const node = sel.anchorNode;
			if (!node || !quoteFrom.contains(node) || formEl?.contains(node)) return;
			selection = sel.toString();
		};
		window.addEventListener('blur', save);
		window.addEventListener('pagehide', save);
		document.addEventListener('visibilitychange', onHide);
		document.addEventListener('selectionchange', onSelect);
		return () => {
			save();
			clearInterval(timer);
			window.removeEventListener('blur', save);
			window.removeEventListener('pagehide', save);
			document.removeEventListener('visibilitychange', onHide);
			document.removeEventListener('selectionchange', onSelect);
			for (const x of xhrs.values()) x.abort();
			for (const u of uploads) if (u.preview) URL.revokeObjectURL(u.preview);
		};
	});

	const find = (k: number) => uploads.find((u) => u.key === k);

	function send(k: number) {
		const u = find(k);
		if (!u || !u.file) return;
		u.status = 'uploading';
		u.progress = 0;
		u.error = '';
		const file = u.file;
		const xhr = new XMLHttpRequest();
		xhrs.set(k, xhr);
		xhr.open('POST', api(`/reports/uploads?filename=${encodeURIComponent(file.name)}`));
		xhr.withCredentials = true;
		xhr.setRequestHeader(
			'content-type',
			file.type.startsWith('image/') ? file.type : 'application/octet-stream'
		);
		xhr.upload.onprogress = (e) => {
			const cur = find(k);
			if (cur && e.lengthComputable) cur.progress = e.loaded / e.total;
		};
		const fail = (msg: string) => {
			const cur = find(k);
			if (!cur) return;
			cur.status = 'error';
			cur.error = msg;
		};
		xhr.onload = () => {
			xhrs.delete(k);
			const cur = find(k);
			if (!cur) return;
			let data: { id?: unknown; error?: unknown } = {};
			try {
				data = JSON.parse(xhr.responseText || '{}');
			} catch {
				/* non-JSON error page */
			}
			if (xhr.status === 201 && typeof data.id === 'string') {
				cur.id = data.id;
				cur.status = 'done';
				cur.progress = 1;
				cur.at = Date.now();
				save();
				return;
			}
			if (xhr.status === 401) {
				signedIn = false;
				fail('Sign in to upload photos.');
				return;
			}
			if (xhr.status === 429) return fail('Too many uploads just now. Retry in a minute.');
			fail(typeof data.error === 'string' ? data.error : 'Upload failed.');
		};
		xhr.onerror = () => {
			xhrs.delete(k);
			fail('Connection lost. Retry when you are back online.');
		};
		xhr.send(file);
	}

	async function addFiles(files: FileList | File[] | null) {
		notice = null;
		if (!files) return;
		for (const file of Array.from(files)) {
			if (uploads.length >= MAX_PHOTOS) {
				notice = { variant: 'error', heading: 'At most 3 photos.', text: 'Extra photos were skipped.' };
				break;
			}
			if (file.size > MAX_PHOTO_BYTES) {
				notice = { variant: 'error', heading: 'Photo too large.', text: `${file.name} is over 8 MiB.` };
				continue;
			}
			const head = new Uint8Array(await file.slice(0, 12).arrayBuffer());
			if (!photoKind(head)) {
				notice = {
					variant: 'error',
					heading: 'Photo type not allowed.',
					text: `${file.name} is not a JPEG, PNG, WebP or GIF photo.`
				};
				continue;
			}
			const k = nextKey++;
			uploads.push({
				key: k,
				id: null,
				name: file.name,
				size: file.size,
				at: Date.now(),
				status: 'uploading',
				progress: 0,
				error: '',
				preview: URL.createObjectURL(file),
				file
			});
			send(k);
		}
		if (picker) picker.value = '';
	}

	function remove(k: number) {
		const u = find(k);
		if (!u) return;
		xhrs.get(k)?.abort();
		xhrs.delete(k);
		if (u.preview) URL.revokeObjectURL(u.preview);
		if (u.id) {
			fetch(api(`/reports/uploads/${u.id}`), { method: 'DELETE', credentials: 'same-origin' }).catch(
				() => {}
			);
		}
		uploads = uploads.filter((x) => x.key !== k);
		save();
	}

	function quote() {
		const line = quoteLine(selection);
		if (!line) return;
		const sep = body && !body.endsWith('\n') ? '\n' : '';
		body = (body + sep + line).slice(0, MAX_BODY_CHARS);
		selection = '';
		note?.focus();
	}

	// Full class strings per state (never stacked).
	const dropCls = (over: boolean) =>
		over
			? `flex cursor-pointer flex-col items-center gap-1 rounded-sm border-2 border-dashed border-accent-strong bg-surface px-4 py-5 text-center transition-colors focus-within:ring-2 focus-within:ring-accent-strong`
			: `flex cursor-pointer flex-col items-center gap-1 rounded-sm border-2 border-dashed border-line px-4 py-5 text-center transition-colors hover:bg-surface focus-within:ring-2 focus-within:ring-accent-strong`;

	const size = (n: number) =>
		n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`;
	const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

	async function onSubmit(event: SubmitEvent) {
		event.preventDefault();
		notice = null;
		sentId = null;
		const office = (slug ?? authority).trim();
		if (!office) {
			notice = { variant: 'error', heading: 'Pick an office.', text: 'Tell us which issuing office this is about.' };
			return;
		}
		if (!body.trim()) {
			notice = { variant: 'error', heading: 'Describe the change.', text: 'A short note is required.' };
			note?.focus();
			return;
		}
		if (uploading) return;
		if (uploads.some((u) => u.status === 'error')) {
			notice = { variant: 'error', heading: 'A photo did not upload.', text: 'Retry it or remove it first.' };
			return;
		}
		save();
		busy = true;
		try {
			const res = await fetch(api('/reports'), {
				method: 'POST',
				credentials: 'same-origin',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					authority: office,
					body: body.trim(),
					upload_ids: uploads.filter((u) => u.id).map((u) => u.id)
				})
			});
			if (res.status === 401) {
				signedIn = false;
				return;
			}
			if (res.status === 429) {
				notice = { variant: 'error', heading: 'Slow down.', text: 'Too many reports just now. Try again in a bit.' };
				return;
			}
			const data = await res.json().catch(() => ({}));
			if (!res.ok) {
				notice = {
					variant: 'error',
					heading: 'Report not sent. Your draft is kept.',
					text: typeof data?.error === 'string' ? data.error : 'Try again in a bit.'
				};
				return;
			}
			sentId = typeof data?.id === 'number' ? data.id : null;
			clearDraft(store(), key);
			for (const u of uploads) if (u.preview) URL.revokeObjectURL(u.preview);
			body = '';
			uploads = [];
			savedSnap = JSON.stringify(draft());
			savedAt = null;
			notice = {
				variant: 'success',
				heading: sentId !== null ? `Report #${sentId} received.` : 'Report received.',
				text: 'It stays private while we remove personal details. Nothing you wrote shows publicly until then.'
			};
		} catch {
			notice = {
				variant: 'error',
				heading: 'Report not sent. Your draft is kept.',
				text: 'The service is unreachable. Try again in a bit.'
			};
		} finally {
			busy = false;
		}
	}
</script>

<form bind:this={formEl} class="flex flex-col gap-4" onsubmit={onSubmit} novalidate>
	{#if notice}
		<Notice variant={notice.variant} heading={notice.heading}><p>{notice.text}</p></Notice>
	{/if}

	{#if signedIn === false}
		<Notice variant="info" heading="Sign in to send your report.">
			<p>
				Keep writing: your note is saved on this device.
				<a href={loginWithNext} onclick={save}>Sign in</a> and you come back here with it.
			</p>
		</Notice>
	{/if}

	{#if slug}
		<p class="m-0 text-sm text-ink-muted">
			About <span class="font-medium text-ink">{officeName || slug}</span>
		</p>
	{:else}
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
	{/if}

	<div class="flex flex-col gap-1">
		<div class="flex items-end justify-between gap-2">
			<label for="report-body" class="text-sm font-medium">What changed?</label>
			<span class="text-xs text-ink-muted tabular-nums">{body.length}/{MAX_BODY_CHARS}</span>
		</div>
		<textarea
			id="report-body"
			bind:this={note}
			class="min-h-36 rounded-sm border border-line bg-bg px-3 py-2"
			name="body"
			bind:value={body}
			onblur={save}
			maxlength={MAX_BODY_CHARS}
			placeholder="e.g. The window now says the fee is $20, not $15. Seen on 4 Nov."
			required
		></textarea>
		{#if quoteFrom}
			<div class="flex min-h-8 flex-wrap items-center gap-2 text-xs text-ink-muted">
				{#if selection.trim()}
					<button
						type="button"
						onclick={quote}
						class="inline-flex min-h-8 items-center gap-1.5 rounded-sm border border-line bg-surface px-2.5 font-medium text-ink hover:bg-bg"
					>
						<TextQuote class="size-4" aria-hidden="true" />
						Quote the text you selected
					</button>
				{:else}
					<span>Tip: select text on the page to quote it here.</span>
				{/if}
			</div>
		{/if}
	</div>

	<div class="flex flex-col gap-2">
		<span class="text-sm font-medium" id="report-photos">
			Photos <span class="font-normal text-ink-muted">(optional, up to {MAX_PHOTOS})</span>
		</span>
		{#if uploads.length < MAX_PHOTOS}
			<label
				class={dropCls(dragging)}
				ondragover={(e) => {
					e.preventDefault();
					dragging = true;
				}}
				ondragleave={() => (dragging = false)}
				ondrop={(e) => {
					e.preventDefault();
					dragging = false;
					addFiles(e.dataTransfer?.files ?? null);
				}}
			>
				<input
					bind:this={picker}
					class="sr-only"
					type="file"
					accept="image/jpeg,image/png,image/webp,image/gif"
					multiple
					aria-describedby="report-photos report-photos-hint"
					onchange={(e) => addFiles(e.currentTarget.files)}
				/>
				<span
					class="inline-flex min-h-11 items-center gap-2 rounded-sm bg-accent-fill px-4 font-medium text-accent-ink"
				>
					<ImagePlus class="size-5" aria-hidden="true" />
					Choose photos
				</span>
				<span id="report-photos-hint" class="text-xs text-ink-muted">
					or drop them here · JPEG, PNG, WebP or GIF, 8 MB each · uploads right away
				</span>
			</label>
		{/if}

		{#if uploads.length > 0}
			<ul class="m-0 flex list-none flex-col gap-2 p-0" aria-live="polite">
				{#each uploads as u (u.key)}
					<li class="flex items-center gap-3 rounded-sm border border-line bg-bg p-2">
						{#if u.preview}
							<img src={u.preview} alt="" class="size-12 flex-none rounded-xs object-cover" />
						{:else}
							<span class="grid size-12 flex-none place-items-center rounded-xs bg-surface text-ink-muted">
								<ImagePlus class="size-5" aria-hidden="true" />
							</span>
						{/if}
						<div class="min-w-0 flex-1">
							<p class="m-0 truncate text-sm text-ink">{u.name}</p>
							{#if u.status === 'uploading'}
								<div class="mt-1 h-1.5 overflow-hidden rounded-full bg-surface" aria-hidden="true">
									<div
										class="h-full bg-accent-fill transition-[width]"
										style="width: {Math.round(u.progress * 100)}%"
									></div>
								</div>
								<p class="m-0 mt-0.5 text-xs text-ink-muted">Uploading… {Math.round(u.progress * 100)}%</p>
							{:else if u.status === 'done'}
								<p class="m-0 mt-0.5 inline-flex items-center gap-1 text-xs text-ink-muted">
									<CheckIcon class="size-3.5" aria-hidden="true" /> Uploaded · {size(u.size)}
								</p>
							{:else}
								<p class="m-0 mt-0.5 text-xs text-error" role="alert">{u.error}</p>
							{/if}
						</div>
						{#if u.status === 'uploading'}
							<LoaderCircle class="size-4 flex-none animate-spin text-ink-muted" aria-hidden="true" />
						{/if}
						{#if u.status === 'error' && u.file && signedIn !== false}
							<button
								type="button"
								onclick={() => send(u.key)}
								class="inline-flex size-9 flex-none items-center justify-center rounded-sm hover:bg-surface"
								aria-label="Retry {u.name}"
							>
								<RotateCw class="size-4" aria-hidden="true" />
							</button>
						{/if}
						<button
							type="button"
							onclick={() => remove(u.key)}
							class="inline-flex size-9 flex-none items-center justify-center rounded-sm hover:bg-surface"
							aria-label="Remove {u.name}"
						>
							<XIcon class="size-4" aria-hidden="true" />
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>

	<div class="flex flex-wrap items-center gap-3">
		<button
			type="submit"
			disabled={busy || uploading || signedIn === false}
			class="inline-flex min-h-11 items-center justify-center gap-2 rounded-sm bg-accent-fill px-5 font-medium text-accent-ink transition-colors hover:bg-accent-hover disabled:opacity-60"
		>
			{#if busy}<LoaderCircle class="size-4 animate-spin" aria-hidden="true" />{/if}
			{busy ? 'Sending…' : uploading ? 'Waiting for photos…' : 'Send report'}
		</button>
		<span class="text-xs text-ink-muted" aria-live="polite">
			{#if dirty}
				Unsaved changes · saves automatically
			{:else if savedAt}
				Draft saved on this device at {clock(savedAt)}
			{/if}
		</span>
	</div>
	<p class="m-0 text-xs text-ink-muted">
		Photos lose their camera metadata on our end; your note stays private while personal details are
		removed.
	</p>
</form>
