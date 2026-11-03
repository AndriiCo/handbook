<script lang="ts">
	import world from '../../geo/world.json';
	import { membersFromFacts } from './HagueMap.members.js';

	// Hague member map: precomputed country paths (scripts/geo.mjs, no d3
	// in the browser). Runtime-only: a country is tinted only when an
	// accepted HCCH fact names it as a party (`facts`), and a pin renders
	// only for an authority in the live index (`pins`, labelled with the
	// API's own region/country). No accepted facts → an untinted map
	// with an honest caption.
	let {
		facts = [],
		pins = [],
		class: klass = ''
	}: {
		facts?: unknown[];
		pins?: { slug: string; label: string }[];
		class?: string;
	} = $props();

	const tinted = $derived(new Set(membersFromFacts(facts)));
	const labels = $derived(new Map(pins.map((p) => [p.slug, p.label])));
	const shownPins = $derived(world.pins.filter((p) => labels.has(p.slug)));
	const shownArcs = $derived(world.arcs.filter((a) => labels.has(a.slug)));
	const caption = $derived(
		tinted.size > 0
			? `${tinted.size} ${tinted.size === 1 ? 'party' : 'parties'} named in staff-approved HCCH facts`
			: 'Member states appear here once staff approve HCCH status facts'
	);
</script>

<figure class={`m-0 ${klass}`}>
	<svg
		viewBox={`0 0 ${world.width} ${world.height}`}
		class="w-full"
		role="img"
		aria-label={`World map: ${caption}`}
		data-graphic="map"
	>
		<g style="fill: var(--surface-2); stroke: var(--bg);" stroke-width="0.5">
			{#each world.countries as c, i (i)}
				<path d={c.d} style={tinted.has(c.id) ? 'fill: var(--color-brand-300);' : undefined} />
			{/each}
		</g>
		<g style="fill: none; stroke: var(--color-brand-500);" stroke-width="1" opacity="0.55">
			{#each shownArcs as a (a.slug)}
				<path d={a.d} stroke-dasharray="3 3" />
			{/each}
		</g>
		{#each shownPins as p (p.slug)}
			<g>
				<circle cx={p.x} cy={p.y} r="4" style="fill: var(--color-brand-700);" />
				<circle
					cx={p.x}
					cy={p.y}
					r="7.5"
					style="fill: none; stroke: var(--color-brand-700);"
					stroke-width="1"
					opacity="0.4"
				/>
				<text
					x={p.x}
					y={p.y - 12}
					text-anchor="middle"
					font-size="13"
					style="fill: var(--color-ink); font-family: var(--font-sans);"
				>
					{labels.get(p.slug)}
				</text>
			</g>
		{/each}
	</svg>
	<figcaption class="mt-2 text-center font-mono text-xs text-ink-muted">{caption}</figcaption>
</figure>
