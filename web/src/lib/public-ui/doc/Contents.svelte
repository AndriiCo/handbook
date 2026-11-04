<script lang="ts">
	import { guide } from './lang.svelte.js';

	// "On this page" (plan §3 row 2): links to the guide's sections. In
	// the wide layout it sits in the rail beside the reading column and
	// stays in view (nothing critical lives in it); on phones it is an
	// ordinary list in the flow, never a sticky bar (WCAG 2.4.11).
	//
	// Props:
	// - links: { id, label }[], one per rendered section, in page order.
	// - current: the id of the section in view, or null.
	let {
		links = [],
		current = null
	}: { links?: { id: string; label: string }[]; current?: string | null } = $props();
</script>

{#if links.length > 0}
	<nav aria-label={guide.t.contents.label}>
		<p class="text-eyebrow m-0 font-semibold tracking-[0.06em] text-ink-muted uppercase">{guide.t.contents.label}</p>
		<ul class="m-0 mt-2 flex list-none flex-wrap gap-x-1 gap-y-0 p-0 xl:flex-col">
			{#each links as link (link.id)}
				<li class="m-0 p-0">
					<a
						href={`#${link.id}`}
						aria-current={current === link.id ? 'location' : undefined}
						class={`inline-flex min-h-11 items-center rounded-sm px-2 text-sm no-underline transition-colors xl:min-h-9 xl:w-full ${
							current === link.id
								? 'bg-surface font-semibold text-ink'
								: 'text-ink-muted hover:bg-surface hover:text-ink'
						}`}
					>
						{link.label}
					</a>
				</li>
			{/each}
		</ul>
	</nav>
{/if}
