<script lang="ts">
	import { resolve } from '$app/paths';

	let { websiteUrl = null as string | null, slug = '' }: { websiteUrl: string | null; slug: string } =
		$props();

	const orderHref = $derived(
		`${resolve('/(customer)/order')}?destination=${encodeURIComponent(slug)}`
	);
</script>

<div class="rounded-xl border border-line bg-surface p-5">
	<p class="m-0">
		<span
			class="inline-block rounded-full border border-line bg-bg px-2 py-0.5 text-xs font-semibold text-ink-muted"
		>
			Not yet verified
		</span>
	</p>
	<p class="text-body-lg mt-3 mb-0 max-w-prose text-ink">
		We are still verifying each step for this authority against official
		sources. Every step appears here once staff approve its evidence.
		General information, not legal advice.
	</p>
	<p class="mt-4 mb-0">
		{#if websiteUrl}
			<a
				href={websiteUrl}
				target="_blank"
				rel="noopener noreferrer"
				class="inline-flex min-h-11 items-center rounded-sm bg-accent-fill px-5 font-medium text-accent-ink no-underline transition-colors hover:bg-accent-hover"
			>
				Check the official website
			</a>
		{:else}
			<a
				href={orderHref}
				class="inline-flex min-h-11 items-center rounded-sm bg-accent-fill px-5 font-medium text-accent-ink no-underline transition-colors hover:bg-accent-hover"
			>
				Start an order and we'll confirm it
			</a>
		{/if}
	</p>
</div>
