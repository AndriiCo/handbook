<script lang="ts">
	import type { ComponentProps, Snippet } from 'svelte';
	import CheckIcon from '@lucide/svelte/icons/check';
	import CircleQuestionMarkIcon from '@lucide/svelte/icons/circle-question-mark';
	import ValueRow from './ValueRow.svelte';
	import InlineValue from './InlineValue.svelte';
	import ConflictBox from './ConflictBox.svelte';
	import LabelCard from './LabelCard.svelte';
	import OfficeCards from './OfficeCards.svelte';
	import { DOC_IDS } from './contract.js';
	import { OFFICE_SLOTS, SLOT, bySlot, notConfirmed, usable } from './items.js';
	import { carrierChips, isPoBox, isStreetOnly, telHref, type Chip, type EvidenceNote } from './postal.js';
	import type { SendRepeats } from './send.js';
	import type { Office } from '../plan.js';
	import type { Item, Section } from './contract.js';

	// "Send or bring your documents" (plan §3 step 5, D3): the section
	// body, three h3 branches always shown, on every page, in this order
	// (a branch with nothing on file says "Not confirmed yet — ask the
	// office"):
	//
	// - By mail (USPS), #send-mail: the mailing address with Copy, or the
	//   ConflictBox while the office's pages disagree; carrier chips from
	//   cited statements only ("✓ USPS" when one limits mail to USPS,
	//   every other carrier "not stated", never ✗); what to enclose and
	//   how it comes back, as printed; the free address label while the
	//   address is undisputed. On a US page this branch never shows a
	//   street: a mailing value that names a street and no PO box reads
	//   "Not confirmed yet" here and is listed under "Also on the
	//   office's pages", without Copy.
	// - By courier (FedEx, UPS), #send-courier: the courier street with
	//   Copy (never a PO box: a courier value that is one is listed under
	//   "Also on the office's pages", without Copy), the courier rule as
	//   printed, the recipient phone line, and the prepaid-label offer.
	// - In person, #visit: OfficeCards and the next closures.
	//
	// Then any other row of the section and its `also` rows.
	//
	// Rows repeated from other steps (what to enclose, how it comes back,
	// the office phone and its menu option) render as the page's copies
	// of them (`repeats`, doc/send.js), each with its own marker id and
	// the same [n]; without copies they render unmarked, so each marker id
	// appears once on the page. A row of this step renders itself, marked.
	// Label offers render only on profiled pages (not thin). In print the
	// mail and courier branches and the conflict box each keep to one
	// page (`data-print-keep`); label offers never print.
	//
	// Props:
	// - section: the send Section (`present()`).
	// - items: every Item on the page (fees, form, after and contact rows
	//   this block repeats or reads); optional.
	// - offices: `page.offices` groups; closures: `page.closures`.
	// - officeSet: accepted OfficeSetV1 offices (plan.ts Office[]), or
	//   null. The slice spec calls this list `offices`: OfficeCards sorts
	//   both lists by shape, so either may arrive under either name.
	// - options: the `/p2/orders/options` body, or null.
	// - evidence: `/plan` `evidence[]`, or null.
	// - slug: the authority slug.
	// - us: a US office (carrier names in the headings; the USPS street
	//   rule).
	// - thin, empty: the page states (contract Page).
	// - noticeActive: an active service notice (no live office status).
	// - now, country: for office status and map searches.
	// - issuer: who the label service is not ("the Secretary of State",
	//   else "the issuing office").
	// - repeats: the page's copies of the rows this step repeats
	//   (`sendRepeats(page.sections)`), numbered with the page; optional.
	// - marker: snippet `marker(item)` from the citations layer.
	// - inline: snippet `inline(item)`, markers inside a sentence.
	type Group = { instance: string | null; name?: string | null; items: Item[] };

	let {
		section,
		items = [],
		offices = [],
		closures = [],
		officeSet = null,
		options = null,
		evidence = null,
		slug,
		us = false,
		thin = false,
		empty = false,
		noticeActive = false,
		now = new Date(),
		country = null,
		issuer = 'the issuing office',
		repeats = null,
		marker,
		inline
	}: {
		section: Section;
		items?: Item[];
		offices?: Group[] | Office[] | null;
		closures?: Item[];
		officeSet?: Office[] | Group[] | null;
		options?: ComponentProps<typeof LabelCard>['options'];
		evidence?: EvidenceNote[] | null;
		slug: string;
		us?: boolean;
		thin?: boolean;
		empty?: boolean;
		noticeActive?: boolean;
		now?: Date;
		country?: string | null;
		issuer?: string;
		repeats?: SendRepeats | null;
		marker?: Snippet<[Item]>;
		inline?: Snippet<[Item]>;
	} = $props();

	const own = $derived(section.items);
	const all = $derived(items.length > 0 ? items : own);
	const find = (slot: string) => bySlot(own, slot) ?? bySlot(all, slot);
	// A repeated row as shown here: the page's copy (marked), the step's
	// own row (marked), or another step's row unmarked.
	const shownAs = (orig: Item | null, copy: Item | null | undefined) =>
		copy ? { item: copy, marked: true } : orig ? { item: orig, marked: own.includes(orig) } : null;

	const usps = $derived(find(SLOT.usps));
	const courierRaw = $derived(find(SLOT.courier));
	// D3: the courier branch never shows a PO box.
	const courier = $derived(courierRaw && !isPoBox(courierRaw.display) ? courierRaw : null);
	const courierBox = $derived(courierRaw && !courier ? courierRaw : null);
	const courierRule = $derived(find(SLOT.courierRule));
	const enclose = $derived(bySlot(all, SLOT.enclose));
	const returns = $derived(bySlot(all, SLOT.return));
	const encloseRow = $derived(shownAs(enclose, repeats?.enclose));
	const returnsRow = $derived(shownAs(returns, repeats?.returns));
	const phone = $derived(bySlot(all, SLOT.phone));
	const phoneOption = $derived(bySlot(all, SLOT.phoneOption));
	const courierPhone = $derived(shownAs(phone, repeats?.courierPhone));
	const fee = $derived(bySlot(all, SLOT.perDocument));
	const feeOk = $derived(fee?.state === 'ok');

	const disputed = $derived(
		!!usps && (usps.state === 'conflict' || usps.state === 'worded') && (usps.alternatives?.length ?? 0) > 0
	);
	// D3: on a US page the USPS branch never shows a street.
	const streetOnly = (i: Item | null | undefined) => !!i && us && isStreetOnly(i.display);
	const uspsStreet = $derived(usps && !disputed && streetOnly(usps) ? usps : null);
	const uspsShown = $derived(usps && !uspsStreet ? usps : null);
	const uspsUndisputed = $derived(uspsShown && !disputed && usable(uspsShown) ? uspsShown : null);

	const closureItems = $derived(closures && closures.length > 0 ? closures : own.filter((i) => i.slot === SLOT.closure));
	const officeRows = $derived(own.filter((i) => OFFICE_SLOTS.includes(i.slot ?? '')));

	const handled = $derived(
		new Set(
			[usps, courierRaw, courierRule, enclose, returns, ...officeRows, ...closureItems]
				.filter((i): i is Item => !!i)
				.map((i) => i.id)
		)
	);
	const rest = $derived(own.filter((i) => !handled.has(i.id) && i.slot !== SLOT.closure));
	const mailRouted = $derived(rest.filter((i) => i.route === 'mail'));
	const mailExtra = $derived(mailRouted.filter((i) => !streetOnly(i)));
	const courierExtra = $derived(rest.filter((i) => i.route === 'courier'));
	const visitExtra = $derived(rest.filter((i) => i.route === 'counter'));
	// Values a branch may not show stay on the page here, without Copy.
	const moved = $derived(
		[uspsStreet, courierBox, ...mailRouted.filter((i) => streetOnly(i))]
			.filter((i): i is Item => !!i)
			.map((i) => ({ ...i, copy: false }))
	);
	const alsoRows = $derived([...moved, ...rest.filter((i) => !i.route), ...section.also]);

	const chips = $derived(
		carrierChips({ mail: [uspsUndisputed, ...mailExtra], courier: [courier, courierRule, ...courierExtra] })
	);

	const sameDay = $derived(!!bySlot(all, SLOT.sameDay) || !!bySlot(all, SLOT.sameDayOnly) || !!bySlot(all, SLOT.sameDayTime));
	const headings = $derived({
		mail: us ? 'By mail (USPS)' : 'By mail',
		courier: us ? 'By courier (FedEx, UPS)' : 'By courier',
		visit: sameDay ? 'In person, same day' : 'In person'
	});
</script>

{#snippet chipRow(list: Chip[])}
	<ul class="m-0 mt-3 flex list-none flex-wrap gap-2 p-0" aria-label="Carriers the office's pages name">
		{#each list as chip (chip.key)}
			<li class="m-0 p-0">
				{#if chip.stated}
					<span
						class="inline-flex min-h-7 items-center gap-1.5 rounded-full border border-verified bg-verified-bg px-3 text-xs font-semibold text-verified"
						data-fact-ids={chip.factIds.join(' ')}
					>
						<CheckIcon class="size-3.5 flex-none" aria-hidden="true" />{chip.label}
					</span>
				{:else}
					<span
						class="inline-flex min-h-7 items-center gap-1.5 rounded-full border border-line bg-surface px-3 text-xs font-medium text-ink-muted"
						data-state="missing"
					>
						<CircleQuestionMarkIcon class="size-3.5 flex-none" aria-hidden="true" />{chip.label}: not stated
					</span>
				{/if}
			</li>
		{/each}
	</ul>
{/snippet}

<div class="flex flex-col gap-10">
	<div id={DOC_IDS.sendMail} class="flex flex-col gap-4" data-route="mail" data-print-keep>
		<h3 class="text-h3 m-0 font-semibold text-ink">{headings.mail}</h3>
		{#if usps && disputed}
			<ConflictBox
				item={usps}
				{evidence}
				phone={repeats?.conflictPhone ?? phone}
				phoneOption={repeats?.conflictOption ?? phoneOption}
				markRepeats={!!repeats?.conflictPhone}
				{courier}
				{marker}
				{inline}
			/>
		{:else if uspsShown}
			<dl class="m-0 border-t border-line">
				<ValueRow item={uspsShown} {marker} copyName="the mailing address" />
			</dl>
		{:else}
			<dl class="m-0 border-t border-line">
				<ValueRow item={notConfirmed('send-mail-missing', 'Mailing address', SLOT.usps)} />
			</dl>
		{/if}
		{#if chips.mail.length > 0}{@render chipRow(chips.mail)}{/if}
		{#if enclose || returns || mailExtra.length > 0}
			<dl class="m-0 border-t border-line">
				{#if encloseRow}<ValueRow item={encloseRow.item} marker={encloseRow.marked ? marker : undefined} />{/if}
				{#if returnsRow}<ValueRow item={returnsRow.item} marker={returnsRow.marked ? marker : undefined} />{/if}
				{#each mailExtra as item (item.id)}<ValueRow {item} {marker} />{/each}
			</dl>
		{/if}
		{#if !disputed && !thin}
			<LabelCard variant="mail" {options} {slug} usps={uspsUndisputed} {feeOk} {empty} {issuer} />
		{/if}
	</div>

	<div id={DOC_IDS.sendCourier} class="flex flex-col gap-4" data-route="courier" data-print-keep>
		<h3 class="text-h3 m-0 font-semibold text-ink">{headings.courier}</h3>
		<dl class="m-0 border-t border-line">
			{#if courier}
				<ValueRow item={courier} {marker} copyName="the courier address" />
			{:else}
				<ValueRow item={notConfirmed('send-courier-missing', 'Courier address', SLOT.courier)} />
			{/if}
			{#if courierRule}<ValueRow item={courierRule} {marker} />{/if}
			{#each courierExtra as item (item.id)}<ValueRow {item} {marker} />{/each}
		</dl>
		{#if chips.courier.length > 0}{@render chipRow(chips.courier)}{/if}
		{#if courier && usable(courier) && courierPhone && usable(courierPhone.item)}
			<p class="m-0 text-ink">
				Courier labels ask for a recipient phone; the office's published number is <InlineValue
					item={courierPhone.item}
					href={telHref(courierPhone.item.display)}
					marker={courierPhone.marked ? inline : undefined}
				/>.
			</p>
		{/if}
		{#if !thin}
			<LabelCard variant="courier" {options} {slug} {courier} {feeOk} {empty} {issuer} />
		{/if}
	</div>

	<div id={DOC_IDS.visit} class="flex flex-col gap-4" data-route="counter">
		<h3 class="text-h3 m-0 font-semibold text-ink">{headings.visit}</h3>
		<OfficeCards
			groups={offices}
			items={own}
			closures={closureItems}
			{officeSet}
			{noticeActive}
			{now}
			{country}
			{marker}
		/>
		{#if visitExtra.length > 0}
			<dl class="m-0 border-t border-line">
				{#each visitExtra as item (item.id)}<ValueRow {item} {marker} />{/each}
			</dl>
		{/if}
	</div>

	{#if alsoRows.length > 0}
		<div class="flex flex-col gap-4">
			<h3 class="text-h3 m-0 font-semibold text-ink">Also on the office's pages</h3>
			<dl class="m-0 border-t border-line">
				{#each alsoRows as item (item.id)}<ValueRow {item} {marker} />{/each}
			</dl>
		</div>
	{/if}
</div>
