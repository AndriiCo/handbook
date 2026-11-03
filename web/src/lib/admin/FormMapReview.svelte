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
	import { decide, fetchFormMaps, type FormMap } from './proposals.js';
	import { errorText } from './admin-api.js';
	import FileTextIcon from '@lucide/svelte/icons/file-text';

	// Forms tab: Muse Spark labels each AcroForm field of an official
	// PDF in plain words; guards check every label against the PDF text
	// layer. Accept inserts the `form_template` row customers fill.
	let { onCount }: { onCount?: (n: number) => void } = $props();

	let maps = $state<FormMap[]>([]);
	let loadState = $state<'loading' | 'ready' | 'failed'>('loading');
	let loadError = $state<string | null>(null);
	let acting = $state<number | null>(null);

	async function load() {
		loadState = 'loading';
		try {
			maps = await fetchFormMaps();
			loadState = 'ready';
			onCount?.(maps.filter((m) => m.status === 'proposed').length);
		} catch (err) {
			loadError = errorText(err, 'Form maps failed to load.');
			loadState = 'failed';
		}
	}

	onMount(() => {
		void load();
	});

	function host(url: string): string {
		try {
			return new URL(url).host;
		} catch {
			return url;
		}
	}

	async function act(m: FormMap, action: 'accept' | 'reject', reason?: string) {
		if (acting !== null) return;
		acting = m.id;
		const prior = m.status;
		m.status = action === 'accept' ? 'accepted' : 'rejected';
		try {
			m.status = await decide('form-maps', m.id, action, reason);
			onCount?.(maps.filter((x) => x.status === 'proposed').length);
			toast.success(action === 'accept' ? `Form #${m.id} is fillable on ${m.authority_slug}.` : `Form #${m.id} rejected.`);
		} catch (err) {
			m.status = prior;
			toast.error(errorText(err, 'Decision failed — nothing changed.'));
		} finally {
			acting = null;
		}
	}
</script>

{#if loadState === 'loading'}
	<div role="status" aria-label="Loading form maps">
		<Skeleton class="h-72 rounded-md" />
	</div>
{:else if loadState === 'failed'}
	<Notice variant="warning" heading="Form maps are unavailable.">
		<p class="m-0">{loadError}</p>
		<p class="mt-3 mb-0"><Button variant="outline" onclick={load}>Try again</Button></p>
	</Notice>
{:else if maps.length === 0}
	<Card.Root class="items-start px-6 py-10">
		<p class="m-0 text-h3 font-semibold">No form maps waiting</p>
		<p class="m-0 text-sm text-ink-muted">Upload an official fillable PDF and Muse Spark maps its fields.</p>
		<Button variant="outline" href="/workers">Upload a form</Button>
	</Card.Root>
{:else}
	<ul class="m-0 flex list-none flex-col gap-4 p-0">
		{#each maps as m (m.id)}
			<li class="min-w-0">
				<Card.Root class="gap-4 shadow-sm">
					<Card.Header>
						<Card.Title class="flex flex-wrap items-center gap-2">
							<FileTextIcon class="size-4 text-ink-muted" aria-hidden="true" />
							<h2 class="m-0 text-body font-semibold">{m.authority_slug} · form #{m.id}</h2>
							<Badge variant="outline" class={m.mapped === m.acroform_fields ? 'border-verified text-verified' : 'border-changed text-changed'}>
								{m.mapped}/{m.acroform_fields} fields mapped
							</Badge>
						</Card.Title>
						<Card.Description class="truncate">
							{#if m.source_url}<a class="underline underline-offset-2" href={m.source_url} rel="noopener noreferrer" target="_blank">{host(m.source_url)}</a>{/if}
							{m.capture_id ? ` · capture #${m.capture_id}` : ''}
						</Card.Description>
					</Card.Header>
					<Card.Content class="flex flex-col gap-4">
						<div class="grid gap-4 lg:grid-cols-3">
							<div class="min-w-0 lg:col-span-2">
								<table class="w-full border-collapse text-sm">
									<caption class="sr-only">Mapped fields for form #{m.id}</caption>
									<thead class="sr-only md:not-sr-only">
										<tr class="border-b border-line text-left text-xs text-ink-muted">
											<th scope="col" class="py-2 pr-3 font-medium">Field</th>
											<th scope="col" class="py-2 pr-3 font-medium">Plain label</th>
											<th scope="col" class="py-2 pr-3 font-medium">Type</th>
											<th scope="col" class="py-2 font-medium">Source text</th>
										</tr>
									</thead>
									<tbody>
										{#each m.fields as f (f.name)}
											<tr class="flex flex-col gap-1 border-b border-line py-2 md:table-row">
												<td class="font-mono text-xs md:py-2 md:pr-3">{f.name}</td>
												<td class="md:py-2 md:pr-3">
													<span class="font-medium">{f.label}</span>{#if f.required}<span class="text-ink-muted">{' · required'}</span>{/if}
													{#if f.help}<span class="block text-xs text-ink-muted">{f.help}</span>{/if}
												</td>
												<td class="md:py-2 md:pr-3">
													<Badge variant="secondary">{f.type}</Badge>
													{#if f.prefill_topic}<span class="block text-xs text-ink-muted">prefill: {f.prefill_topic}</span>{/if}
												</td>
												<td class="font-quote text-ink-muted italic md:py-2">“{f.label_quote}”</td>
											</tr>
										{/each}
									</tbody>
								</table>
							</div>
							<div class="flex flex-col gap-4">
								<GuardList guards={m.guards} label={`Guards for form #${m.id}`} />
								<DecisionBar
									noun="form map"
									id={`form-${m.id}`}
									status={m.status}
									acting={acting === m.id}
									onAccept={() => act(m, 'accept')}
									onReject={(reason) => act(m, 'reject', reason)}
								/>
							</div>
						</div>
					</Card.Content>
				</Card.Root>
			</li>
		{/each}
	</ul>
{/if}
