<script lang="ts">
	import { tick } from 'svelte';
	import { toast } from 'svelte-sonner';
	import * as Dialog from '#lib/components/ui/dialog/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import { EvidenceError, RETIRE_MAX, RETIRE_MIN, retireBody, retireFact } from './evidence-client.js';
	import ArchiveXIcon from '@lucide/svelte/icons/archive-x';

	// "Retire this fact…" (plan §5.5): undoes a wrong approval. A staff
	// reason (8-500 characters) and a confirmation step that says what
	// happens, then POST /admin/facts/{id}/retire. The server turns the
	// fact `superseded`, audits the reason and announces `fact_retired`;
	// every public read drops it at once. `onRetired` lets the list show
	// the row as retired with its reason; `onPending` brackets the request
	// (its `fact_retired` event can arrive before the response does).
	// `focusAfter` names the element (the row's Retired note) that takes
	// focus once this trigger has unmounted, so focus never drops to body.
	let {
		fact,
		onRetired,
		onPending,
		focusAfter,
		size = 'sm'
	}: {
		fact: { id: number; statement: string; authority_slug: string };
		onRetired?: (reason: string) => void;
		onPending?: (pending: boolean) => void;
		focusAfter?: string;
		size?: 'sm' | 'default';
	} = $props();

	const uid = $props.id();
	let open = $state(false);
	let reason = $state('');
	let busy = $state(false);
	let error = $state<string | null>(null);
	const count = $derived([...reason.trim()].length);
	const body = $derived(retireBody(reason));

	async function start() {
		reason = '';
		error = null;
		open = true;
		await tick();
		document.getElementById(`${uid}-reason`)?.focus();
	}

	async function confirm() {
		const payload = body;
		if (!payload || busy) return;
		const { id, authority_slug } = fact;
		busy = true;
		error = null;
		onPending?.(true);
		try {
			await retireFact('/apostille/api/v1', id, payload);
			open = false;
			toast.success(`Fact #${id} retired: it is off the ${authority_slug} guide.`);
			onRetired?.(payload.reason);
			if (focusAfter) {
				const target = focusAfter;
				await tick();
				requestAnimationFrame(() => document.getElementById(target)?.focus());
			}
		} catch (err) {
			error =
				err instanceof EvidenceError && err.reason
					? `Not retired: ${err.reason}.`
					: err instanceof Error
						? err.message
						: 'Retire failed.';
		} finally {
			busy = false;
			onPending?.(false);
		}
	}
</script>

<Button variant="outline" {size} class="text-error" aria-haspopup="dialog" onclick={start}>
	<ArchiveXIcon aria-hidden="true" />Retire this fact…<span class="sr-only"> (#{fact.id})</span>
</Button>

<Dialog.Root bind:open>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>Retire fact #{fact.id}?</Dialog.Title>
			<Dialog.Description>
				The fact disappears from the public guide as soon as you confirm, with every page block,
				office card and note that quotes it. The reason stays in the audit log.
			</Dialog.Description>
		</Dialog.Header>
		<p class="m-0 rounded-sm border border-line px-3 py-2 text-sm break-words">{fact.statement}</p>
		<label class="flex flex-col gap-1 text-sm font-medium" for={`${uid}-reason`}>
			Reason (required)
			<Textarea id={`${uid}-reason`} rows={3} bind:value={reason} aria-describedby={`${uid}-count`} disabled={busy} />
		</label>
		<p id={`${uid}-count`} class={`m-0 text-xs ${count > RETIRE_MAX ? 'text-error' : 'text-ink-muted'}`}>
			{count}/{RETIRE_MAX} characters{count < RETIRE_MIN ? ` · at least ${RETIRE_MIN}` : ''}
		</p>
		{#if error}
			<p class="m-0 text-sm font-semibold text-error" role="alert">{error}</p>
		{/if}
		<Dialog.Footer>
			<Button variant="ghost" disabled={busy} onclick={() => (open = false)}>Cancel</Button>
			<Button variant="destructive" disabled={busy || !body} onclick={confirm}>
				{busy ? 'Retiring…' : `Retire fact #${fact.id}`}
			</Button>
		</Dialog.Footer>
	</Dialog.Content>
</Dialog.Root>
