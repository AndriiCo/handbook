<script lang="ts">
	// The 10 numbered fields of the HCCH model apostille certificate as a
	// clean SVG. `highlight` (1–10) tints the field the current step reads.
	// Never a real government seal or emblem — plain numbered rows.
	let { highlight = 0 }: { highlight?: number } = $props();

	const fields = [
		'Country',
		'Signed by',
		'Capacity',
		'Seal of',
		'At',
		'Date',
		'By',
		'No.',
		'Seal',
		'Signature'
	];
</script>

<svg
	viewBox="0 0 240 300"
	class="h-auto w-full"
	role="img"
	aria-label="Model apostille certificate with ten numbered fields"
	data-graphic="certificate"
>
	<rect
		x="8"
		y="8"
		width="224"
		height="284"
		rx="8"
		style="fill: var(--surface); stroke: var(--border);"
		stroke-width="1"
	/>
	<rect
		x="8"
		y="8"
		width="224"
		height="40"
		rx="8"
		style="fill: var(--surface-2);"
	/>
	<text
		x="120"
		y="33"
		text-anchor="middle"
		font-size="13"
		font-weight="600"
		style="fill: var(--color-ink); font-family: var(--font-sans);"
	>
		Apostille
	</text>
	{#each fields as label, i (label)}
		{@const n = i + 1}
		{@const on = highlight === n}
		<g opacity={highlight === 0 || on ? 1 : 0.45}>
			<circle
				cx="30"
				cy={62 + i * 23}
				r="8"
				style={on
					? 'fill: var(--color-brand-700);'
					: 'fill: none; stroke: var(--border);'}
				stroke-width="1"
			/>
			<text
				x="30"
				y={62 + i * 23 + 3.5}
				text-anchor="middle"
				font-size="9"
				style={on
					? 'fill: #ffffff;'
					: 'fill: var(--color-ink-muted); font-family: var(--font-mono);'}
			>
				{n}
			</text>
			<text
				x="46"
				y={62 + i * 23 + 3.5}
				font-size="10.5"
				style="fill: var(--color-ink); font-family: var(--font-sans);"
			>
				{label}
			</text>
			<line
				x1="46"
				y1={62 + i * 23 + 8}
				x2={120 + ((i * 37) % 80)}
				y2={62 + i * 23 + 8}
				style="stroke: var(--border);"
				stroke-width="1"
			/>
		</g>
	{/each}
</svg>
