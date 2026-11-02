<script lang="ts">
	import { CHROME } from './translate.js';

	interface Props {
		lang: 'es' | 'fr';
		translated: string;
		original: string;
		model?: string;
	}

	let { lang, translated, original, model = '' }: Props = $props();
	let showOriginal = $state(false);
	const chrome = $derived(CHROME[lang] ?? CHROME.en);
</script>

<div lang={showOriginal ? 'en' : lang}>
	<p class="text-body-lg m-0 break-words text-ink">{showOriginal ? original : translated}</p>
	<p class="mt-2 mb-0 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-ink-muted">
		<span
			class="inline-block rounded border border-line bg-surface-2 px-2 py-0.5 text-xs font-semibold"
		>
			{chrome.machineNote}
		</span>
		<button
			type="button"
			class="inline-flex min-h-11 cursor-pointer items-center font-medium text-accent-strong underline decoration-dotted underline-offset-4"
			onclick={() => (showOriginal = !showOriginal)}
		>
			{showOriginal ? chrome.showTranslation : chrome.seeOriginal}
		</button>
		{#if model}
			<span class="font-mono text-xs">{chrome.translatedFrom} · {chrome.modelBy} {model}</span>
		{/if}
	</p>
</div>
