<script lang="ts">
	import CopyIcon from '@lucide/svelte/icons/copy';
	import CheckIcon from '@lucide/svelte/icons/check';
	import { guide } from './lang.svelte.js';

	// Copy one value exactly as printed (a payee, an address). A polite
	// live region says "Copied: …". When the browser refuses the
	// clipboard (permission denied, insecure context), the value on the
	// page is selected instead so the customer can copy it with their own
	// keys, and the live region says so. Hidden in print.
	//
	// Props:
	// - text: the exact text to copy (the Item's display, verbatim).
	// - name: what is copied, for the button's accessible name
	//   ("Copy" + name, e.g. "the payee name"); optional.
	// - target: the value node to select when the clipboard is denied.
	let {
		text,
		name = '',
		target = null
	}: { text: string; name?: string; target?: HTMLElement | null } = $props();

	let said = $state('');
	let copied = $state(false);
	let timer: ReturnType<typeof setTimeout> | null = null;

	function settle(message: string, ok: boolean) {
		said = message;
		copied = ok;
		if (timer) clearTimeout(timer);
		timer = setTimeout(() => {
			said = '';
			copied = false;
			timer = null;
		}, 4000);
	}

	function selectTarget(): boolean {
		if (!target) return false;
		const selection = window.getSelection();
		if (!selection) return false;
		const range = document.createRange();
		range.selectNodeContents(target);
		selection.removeAllRanges();
		selection.addRange(range);
		return true;
	}

	async function copy() {
		try {
			if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
			await navigator.clipboard.writeText(text);
			settle(guide.t.copyButton.copied(text), true);
		} catch {
			const t = guide.t.copyButton;
			settle(selectTarget() ? t.selected(text) : t.selectYourself(text), false);
		}
	}

	$effect(() => () => {
		if (timer) clearTimeout(timer);
	});
</script>

<button
	type="button"
	class="inline-flex min-h-8 cursor-pointer items-center gap-1.5 rounded-sm border border-line bg-bg px-2.5 text-sm font-medium text-ink transition-colors hover:border-focus hover:bg-surface print:hidden"
	onclick={copy}
>
	{#if copied}
		<CheckIcon class="size-4 text-verified" aria-hidden="true" />
	{:else}
		<CopyIcon class="size-4 text-ink-muted" aria-hidden="true" />
	{/if}
	{guide.t.copyButton.label}{#if name}<span class="sr-only">&nbsp;{name}</span>{/if}
</button>
<span class="sr-only" role="status" aria-live="polite">{said}</span>
