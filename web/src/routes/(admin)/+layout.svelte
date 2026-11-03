<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import * as Sidebar from '#lib/components/ui/sidebar/index.js';
	import * as Breadcrumb from '#lib/components/ui/breadcrumb/index.js';
	import * as Command from '#lib/components/ui/command/index.js';
	import { Separator } from '#lib/components/ui/separator/index.js';
	import { Kbd } from '#lib/components/ui/kbd/index.js';
	import { subscribe } from '#lib/live.svelte';
	import LayoutDashboardIcon from '@lucide/svelte/icons/layout-dashboard';
	import ListChecksIcon from '@lucide/svelte/icons/list-checks';
	import ActivityIcon from '@lucide/svelte/icons/activity';
	import DatabaseIcon from '@lucide/svelte/icons/database';
	import TagIcon from '@lucide/svelte/icons/tag';
	import PackageIcon from '@lucide/svelte/icons/package';
	import PhoneIcon from '@lucide/svelte/icons/phone';
	import MailIcon from '@lucide/svelte/icons/mail';
	import SettingsIcon from '@lucide/svelte/icons/settings';
	import SearchIcon from '@lucide/svelte/icons/search';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import type { Component } from 'svelte';
	import type { LayoutProps } from './$types';

	// Staff console shell. Ships ONLY in the web-admin build
	// (admin.andrii.ai); the public build's routes dir has no (admin)
	// group at all. The root layout skips the public chrome here.
	let { children }: LayoutProps = $props();

	type NavItem = { label: string; href: string; icon: Component; keywords: string };
	type NavGroup = { label: string; items: NavItem[] };

	// Groups per the console plan (Overview, Content, Commerce, First
	// source, System). "First source" is staff asking the office itself
	// (calls, email) when the published pages leave a question open.
	const NAV: NavGroup[] = [
		{
			label: 'Overview',
			items: [
				{ label: 'Dashboard', href: resolve('/(admin)/dashboard'), icon: LayoutDashboardIcon, keywords: 'kpi stats meta latency tokens' }
			]
		},
		{
			label: 'Content',
			items: [
				// Review and the old Triage list are one screen now: "Riskiest
				// first" or "By authority" (bulk). /triage stays as a page.
				{ label: 'Review', href: resolve('/(admin)/review'), icon: ListChecksIcon, keywords: 'facts approve queue triage bulk by authority' },
				{ label: 'Trace', href: resolve('/(admin)/trace'), icon: ActivityIcon, keywords: 'draft model tokens latency' },
				{ label: 'Workers', href: resolve('/(admin)/workers'), icon: DatabaseIcon, keywords: 'fetch captures sweep' }
			]
		},
		{
			label: 'Commerce',
			items: [
				{ label: 'Orders', href: resolve('/(admin)/staff-orders'), icon: PackageIcon, keywords: 'orders fulfilment kit refund stripe' },
				{ label: 'Labels', href: resolve('/(admin)/labels'), icon: TagIcon, keywords: 'fedex usps postage' }
			]
		},
		{
			label: 'First source',
			items: [
				{ label: 'Voice calls', href: resolve('/(admin)/calls'), icon: PhoneIcon, keywords: 'phone telnyx open questions' },
				{ label: 'Email', href: resolve('/(admin)/email'), icon: MailIcon, keywords: 'email outreach office reply questions' }
			]
		},
		{
			label: 'System',
			items: [{ label: 'Config', href: resolve('/(admin)/config'), icon: SettingsIcon, keywords: 'keys settings' }]
		}
	];
	const ALL = NAV.flatMap((g) => g.items.map((i) => ({ ...i, group: g.label })));

	const BARE = ['/staff-login', '/enrol'];
	const bare = $derived(BARE.includes(page.url.pathname));
	const current = $derived(ALL.find((i) => page.url.pathname.startsWith(i.href)) ?? null);

	let env = $state<{ version: string; env: string } | null>(null);
	let live = $state(false);
	let paletteOpen = $state(false);

	const ENV_TONE: Record<string, string> = {
		production: 'bg-changed-bg text-changed',
		staging: 'bg-surface-2 text-accent-strong',
		local: 'bg-surface text-ink-muted',
		judge: 'bg-surface text-ink-muted'
	};

	function onKey(e: KeyboardEvent) {
		if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
			e.preventDefault();
			paletteOpen = !paletteOpen;
		}
	}

	async function go(href: string) {
		paletteOpen = false;
		await goto(href);
	}

	// Client-side session gate (onMount only — never runs during
	// prerender, so the static build stays green). /staff-login
	// and /enrol are exempt (redirect loop otherwise), and
	// /config is exempt while the setup window is open (no
	// session can exist yet — the page runs the wizard instead).
	onMount(() => {
		let stop: (() => void) | null = null;
		(async () => {
			const path = window.location.pathname;
			if (BARE.includes(path)) return;
			if (path === '/config') {
				try {
					const setup = await fetch('/apostille/api/v1/setup/status', {
						credentials: 'same-origin'
					});
					if (setup.ok && (await setup.json().catch(() => ({})))?.setup_required) return;
				} catch {
					// Fall through to the session check below.
				}
			}
			try {
				const res = await fetch('/apostille/api/v1/me', { credentials: 'same-origin' });
				if (res.status === 401 || res.status === 403) {
					window.location.href = resolve('/(admin)/staff-login');
					return;
				}
			} catch {
				// Offline or cert-less: pages surface their own errors;
				// never strand the user on a blank shell.
			}
			try {
				const res = await fetch('/apostille/api/v1/admin/version', {
					credentials: 'same-origin'
				});
				if (res.ok) env = await res.json();
			} catch {
				// The badge is informational; the shell works without it.
			}
			// Staff activity feed: the dot shows the console is live.
			stop = subscribe('ops', (msg) => {
				if (msg.type === 'subscribed') live = true;
			});
		})();
		return () => stop?.();
	});
</script>

<svelte:window onkeydown={onKey} />

{#if bare}
	{@render children()}
{:else}
	<Sidebar.Provider>
		<Sidebar.Root collapsible="icon">
			<Sidebar.Header>
				<div class="flex items-center gap-2 px-2 py-1">
					<ShieldCheckIcon class="size-5 flex-none text-accent-strong" aria-hidden="true" />
					<div class="min-w-0 group-data-[collapsible=icon]:hidden">
						<p class="m-0 truncate text-sm font-semibold">Apostille console</p>
						<p class="m-0 truncate text-xs text-ink-muted">Staff only</p>
					</div>
				</div>
			</Sidebar.Header>
			<Sidebar.Content>
				{#each NAV as group (group.label)}
					<Sidebar.Group>
						<Sidebar.GroupLabel>{group.label}</Sidebar.GroupLabel>
						<Sidebar.GroupContent>
							<Sidebar.Menu>
								{#each group.items as item (item.href)}
									<Sidebar.MenuItem>
										<Sidebar.MenuButton isActive={current?.href === item.href} tooltipContent={item.label}>
											{#snippet child({ props })}
												<a href={item.href} {...props} aria-current={current?.href === item.href ? 'page' : undefined}>
													<item.icon aria-hidden="true" />
													<span>{item.label}</span>
												</a>
											{/snippet}
										</Sidebar.MenuButton>
									</Sidebar.MenuItem>
								{/each}
							</Sidebar.Menu>
						</Sidebar.GroupContent>
					</Sidebar.Group>
				{/each}
			</Sidebar.Content>
			<Sidebar.Footer>
				<p class="m-0 px-2 text-xs text-ink-muted group-data-[collapsible=icon]:hidden" role="note">
					Staff only — served from admin.andrii.ai. Every action is audited.
				</p>
			</Sidebar.Footer>
			<Sidebar.Rail />
		</Sidebar.Root>
		<Sidebar.Inset>
			<header
				class="sticky top-0 z-30 flex h-14 items-center gap-2 border-b border-line bg-bg/85 px-4 backdrop-blur"
			>
				<Sidebar.Trigger class="-ml-1" />
				<Separator orientation="vertical" class="mr-1 h-4" />
				<Breadcrumb.Root>
					<Breadcrumb.List>
						<Breadcrumb.Item class="hidden md:block">Console</Breadcrumb.Item>
						{#if current}
							<Breadcrumb.Separator class="hidden md:block" />
							<Breadcrumb.Item class="hidden md:block">{current.group}</Breadcrumb.Item>
							<Breadcrumb.Separator class="hidden md:block" />
							<Breadcrumb.Item>
								<Breadcrumb.Page>{current.label}</Breadcrumb.Page>
							</Breadcrumb.Item>
						{/if}
					</Breadcrumb.List>
				</Breadcrumb.Root>
				<div class="ml-auto flex items-center gap-2">
					<button
						type="button"
						class="inline-flex min-h-9 items-center gap-2 rounded-md border border-line bg-bg px-3 text-sm text-ink-muted hover:text-ink"
						aria-label="Jump to a console screen"
						onclick={() => (paletteOpen = true)}
					>
						<SearchIcon class="size-4" aria-hidden="true" />
						<span class="hidden sm:inline">Jump to</span>
						<Kbd class="hidden sm:inline-flex">⌘K</Kbd>
					</button>
					{#if live}
						<span
							class="inline-flex items-center gap-1 text-xs text-ink-muted"
							title="Live: staff activity feed connected"
						>
							<span class="size-2 rounded-full bg-verified" aria-hidden="true"></span>
							<span class="sr-only md:not-sr-only">Live</span>
						</span>
					{/if}
					{#if env}
						<span
							class={`rounded-md px-2 py-1 font-mono text-xs ${ENV_TONE[env.env] ?? ENV_TONE.local}`}
							title={env.version}
						>
							{env.env}
						</span>
					{/if}
				</div>
			</header>
			<div class="min-w-0 flex-1 p-4 md:p-8">
				{@render children()}
			</div>
		</Sidebar.Inset>
	</Sidebar.Provider>

	<Command.Dialog bind:open={paletteOpen} title="Jump to" description="Go to a console screen">
		<Command.Input placeholder="Go to…" />
		<Command.List>
			<Command.Empty>No screen matches.</Command.Empty>
			{#each NAV as group (group.label)}
				<Command.Group heading={group.label}>
					{#each group.items as item (item.href)}
						<Command.Item value={`${item.label} ${item.keywords}`} onSelect={() => go(item.href)}>
							<item.icon aria-hidden="true" />
							{item.label}
						</Command.Item>
					{/each}
				</Command.Group>
			{/each}
		</Command.List>
	</Command.Dialog>
{/if}
