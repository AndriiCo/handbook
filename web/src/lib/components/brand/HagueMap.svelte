<script lang="ts">
	import world from '../../geo/world.json';

	// Hague member map: precomputed country paths (scripts/geo.mjs, no d3
	// in the browser). `members` is the set of ISO-3166 numeric ids to
	// tint; featured pins + Seattle arcs render labelled.
	let {
		members = [],
		class: klass = ''
	}: {
		members?: string[];
		class?: string;
	} = $props();

	const tinted = $derived(new Set(members));
</script>

<svg
	viewBox={`0 0 ${world.width} ${world.height}`}
	class={klass}
	role="img"
	aria-label="Map of Hague Convention member states covered by the handbook"
	data-graphic="map"
>
	<g style="fill: var(--surface-2); stroke: var(--bg);" stroke-width="0.5">
		{#each world.countries as c, i (i)}
			<path d={c.d} style={tinted.has(c.id) ? 'fill: var(--color-brand-300);' : undefined} />
		{/each}
	</g>
	<g style="fill: none; stroke: var(--color-brand-500);" stroke-width="1" opacity="0.55">
		{#each world.arcs as a (a.slug)}
			<path d={a.d} stroke-dasharray="3 3" />
		{/each}
	</g>
	{#each world.pins as p (p.slug)}
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
				{p.label}
			</text>
		</g>
	{/each}
</svg>
