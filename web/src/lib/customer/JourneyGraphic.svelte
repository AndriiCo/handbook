<script lang="ts">
	import type { LegRole, OrderKind } from './orders';
	import { site } from '../locale.svelte.js';
	import { ORDER_COPY } from './order-copy.js';

	// The order's journey, drawn from its own data: which legs the chosen
	// option has (outbound, return, kit to you) and, once quoted, the
	// carrier on each leg. Nothing here is a fact about the authority.
	let {
		authority = null,
		kind = null,
		carriers = {}
	}: {
		authority?: string | null;
		kind?: OrderKind | null;
		carriers?: Partial<Record<LegRole, string>>;
	} = $props();

	const legs = $derived.by((): LegRole[] => {
		if (!kind) return [];
		if (kind === 'address_label') return ['outbound'];
		if (kind === 'mail_kit') return ['kit_to_customer', 'outbound', 'return'];
		return ['outbound', 'return'];
	});
	const c = $derived(ORDER_COPY[site.locale]);
	const has = (r: LegRole) => legs.includes(r);
	const arcStyle = (r: LegRole, on: string) => 'stroke: ' + (has(r) ? on : 'var(--border)') + ';';
	const label = (r: LegRole) => (carriers[r] ? `${c.roles[r]} · ${carriers[r]}` : c.roles[r]);
	const clip = (s: string, n: number) => (s.length > n ? s.slice(0, n - 1) + '…' : s);
	// "Country — Office" names split onto two lines under the node.
	const nameLines = $derived.by(() => {
		if (!authority) return [c.chooseDestination];
		const parts = authority.split(/\s+[—–-]\s+/);
		return parts.length > 1
			? [clip(parts[0], 30), clip(parts.slice(1).join(' — '), 30)]
			: [clip(authority, 30)];
	});
	const summary = $derived(
		authority
			? legs.length
				? c.journeyLegs(legs.map((l) => label(l)).join(', '))
				: c.journeyBetween(authority)
			: c.journeyChoose
	);
</script>

<svg viewBox="0 0 460 180" class="w-full" role="img" aria-label={summary} data-graphic="journey">
	<!-- You -->
	<g transform="translate(70,92)">
		<circle r="30" style="fill: var(--surface); stroke: var(--border);" stroke-width="1.5" />
		<path
			d="M-12,4 L0,-8 L12,4 V14 H-12 Z"
			style="fill: none; stroke: var(--accent);"
			stroke-width="2"
			stroke-linejoin="round"
		/>
		<text y="50" text-anchor="middle" font-size="12" style="fill: var(--text-muted); font-family: var(--font-sans);">{c.you}</text>
	</g>
	<!-- Authority -->
	<g transform="translate(390,92)">
		{#if authority}
			<circle r="30" style="fill: var(--surface-2); stroke: var(--accent);" stroke-width="1.5" />
			<path
				d="M-14,12 H14 M-11,8 V-2 M-4,8 V-2 M4,8 V-2 M11,8 V-2 M-15,-5 L0,-14 L15,-5 Z"
				style="fill: none; stroke: var(--accent);"
				stroke-width="2"
				stroke-linejoin="round"
			/>
		{:else}
			<circle r="30" style="fill: none; stroke: var(--border);" stroke-width="1.5" stroke-dasharray="5 4" />
		{/if}
		<text x="40" y="50" text-anchor="end" font-size="12" style="fill: var(--text-muted); font-family: var(--font-sans);">
			{#each nameLines as line, i (i)}
				<tspan x="40" dy={i === 0 ? 0 : 15}>{line}</tspan>
			{/each}
		</text>
	</g>

	<!-- Outbound: you → authority -->
	<path
		d="M104,78 Q230,18 356,78"
		fill="none"
		style={arcStyle('outbound', 'var(--accent)')}
		stroke-width={has('outbound') ? 2 : 1.5}
		stroke-dasharray={has('outbound') ? undefined : '4 4'}
	/>
	{#if has('outbound')}
		<path d="M348,70 L357,79 L345,82" fill="none" style="stroke: var(--accent);" stroke-width="2" />
		<text x="230" y="40" text-anchor="middle" font-size="12" style="fill: var(--text); font-family: var(--font-sans);">
			{label('outbound')}
		</text>
	{/if}

	<!-- Return: authority → you -->
	<path
		d="M356,108 Q230,160 104,108"
		fill="none"
		style={arcStyle('return', 'var(--verified)')}
		stroke-width={has('return') ? 2 : 1.5}
		stroke-dasharray={has('return') ? undefined : '4 4'}
	/>
	{#if has('return')}
		<path d="M112,100 L103,109 L115,113" fill="none" style="stroke: var(--verified);" stroke-width="2" />
		<text x="190" y="152" text-anchor="middle" font-size="12" style="fill: var(--text); font-family: var(--font-sans);">
			{label('return')}
		</text>
	{/if}

	<!-- Kit to you -->
	{#if has('kit_to_customer')}
		<g transform="translate(70,18)">
			<rect x="-14" y="-10" width="28" height="20" rx="3" style="fill: var(--changed-bg); stroke: var(--changed);" stroke-width="1.5" />
			<path d="M-14,-3 H14 M0,-10 V10" style="stroke: var(--changed);" stroke-width="1.5" />
			<path d="M0,14 V34 M-5,29 L0,35 L5,29" fill="none" style="stroke: var(--changed);" stroke-width="1.5" />
			<text x="22" y="4" font-size="12" style="fill: var(--text); font-family: var(--font-sans);">{label('kit_to_customer')}</text>
		</g>
	{/if}
</svg>
