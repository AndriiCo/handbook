<script lang="ts">
	import type { Snippet } from 'svelte';
	import MinusIcon from '@lucide/svelte/icons/minus';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import CheckIcon from '@lucide/svelte/icons/check';
	import ValueRow from './ValueRow.svelte';
	import InlineValue from './InlineValue.svelte';
	import {
		ASK_ID,
		ESTIMATE_ID,
		FEE_SLOTS,
		MAX_DOCS,
		SPEEDS,
		askSentence,
		asksOffice,
		estimate as estimateOf,
		estimateItem,
		feeModel,
		type Speed
	} from '../fees.js';
	import { SLOT, bySlot, localItem, usable } from './items.js';
	import { telHref } from './postal.js';
	import type { Item, Section } from './contract.js';

	// "Pay the fees" (plan §3 step 4, D2): the section body. The section
	// itself (the one region whose name contains "fees") belongs to the
	// page; this block renders no region or section of its own.
	//
	// Order: the fee table (per document, expedite, same day, refunds, as
	// printed), the calculator, then how to pay (payee with Copy, counter
	// methods as printed, scoped by the office's own words), then every
	// other row of the section and its `also` rows.
	//
	// The calculator shows on profiled pages (not thin) whenever the
	// per-document fee has a row. When that fee is cited and undisputed:
	// a 1–20 document-count stepper (44 px buttons around a numeric text
	// field labelled "Number of documents"; one typed count is one
	// input), a speed choice offering only speeds whose fee has a real,
	// cited value, and one computed row: "Total to enclose: $145.00 = 3 ×
	// $15.00 + $100.00 expedite fee (1–10 documents)", or "Total at the
	// counter" for same day. Every number comes from `fees.js` over cited
	// values; the row is a computed Item (data-state="computed",
	// data-inputs) whose cites are its inputs' cites and which carries
	// the worst staleness of its inputs. When the tiers don't cover the
	// count, the count is above the stepper's range, or an input is
	// disputed, it says why and asks the customer to ask the office, with
	// the office's phone when known; a count still to enter only asks for
	// the count. When the per-document fee itself is disputed or has no
	// source on file, the calculator is that one "ask the office" row.
	// In print only the result row shows (`data-print-calc`).
	//
	// The computed Item (id ESTIMATE_ID) is owned here: the presenter
	// builds none (one in `section.items` is ignored), and `estimate`
	// hands it to the page, which numbers it after every other item
	// (`numberSources([...orderedItems(page), estimate])`) so a changing
	// count or speed never renumbers the office's sources. The ask row
	// (id ASK_ID) has no cites and is never numbered.
	//
	// Props:
	// - section: the fees Section (`present()`), items and `also`.
	// - items: every Item on the page (for the office phone,
	//   `contacts.phone`); optional.
	// - thin: a thin page (no calculator).
	// - marker: snippet `marker(item)` from the citations layer.
	// - docs, speed: bindable calculator state (default 1, 'standard'),
	//   so the page can keep them in the URL.
	// - estimate: bindable, written here only: the computed Item while
	//   the calculator answers, else null.
	let {
		section,
		items = [],
		thin = false,
		marker,
		docs = $bindable(1),
		speed = $bindable('standard'),
		estimate = $bindable(null)
	}: {
		section: Section;
		items?: Item[];
		thin?: boolean;
		marker?: Snippet<[Item]>;
		docs?: number | null;
		speed?: Speed;
		estimate?: Item | null;
	} = $props();

	const uid = $props.id();

	const SPEED_LABEL: Record<Speed, string> = {
		standard: 'Standard',
		expedite: 'Expedited',
		same_day: 'Same day, in person'
	};

	const own = $derived(section.items.filter((i) => i.id !== ESTIMATE_ID && i.id !== ASK_ID));
	const feeRows = $derived(own.filter((i) => (i.slot ?? '').startsWith('fees.')));
	const payRows = $derived(own.filter((i) => (i.slot ?? '').startsWith('payment.')));
	const otherRows = $derived([
		...own.filter((i) => !(i.slot ?? '').startsWith('fees.') && !(i.slot ?? '').startsWith('payment.')),
		...section.also
	]);

	const model = $derived(feeModel(own));
	// A per-document fee row the page shows (any value but an honest gap
	// or a stated absence): the calculator answers from it, or says why
	// it can't.
	const perRow = $derived(
		feeRows.find((i) => i.slot === FEE_SLOTS.perDocument && i.state !== 'missing' && i.state !== 'absent') ?? null
	);
	const showCalc = $derived(!thin && !!perRow);
	const ready = $derived(typeof model.perDocument?.cents === 'number' && model.speeds.standard !== null);
	const offered = $derived(SPEEDS.filter((s) => model.speeds[s] !== null));
	const chosen = $derived<Speed>(offered.includes(speed) ? speed : 'standard');
	const n = $derived(typeof docs === 'number' && Number.isFinite(docs) ? docs : 0);
	const result = $derived(estimateOf(model, n, chosen));
	const total = $derived(showCalc && ready ? estimateItem(model, n, chosen) : null);
	const askItem = $derived(
		showCalc && result.ask
			? localItem({ id: ASK_ID, label: 'Your total', display: askSentence(result.reason) })
			: null
	);
	const askPhone = $derived(showCalc && result.ask && asksOffice(result.reason));
	const phone = $derived(bySlot(items, SLOT.phone) ?? bySlot(own, SLOT.phone));
	const phoneHref = $derived(phone ? telHref(phone.display) : null);

	$effect(() => {
		if (showCalc && ready && !offered.includes(speed)) speed = 'standard';
	});

	$effect(() => {
		estimate = total;
	});

	// The stepper's ends are aria-disabled, not disabled, so a focused
	// button keeps focus at 1 and at 20; a press there does nothing.
	function stepClass(off: boolean): string {
		return `inline-flex size-11 items-center justify-center rounded-full text-ink transition-colors ${off ? 'cursor-not-allowed opacity-40' : 'cursor-pointer hover:bg-surface'}`;
	}

	function step(by: number) {
		if ((by < 0 && n <= 1) || (by > 0 && n >= MAX_DOCS)) return;
		docs = Math.min(MAX_DOCS, Math.max(1, (n >= 1 ? n : 0) + by));
	}

	// A typed count (GOV.UK: a text field with a numeric keypad, not a
	// number input). Anything but whole digits clears it; counts above
	// the stepper's range still answer, with "ask the office".
	function count(text: string) {
		const t = text.trim();
		docs = /^\d{1,4}$/.test(t) ? Number(t) : null;
	}
</script>

{#snippet officePhone()}
	{#if askPhone && phone && usable(phone)}
		<p class="m-0 mt-1 text-ink">Call the office: <InlineValue item={phone} href={phoneHref} /></p>
	{/if}
{/snippet}

<div class="flex flex-col gap-8">
	{#if feeRows.length > 0}
		<dl class="m-0 border-t border-line" data-print-keep>
			{#each feeRows as item (item.id)}
				<ValueRow {item} {marker} />
			{/each}
		</dl>
	{/if}

	{#if showCalc}
		<div
			class="rounded-md border border-line bg-surface p-4 sm:p-6 print:border-0 print:bg-transparent print:p-0"
			role="group"
			aria-labelledby={`${uid}-calc`}
			data-calculator
			data-print-calc
		>
			<h3 id={`${uid}-calc`} class="text-h3 m-0 font-semibold text-ink print:hidden">Work out your total</h3>
			{#if ready}
				<div class="mt-4 flex flex-wrap items-end gap-x-8 gap-y-5 print:hidden">
					<div class="flex flex-col gap-1">
						<label for={`${uid}-docs`} class="text-sm font-semibold text-ink-muted">Number of documents</label>
						<div class="inline-flex w-fit items-center rounded-full border border-line bg-bg">
							<button
								type="button"
								class={stepClass(n <= 1)}
								aria-label="One document fewer"
								aria-controls={`${uid}-docs`}
								aria-disabled={n <= 1}
								onclick={() => step(-1)}
							>
								<MinusIcon class="size-4" aria-hidden="true" />
							</button>
							<input
								id={`${uid}-docs`}
								type="text"
								inputmode="numeric"
								pattern="[0-9]*"
								autocomplete="off"
								value={docs ?? ''}
								oninput={(e) => count(e.currentTarget.value)}
								class="h-11 w-14 rounded-sm border-0 bg-transparent text-center font-mono text-body text-ink"
							/>
							<button
								type="button"
								class={stepClass(n >= MAX_DOCS)}
								aria-label="One document more"
								aria-controls={`${uid}-docs`}
								aria-disabled={n >= MAX_DOCS}
								onclick={() => step(1)}
							>
								<PlusIcon class="size-4" aria-hidden="true" />
							</button>
						</div>
					</div>
					{#if offered.length > 1}
						<fieldset class="m-0 min-w-0 border-0 p-0">
							<legend class="p-0 text-sm font-semibold text-ink-muted">Speed</legend>
							<div class="mt-1 flex flex-wrap gap-2">
								{#each offered as s (s)}
									<label
										class="inline-flex min-h-11 cursor-pointer items-center gap-2 rounded-full border border-line bg-bg px-4 text-sm font-medium text-ink transition-colors hover:border-focus has-[input:checked]:border-accent-fill has-[input:checked]:bg-accent-fill has-[input:checked]:text-accent-ink has-[input:focus-visible]:outline-3 has-[input:focus-visible]:outline-offset-2 has-[input:focus-visible]:outline-focus"
									>
										<input class="sr-only" type="radio" name={`${uid}-speed`} value={s} bind:group={speed} />
										{#if chosen === s}<CheckIcon class="size-4" aria-hidden="true" />{/if}
										{SPEED_LABEL[s]}
									</label>
								{/each}
							</div>
						</fieldset>
					{/if}
				</div>
			{/if}
			<dl class="m-0 mt-5 border-t border-line print:mt-0" aria-live="polite" data-print-line>
				{#if total}
					<ValueRow item={total} {marker} emphasis />
				{:else if askItem}
					<ValueRow item={askItem} extra={officePhone} />
				{/if}
			</dl>
		</div>
	{/if}

	{#if payRows.length > 0}
		<dl class="m-0 border-t border-line">
			{#each payRows as item (item.id)}
				{#if item.slot === SLOT.payee}
					<ValueRow {item} {marker} copyName="the payee name" emphasis />
				{:else}
					<ValueRow {item} {marker} />
				{/if}
			{/each}
		</dl>
	{/if}

	{#if otherRows.length > 0}
		<dl class="m-0 border-t border-line">
			{#each otherRows as item (item.id)}
				<ValueRow {item} {marker} />
			{/each}
		</dl>
	{/if}
</div>
