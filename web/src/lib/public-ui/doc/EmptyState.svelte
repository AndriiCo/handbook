<script lang="ts">
	import { resolve } from '$app/paths';
	import SearchXIcon from '@lucide/svelte/icons/search-x';
	import { guide } from './lang.svelte.js';

	// An office we haven't checked (plan §3 States, D11): no values at
	// all, and two ways on. The listed official site (the authority row's
	// `official_url`) is printed as plain text, "not yet checked by us",
	// never as a link: nobody here has matched anything to it. No trust
	// line, citations switch, offers or records.
	//
	// Props:
	// - empty: `page.empty` ({ name, officialHost, officialUrl }).
	let { empty }: { empty: { name: string; officialHost: string | null; officialUrl: string | null } } = $props();

	const t = $derived(guide.t.empty);
</script>

<section aria-labelledby="empty-h" class="border-t border-line py-10 xl:py-16" data-empty>
	<div class="max-w-3xl">
		<h2 id="empty-h" class="m-0 flex items-center gap-3 font-brand text-h2 font-medium text-ink">
			<SearchXIcon class="size-7 flex-none text-ink-muted" aria-hidden="true" />
			{t.heading}
		</h2>
		<p class="mt-3 mb-0 max-w-prose text-ink">{t.body}</p>
		<ul class="m-0 mt-6 flex list-none flex-col gap-3 p-0">
			{#if empty.officialUrl || empty.officialHost}
				<li class="m-0 p-0 text-ink">
					<span class="text-ink-muted">{t.listed}</span>
					<span class="font-mono text-sm break-all">{empty.officialUrl ?? empty.officialHost}</span>
					<span class="text-ink-muted">{t.notChecked}</span>
				</li>
			{/if}
			<li class="m-0 p-0">
				<a href={resolve('/(public)')} class="inline-flex min-h-11 items-center font-medium text-accent-strong underline underline-offset-4"
					>{t.browse}</a
				>
			</li>
		</ul>
	</div>
</section>
