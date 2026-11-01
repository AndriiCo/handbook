<script lang="ts">
	import * as Card from '../components/ui/card/index.js';
	import { Input } from '../components/ui/input/index.js';
	import { Button } from '../components/ui/button/index.js';
	import BellRing from '@lucide/svelte/icons/bell-ring';

	let { slug, name, apiBase }: { slug: string; name: string; apiBase: string } = $props();

	let email = $state('');
	let busy = $state(false);
	let done = $state(false);
	let error = $state<string | null>(null);

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
			error = 'Enter a valid email address.';
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
				error = 'Enter a valid email address.';
			} else if (res.status === 429) {
				error = 'Too many tries — wait a few minutes and try again.';
			} else if (res.status === 503) {
				error = 'Follow updates are off right now — try again later.';
			} else {
				error = 'Something went wrong — try again later.';
			}
		} catch {
			error = 'Something went wrong — try again later.';
		} finally {
			busy = false;
		}
	}
</script>

<Card.Root id="follow" class="scroll-mt-32">
	<Card.Header>
		<Card.Title class="flex items-center gap-2">
			<BellRing class="size-5 shrink-0" aria-hidden="true" />
			Get updates for {name}
		</Card.Title>
		<Card.Description>
			One email when staff accept a changed fact for this office. Nothing else, ever.
		</Card.Description>
	</Card.Header>
	<Card.Content>
		{#if done}
			<div role="status" class="rounded-lg border border-verified/40 bg-verified-bg p-4">
				<p class="m-0 font-medium text-ink">Check your inbox to confirm.</p>
				<p class="mt-1 mb-0 text-sm text-ink-muted">
					We sent a confirm link to {email.trim()} — it dies in 24 hours, and nothing
					sends until you click it.
				</p>
			</div>
		{:else}
			<form class="flex flex-col gap-3 sm:flex-row" onsubmit={submit}>
				<div class="min-w-0 flex-1">
					<label for="follow-email" class="sr-only">Email address</label>
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
					{busy ? 'Sending…' : 'Follow'}
				</Button>
			</form>
			{#if error}
				<p id="follow-error" role="alert" class="mt-2 mb-0 text-sm text-destructive">{error}</p>
			{/if}
		{/if}
	</Card.Content>
	<Card.Footer class="flex-col items-start gap-2">
		<p class="m-0 text-xs text-ink-muted">
			Privacy: we store only your address (sealed — staff cannot read it) and which
			office you follow. Confirm before anything sends; at most one digest per office
			per hour; every email carries a one-click unsubscribe link and our postal
			address. Unsubscribing always works, instantly.
		</p>
	</Card.Footer>
</Card.Root>
