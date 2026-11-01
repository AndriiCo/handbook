<script lang="ts">
	// Guilloche: fine engraved-line pattern for heroes and panels.
	// 3–5 overlaid hypotrochoid curves, deterministic per seed so every
	// authority gets its own pattern. Decorative only (aria-hidden).
	let {
		seed = 'apostille',
		curves = 4,
		class: klass = ''
	}: {
		seed?: string;
		curves?: number;
		class?: string;
	} = $props();

	function hash(str: string): number {
		let h = 2166136261;
		for (let i = 0; i < str.length; i++) {
			h ^= str.charCodeAt(i);
			h = Math.imul(h, 16777619);
		}
		return h >>> 0;
	}

	function mulberry(seedNum: number) {
		let a = seedNum;
		return () => {
			a |= 0;
			a = (a + 0x6d2b79f5) | 0;
			let t = Math.imul(a ^ (a >>> 15), 1 | a);
			t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
			return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
		};
	}

	const paths = $derived.by(() => {
		const rnd = mulberry(hash(seed));
		const n = Math.min(5, Math.max(3, curves));
		const out: string[] = [];
		for (let c = 0; c < n; c++) {
			const R = 90 + rnd() * 40;
			const r = 12 + rnd() * 30;
			const d = 20 + rnd() * 60;
			const turns = 24 + Math.floor(rnd() * 24);
			const rot = rnd() * Math.PI * 2;
			let path = '';
			for (let i = 0; i <= 720; i++) {
				const t = (i / 720) * Math.PI * 2 * (turns / 24);
				const x =
					(R - r) * Math.cos(t) + d * Math.cos(((R - r) / r) * t) + 150;
				const y =
					(R - r) * Math.sin(t) - d * Math.sin(((R - r) / r) * t) + 150;
				const a = rot;
				const px = 150 + (x - 150) * Math.cos(a) - (y - 150) * Math.sin(a);
				const py = 150 + (x - 150) * Math.sin(a) + (y - 150) * Math.cos(a);
				path += `${i === 0 ? 'M' : 'L'}${px.toFixed(1)},${py.toFixed(1)}`;
			}
			out.push(path);
		}
		return out;
	});
</script>

<svg
	viewBox="0 0 300 300"
	class={klass}
	aria-hidden="true"
	focusable="false"
	role="presentation"
>
	{#each paths as d, i (i)}
		<path
			{d}
			fill="none"
			stroke="currentColor"
			stroke-width="0.5"
			opacity={0.2 + (i % 3) * 0.07}
		/>
	{/each}
</svg>
