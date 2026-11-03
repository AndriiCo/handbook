<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import { Textarea } from '#lib/components/ui/textarea/index.js';
	import CheckIcon from '@lucide/svelte/icons/check';
	import XIcon from '@lucide/svelte/icons/x';

	// Accept / Reject for one proposal. Buttons name the noun ("Accept
	// plan v4") so they never collide with the facts tab's "Accept".
	let {
		noun,
		id,
		status,
		acting = false,
		onAccept,
		onReject
	}: {
		noun: string;
		id: string;
		status: string;
		acting?: boolean;
		onAccept: () => void;
		onReject: (reason: string) => void;
	} = $props();

	let rejectOpen = $state(false);
	let reason = $state('');
</script>

{#if status !== 'proposed'}
	<p
		class={`m-0 inline-flex items-center gap-2 rounded-sm px-3 py-2 text-sm font-semibold ${status === 'accepted' ? 'bg-verified-bg text-verified' : 'bg-surface-2 text-ink'}`}
		role="status"
	>
		{#if status === 'accepted'}<CheckIcon class="size-4" aria-hidden="true" />{:else}<XIcon class="size-4" aria-hidden="true" />{/if}
		{noun.charAt(0).toUpperCase() + noun.slice(1)} {status}.
	</p>
{:else}
	<div class="flex flex-col gap-2">
		<div class="flex flex-wrap gap-2">
			<Button size="lg" disabled={acting} onclick={onAccept}>
				<CheckIcon aria-hidden="true" />
				Accept {noun}
			</Button>
			<Button size="lg" variant="outline" disabled={acting} onclick={() => (rejectOpen = !rejectOpen)} aria-expanded={rejectOpen}>
				{rejectOpen ? 'Keep it' : 'Reject…'}
			</Button>
		</div>
		{#if rejectOpen}
			<label class="flex flex-col gap-1 text-sm font-medium" for={`reject-${id}`}>
				Reason (required, audited)
				<Textarea id={`reject-${id}`} rows={2} bind:value={reason} />
			</label>
			<div>
				<Button
					variant="destructive"
					size="lg"
					disabled={acting || reason.trim().length === 0 || reason.length > 1024}
					onclick={() => onReject(reason.trim())}
				>
					Reject {noun}
				</Button>
			</div>
		{/if}
	</div>
{/if}
