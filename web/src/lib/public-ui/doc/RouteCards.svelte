<script lang="ts">
	import MailIcon from '@lucide/svelte/icons/mail';
	import TruckIcon from '@lucide/svelte/icons/truck';
	import BuildingIcon from '@lucide/svelte/icons/building-2';
	import ArrowDownIcon from '@lucide/svelte/icons/arrow-down';
	import { guide } from './lang.svelte.js';

	// The three ways to send (plan §3 row 1): anchor cards to the mail,
	// courier and counter branches of "Send or bring your documents".
	// Only the routes the office's pages fill are passed in; every branch
	// stays on the page below, in the same order, whichever card is used.
	//
	// Props:
	// - routes: `page.routes` ({ id, label, href }).
	let { routes = [] }: { routes?: { id: string; label: string; href: string }[] } = $props();

	const ICONS = { mail: MailIcon, courier: TruckIcon, visit: BuildingIcon } as const;
	const t = $derived(guide.t.routes);
	const HINTS: Record<string, string> = $derived(t.hints);
</script>

{#if routes.length > 0}
	<nav aria-label={t.label} class="mt-8">
		<ul class="m-0 grid list-none gap-3 p-0 sm:grid-cols-3">
			{#each routes as route (route.id)}
				{@const Icon = ICONS[route.id as keyof typeof ICONS] ?? MailIcon}
				<li class="m-0 p-0">
					<a
						href={route.href}
						class="group flex h-full min-h-11 flex-col gap-2 rounded-md border border-line bg-bg p-4 text-ink no-underline transition-colors hover:border-focus"
					>
						<span class="flex items-center gap-2 font-semibold">
							<Icon class="size-5 flex-none text-accent-strong" aria-hidden="true" />
							{route.label}
							<ArrowDownIcon
								class="ml-auto size-4 flex-none text-ink-muted transition-transform group-hover:translate-y-0.5"
								aria-hidden="true"
							/>
						</span>
						{#if HINTS[route.id]}
							<span class="text-sm text-ink-muted">{HINTS[route.id]}</span>
						{/if}
					</a>
				</li>
			{/each}
		</ul>
	</nav>
{/if}
