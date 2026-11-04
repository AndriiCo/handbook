<script lang="ts">
	import * as Card from '../components/ui/card/index.js';
	import { Input } from '../components/ui/input/index.js';
	import { Button } from '../components/ui/button/index.js';
	import BellRing from '@lucide/svelte/icons/bell-ring';
	import { guide } from './doc/lang.svelte.js';

	let { slug, name, apiBase }: { slug: string; name: string; apiBase: string } = $props();

	const t = $derived(guide.t.follow);
	let email = $state('');
	let busy = $state(false);
	let done = $state(false);
	let error = $state<string | null>(null);
	// The card renders only while the notify master switch is on
	// (production runs off). `null` = still checking: render
	// nothing yet, so the dossier never flashes a dead form.
	let enabled = $state<boolean | null>(null);

	$effect(() => {
		let cancelled = false;
		fetch(`${apiBase}/api/v1/notify/status`, { credentials: 'same-origin' })
			.then((res) => (res.ok ? res.json() : { enabled: false }))
			.then((body) => {
				if (!cancelled) enabled = body?.enabled === true;
			})
			.catch(() => {
				if (!cancelled) enabled = false;
			});
		return () => {
			cancelled = true;
		};
	});

	function csrf(): string {
		for (const part of document.cookie.split(';')) {
			const [k, ...rest] = part.trim().split('=');
			if (k === 'hb_csrf') return rest.join('=');
		}
		return '';
	}

	async function submit(event: Event) {
		event.preventDefault();
		if (busy || done) return;
		error = null;
		const address = email.trim();
		if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(address)) {
			error = t.invalid;
			return;
		}
		busy = true;
		try {
			const res = await fetch(`${apiBase}/api/v1/notify/subscribe`, {
				method: 'POST',
				headers: { 'content-type': 'application/json', 'x-csrf': csrf() },
				credentials: 'same-origin',
				body: JSON.stringify({ authority: slug, email: address })
			});
			if (res.ok) {
				done = true;
			} else if (res.status === 400) {
				error = t.invalid;
			} else if (res.status === 429) {
				error = t.tooMany;
			} else if (res.status === 503) {
				error = t.off;
			} else {
				error = t.failed;
			}
		} catch {
			error = t.failed;
		} finally {
			busy = false;
		}
	}
</script>

{#if enabled}
<Card.Root id="follow" class="scroll-mt-32">
	<Card.Header>
		<Card.Title class="flex items-center gap-2">
			<BellRing class="size-5 shrink-0" aria-hidden="true" />
			{t.title(name)}
		</Card.Title>
		<Card.Description>{t.description}</Card.Description>
	</Card.Header>
	<Card.Content>
		{#if done}
			<div role="status" class="rounded-lg border border-verified/40 bg-verified-bg p-4">
				<p class="m-0 font-medium text-ink">{t.checkInbox}</p>
				<p class="mt-1 mb-0 text-sm text-ink-muted">{t.sent(email.trim())}</p>
			</div>
		{:else}
			<form class="flex flex-col gap-3 sm:flex-row" onsubmit={submit}>
				<div class="min-w-0 flex-1">
					<label for="follow-email" class="sr-only">{t.emailLabel}</label>
					<Input
						id="follow-email"
						type="email"
						name="email"
						autocomplete="email"
						placeholder="you@example.org"
						bind:value={email}
						disabled={busy}
						class="min-h-11"
						aria-describedby={error ? 'follow-error' : undefined}
					/>
				</div>
				<Button type="submit" disabled={busy} class="min-h-11 sm:w-auto">
					{busy ? t.sending : t.follow}
				</Button>
			</form>
			{#if error}
				<p id="follow-error" role="alert" class="mt-2 mb-0 text-sm text-destructive">{error}</p>
			{/if}
		{/if}
	</Card.Content>
	<Card.Footer class="flex-col items-start gap-2">
		<p class="m-0 text-xs text-ink-muted">{t.privacy}</p>
	</Card.Footer>
</Card.Root>
{/if}
