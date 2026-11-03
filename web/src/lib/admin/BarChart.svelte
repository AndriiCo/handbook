<script lang="ts" module>
	export interface Bar {
		key: string | number;
		label: string;
		value: number;
		detail?: string;
	}
</script>

<script lang="ts">
	// Zero-dependency column chart (inline SVG, single series). Drawn at
	// the measured pixel width, so the 4 px rounded caps never stretch.
	// Hover a column for its tooltip; a visually hidden table carries
	// every value for screen readers (the SVG itself is one image).

	let {
		bars,
		label,
		unit = '',
		height = 160
	}: { bars: Bar[]; label: string; unit?: string; height?: number } = $props();

	let width = $state(0);
	let active = $state<number | null>(null);

	const PAD_TOP = 12;
	const BAR_MAX = 24;
	const GAP = 2;

	function niceMax(v: number): number {
		if (v <= 4) return 4;
		const pow = 10 ** Math.floor(Math.log10(v));
		for (const step of [1, 2, 2.5, 5, 10]) {
			if (step * pow >= v) return step * pow;
		}
		return 10 * pow;
	}

	const max = $derived(niceMax(Math.max(0, ...bars.map((b) => b.value))));
	const plotH = $derived(height - PAD_TOP);
	const slot = $derived(bars.length > 0 ? width / bars.length : 0);
	const barW = $derived(Math.max(2, Math.min(BAR_MAX, slot - GAP * 2)));
	// Show at most one x label per ~52 px (labels never truncate).
	const every = $derived(Math.max(1, Math.ceil((bars.length * 52) / Math.max(1, width))));

	function y(v: number): number {
		return PAD_TOP + plotH - (v / max) * plotH;
	}

	// Column with a 4 px rounded data-end, square at the baseline.
	function column(i: number, v: number): string {
		const x = i * slot + (slot - barW) / 2;
		const top = y(v);
		const h = height - top;
		const r = Math.min(4, barW / 2, h);
		const base = height;
		return `M${x},${base}V${top + r}Q${x},${top} ${x + r},${top}H${x + barW - r}Q${x + barW},${top} ${x + barW},${top + r}V${base}Z`;
	}

	const total = $derived(bars.reduce((sum, b) => sum + b.value, 0));
	const peak = $derived(bars.reduce((best, b, i) => (b.value > (bars[best]?.value ?? -1) ? i : best), 0));
</script>

<figure class="m-0">
	<div class="relative" style={`height:${height}px`} bind:clientWidth={width}>
		{#if width > 0}
			<svg
				{width}
				{height}
				viewBox={`0 0 ${width} ${height}`}
				class="block overflow-visible"
				role="img"
				aria-label={`${label}: ${total}${unit ? ` ${unit}` : ''} in total`}
			>
				<line x1="0" x2={width} y1={PAD_TOP} y2={PAD_TOP} class="stroke-line" stroke-width="1" />
				<line x1="0" x2={width} y1={y(max / 2)} y2={y(max / 2)} class="stroke-line" stroke-width="1" />
				<line x1="0" x2={width} y1={height - 0.5} y2={height - 0.5} class="stroke-line" stroke-width="1" />
				{#each bars as bar, i (bar.key)}
					{#if bar.value > 0}
						<path
							d={column(i, bar.value)}
							class={`fill-accent-strong transition-opacity duration-150 motion-reduce:transition-none ${active !== null && active !== i ? 'opacity-40' : ''}`}
						/>
					{/if}
					<rect
						x={i * slot}
						y="0"
						width={slot}
						{height}
						fill="transparent"
						role="presentation"
						onpointerenter={() => (active = i)}
						onpointerleave={() => (active = null)}
					/>
				{/each}
			</svg>
			<span
				class="pointer-events-none absolute top-0 left-0 -translate-y-1/2 bg-card pr-1 font-mono text-xs text-ink-muted"
				aria-hidden="true">{max}</span
			>
			{#if active !== null && bars[active]}
				{@const bar = bars[active]}
				<div
					class="pointer-events-none absolute z-10 -translate-x-1/2 -translate-y-full rounded-sm border border-line bg-popover px-2 py-1 text-xs whitespace-nowrap text-popover-foreground shadow-md"
					style={`left:${Math.min(Math.max(active * slot + slot / 2, 48), width - 48)}px;top:${y(bar.value) - 6}px`}
					aria-hidden="true"
				>
					<span class="font-semibold">{bar.value}{unit ? ` ${unit}` : ''}</span>
					<span class="text-muted-foreground"> · {bar.label}</span>
					{#if bar.detail}<span class="block text-muted-foreground">{bar.detail}</span>{/if}
				</div>
			{:else if bars[peak] && bars[peak].value > 0}
				<span
					class="pointer-events-none absolute -translate-x-1/2 -translate-y-full font-mono text-xs font-semibold text-ink"
					style={`left:${peak * slot + slot / 2}px;top:${y(bars[peak].value) - 2}px`}
					aria-hidden="true">{bars[peak].value}</span
				>
			{/if}
		{/if}
	</div>
	<div class="relative mt-2 h-4" aria-hidden="true">
		{#if width > 0}
			{#each bars as bar, i (bar.key)}
				{#if i % every === (bars.length - 1) % every}
					<span
						class="absolute top-0 -translate-x-1/2 text-xs whitespace-nowrap text-ink-muted"
						style={`left:${i * slot + slot / 2}px`}>{bar.label}</span
					>
				{/if}
			{/each}
		{/if}
	</div>
	<table class="sr-only">
		<caption>{label}</caption>
		<tbody>
			{#each bars as bar (bar.key)}
				<tr><th scope="row">{bar.label}</th><td>{bar.value}{unit ? ` ${unit}` : ''}{bar.detail ? `, ${bar.detail}` : ''}</td></tr>
			{/each}
		</tbody>
	</table>
</figure>
