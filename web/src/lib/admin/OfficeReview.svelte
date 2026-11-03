<script lang="ts">
	import { onMount } from 'svelte';
	import { toast } from 'svelte-sonner';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import Notice from '#lib/public-ui/Notice.svelte';
	import GuardList from './GuardList.svelte';
	import DecisionBar from './DecisionBar.svelte';
	import { clock24, dayRange, decide, fetchOffices, type OfficeProposal } from './proposals.js';
	import { errorText } from './admin-api.js';
	import MapPinIcon from '@lucide/svelte/icons/map-pin';
	import MailIcon from '@lucide/svelte/icons/mail';
	import ClockIcon from '@lucide/svelte/icons/clock';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';

	// Offices tab: Muse Spark extracts counters, mail addresses and
	// hours from accepted facts; guards check each value against its
	// cited fact. Accept → `offices_accepted` on `authority:{slug}`.
	let { onCount }: { onCount?: (n: number) => void } = $props();

	let offices = $state<OfficeProposal[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let acting = $state<number | null>(null);

	async function load() {
		loadState = 'loading';
		try {
			offices = await fetchOffices();
			loadState = 'ready';
			onCount?.(offices.filter((o) => o.status === 'proposed').length);
		} catch (err) {
			loadError = errorText(err, 'Offices failed to load.');
			loadState = 'failed';
		}
	}

	onMount(() => {
		void load();
	});

	const VISIT: Record<string, string> = {
		walk_in: 'Walk-in',
		appointment: 'By appointment',
		mail_only: 'Mail only'
	};

	async function act(o: OfficeProposal, action: 'accept' | 'reject', reason?: string) {
		if (acting !== null) return;
		acting = o.id;
		const prior = o.status;
		o.status = action === 'accept' ? 'accepted' : 'rejected';
		try {
			o.status = await decide('offices', o.id, action, reason);
			onCount?.(offices.filter((x) => x.status === 'proposed').length);
			toast.success(action === 'accept' ? `${o.name} is live on ${o.authority_slug}.` : `${o.name} rejected.`);
		} catch (err) {
			o.status = prior;
			toast.error(errorText(err, 'Decision failed — nothing changed.'));
		} finally {
			acting = null;
		}
	}
</script>

{#if loadState === 'loading'}
	<div class="grid gap-4 md:grid-cols-2" role="status" aria-label="Loading offices">
		<Skeleton class="h-64 rounded-md" />
		<Skeleton class="h-64 rounded-md" />
	</div>
{:else if loadState === 'failed'}
	<Notice variant="warning" heading="Office proposals are unavailable.">
		<p class="m-0">{loadError}</p>
		<p class="mt-3 mb-0"><Button variant="outline" onclick={load}>Try again</Button></p>
	</Notice>
{:else if offices.length === 0}
	<Card.Root class="items-start px-6 py-10">
		<p class="m-0 text-h3 font-semibold">No offices waiting</p>
		<p class="m-0 text-sm text-ink-muted">Office cards are proposed after address and hours facts are accepted.</p>
		<Button variant="outline" href="/compile">Start a compile run</Button>
	</Card.Root>
{:else}
	<ul class="m-0 grid list-none gap-4 p-0 lg:grid-cols-2">
		{#each offices as o (o.id)}
			{@const failing = o.guards.filter((g) => !g.ok)}
			<li class="min-w-0">
				<Card.Root class="h-full gap-4 shadow-sm">
					<Card.Header>
						<Card.Title class="flex flex-wrap items-center gap-2">
							<h2 class="m-0 text-body font-semibold">{o.name}</h2>
							{#if o.visit}<Badge variant="secondary">{VISIT[o.visit.kind] ?? o.visit.kind}</Badge>{/if}
						</Card.Title>
						<Card.Description class="font-mono text-xs">{o.authority_slug} · office #{o.id}</Card.Description>
					</Card.Header>
					<Card.Content class="flex flex-col gap-4">
						<dl class="m-0 grid gap-3 text-sm sm:grid-cols-2">
							{#if o.address}
								<div>
									<dt class="flex items-center gap-1 text-xs text-ink-muted"><MapPinIcon class="size-3" aria-hidden="true" />Counter</dt>
									<dd class="m-0">
										{o.address.line1}{o.address.line2 ? `, ${o.address.line2}` : ''}<br />
										{o.address.city}, {o.address.region} {o.address.postal_code}
										{#if o.address.fact_id}<span class="font-mono text-xs text-ink-muted"> · fact #{o.address.fact_id}</span>{/if}
									</dd>
								</div>
							{/if}
							{#if o.mail}
								<div>
									<dt class="flex items-center gap-1 text-xs text-ink-muted"><MailIcon class="size-3" aria-hidden="true" />Mail{o.mail.usps_only ? ' (USPS only)' : ''}</dt>
									<dd class="m-0">
										{o.mail.line1}<br />
										{o.mail.city}, {o.mail.region} {o.mail.postal_code}
										{#if o.mail.fact_id}<span class="font-mono text-xs text-ink-muted"> · fact #{o.mail.fact_id}</span>{/if}
									</dd>
								</div>
							{/if}
							{#if o.hours.length > 0}
								<div>
									<dt class="flex items-center gap-1 text-xs text-ink-muted"><ClockIcon class="size-3" aria-hidden="true" />Hours</dt>
									{#each o.hours as h, hi (hi)}
										<dd class="m-0">{dayRange(h.days)} {clock24(h.open)}–{clock24(h.close)}</dd>
									{/each}
									<dd class="m-0 text-xs text-ink-muted">
										{o.timezone ?? 'No timezone'}{o.timezone_source ? ` · ${o.timezone_source.replace('_', ' ')}` : ''}
									</dd>
								</div>
							{/if}
							{#if o.visit?.notes}
								<div>
									<dt class="text-xs text-ink-muted">Visit notes</dt>
									<dd class="m-0">{o.visit.notes}</dd>
								</div>
							{/if}
						</dl>
						{#if failing.length > 0}
							<div class="flex items-start gap-2 rounded-sm bg-changed-bg px-3 py-2 text-sm" role="note">
								<TriangleAlertIcon class="mt-0.5 size-4 flex-none text-changed" aria-hidden="true" />
								<span>Accepting confirms {failing.map((g) => g.id.replace('_', ' ')).join(', ')} on staff authority.</span>
							</div>
						{/if}
						<GuardList guards={o.guards} label={`Guards for ${o.name}`} />
						{#if o.directions_url}
							<a class="text-sm" href={o.directions_url} rel="noopener noreferrer" target="_blank">Check on the map</a>
						{/if}
						<DecisionBar
							noun="office"
							id={`office-${o.id}`}
							status={o.status}
							acting={acting === o.id}
							onAccept={() => act(o, 'accept')}
							onReject={(reason) => act(o, 'reject', reason)}
						/>
					</Card.Content>
				</Card.Root>
			</li>
		{/each}
	</ul>
{/if}
