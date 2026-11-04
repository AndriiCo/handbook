<script lang="ts">
	import { onMount } from 'svelte';
	import Heading from '#lib/components/brand/Heading.svelte';
	import Notice from '#lib/public-ui/Notice.svelte';
	import * as Card from '#lib/components/ui/card/index.js';
	import { Badge } from '#lib/components/ui/badge/index.js';
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import { Skeleton } from '#lib/components/ui/skeleton/index.js';
	import RequestErrorNotice from '#lib/admin/RequestErrorNotice.svelte';
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import UserPlusIcon from '@lucide/svelte/icons/user-plus';
	import CopyIcon from '@lucide/svelte/icons/copy';
	import {
		addPasskey,
		createInvite,
		fetchStaff,
		revokePasskey,
		revokeStaff,
		shortId,
		validCn,
		withdrawInvite,
		type StaffAccount,
		type StaffList
	} from '#lib/admin/staff-client.js';
	import { CANCELLED_COPY, confirmPasskey, cookieValue, loginErrorCopy } from '#lib/admin/staff-auth.js';
	import { ago, dateTime } from '#lib/admin/format.js';

	// Staff management (customer-auth.md §5.6): who can sign in to this
	// console and with which passkeys. Admins invite, withdraw invites
	// and revoke; every staffer adds or removes their own passkeys.
	// Each change needs a passkey check from the last 10 minutes: a
	// refusal confirms in place and runs the same action again.
	const API = '/apostille/api/v1';

	let list = $state<StaffList | null>(null);
	let loadError = $state<string | null>(null);
	let now = $state(Math.floor(Date.now() / 1000));

	// The one action in flight, and the last refusal with where it
	// belongs (`key` names the control it came from).
	let busyKey = $state<string | null>(null);
	let failure = $state<{ key: string; message: string; requestId: string | null; stepUp: boolean } | null>(null);
	let confirming = $state(false);
	let retry: (() => Promise<unknown>) | null = null;

	let armed = $state<string | null>(null);
	let inviteRole = $state<'staff' | 'admin'>('staff');
	let inviteCn = $state('');
	let inviteDays = $state(1);
	let invite = $state<{ url: string; cn: string; role: string; expires_at: number } | null>(null);
	let copied = $state(false);
	let passkeyLabel = $state('');
	let added = $state(false);

	const me = $derived(list?.accounts.find((a) => a.user_id === list?.me.user_id) ?? null);
	const isAdmin = $derived(list?.me.role === 'admin');
	const others = $derived(list?.accounts.filter((a) => a.user_id !== list?.me.user_id) ?? []);

	async function load() {
		try {
			list = await fetchStaff(API);
			now = Math.floor(Date.now() / 1000);
			loadError = null;
		} catch (err) {
			loadError = err instanceof Error ? err.message : 'Staff list failed.';
		}
	}

	onMount(load);

	async function act(key: string, run: () => Promise<unknown>): Promise<boolean> {
		busyKey = key;
		failure = null;
		try {
			await run();
			retry = null;
			await load();
			return true;
		} catch (err) {
			const info = err as { requestId?: string | null; stepUp?: boolean };
			failure = {
				key,
				message:
					err instanceof DOMException && err.name === 'NotAllowedError'
						? CANCELLED_COPY
						: err instanceof Error
							? err.message
							: 'Request failed.',
				requestId: info?.requestId ?? null,
				stepUp: info?.stepUp === true
			};
			retry = run;
			return false;
		} finally {
			busyKey = null;
		}
	}

	async function staffGet(path: string) {
		const res = await fetch(`${API}${path}`, { credentials: 'same-origin' });
		return { ok: res.ok, status: res.status, body: await res.json().catch(() => ({})) };
	}

	async function staffPost(path: string, body: unknown) {
		const res = await fetch(`${API}${path}`, {
			method: 'POST',
			headers: { 'content-type': 'application/json', 'x-csrf': cookieValue(document.cookie, 'hb_csrf') },
			credentials: 'same-origin',
			body: JSON.stringify(body)
		});
		return { ok: res.ok, status: res.status, body: await res.json().catch(() => ({})) };
	}

	// Stale step-up: one passkey check for this session's own account,
	// then the refused action again.
	async function confirmAndRetry() {
		if (!failure || !retry) return;
		const { key } = failure;
		const run = retry;
		confirming = true;
		try {
			const res = await confirmPasskey({ get: staffGet, post: staffPost, credentials: navigator.credentials });
			if (!res.ok) {
				failure = { key, message: loginErrorCopy(res.status, res.body?.error ?? ''), requestId: null, stepUp: false };
				return;
			}
			await act(key, run);
		} catch (err) {
			failure = {
				key,
				message: err instanceof DOMException && err.name === 'NotAllowedError' ? CANCELLED_COPY : 'Passkey check failed — try again.',
				requestId: null,
				stepUp: false
			};
		} finally {
			confirming = false;
		}
	}

	async function submitInvite(event: SubmitEvent) {
		event.preventDefault();
		invite = null;
		copied = false;
		let minted: { url: string; cn: string; role: string; expires_at: number } | null = null;
		const ok = await act('invite', async () => {
			minted = await createInvite(API, { role: inviteRole, cn: inviteCn.trim(), days: inviteDays });
		});
		if (ok && minted) {
			invite = minted;
			inviteCn = '';
		}
	}

	async function copyInvite() {
		if (!invite) return;
		try {
			await navigator.clipboard.writeText(invite.url);
			copied = true;
		} catch {
			copied = false;
		}
	}

	async function submitPasskey(event: SubmitEvent) {
		event.preventDefault();
		added = false;
		if (await act('add', () => addPasskey(API, { label: passkeyLabel }, { credentials: navigator.credentials }))) {
			added = true;
			passkeyLabel = '';
		}
	}

	function displayName(account: StaffAccount): string {
		return account.name || account.passkeys[0]?.label || `Staff ${shortId(account.user_id)}`;
	}

	function when(at: number | null): string {
		return at ? ago(at, now) : 'never';
	}
</script>

<svelte:head>
	<title>Staff — Admin — Apostille Handbook</title>
</svelte:head>

{#snippet refusal(key: string, heading: string)}
	{#if failure?.key === key}
		<RequestErrorNotice
			{heading}
			message={failure.message}
			requestId={failure.requestId}
			stepUp={failure.stepUp}
			busy={confirming}
			confirmLabel="Confirm with passkey and retry"
			onConfirm={confirmAndRetry}
		/>
	{/if}
{/snippet}

{#snippet passkeyRows(account: StaffAccount, canRemove: boolean)}
	<ul class="m-0 flex list-none flex-col gap-2 p-0">
		{#each account.passkeys as key (key.id)}
			<li class="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-sm border border-line px-3 py-2 text-sm">
				<KeyRoundIcon class="size-4 flex-none text-ink-muted" aria-hidden="true" />
				<span class="min-w-0 flex-1 font-medium break-words">{key.label || 'Passkey'}</span>
				<span class="text-xs text-ink-muted">added {dateTime(key.created_at)} · used {when(key.last_used_at)}</span>
				{#if canRemove}
					<Button
						variant="outline"
						size="sm"
						disabled={busyKey !== null || account.passkeys.length < 2}
						title={account.passkeys.length < 2 ? 'The last passkey stays: revoke the account instead.' : undefined}
						onclick={() => act(`passkey:${key.id}`, () => revokePasskey(API, key.id))}
					>
						{busyKey === `passkey:${key.id}` ? 'Removing…' : 'Remove'}
					</Button>
				{/if}
			</li>
			{@render refusal(`passkey:${key.id}`, 'Remove failed.')}
		{:else}
			<li class="text-sm text-ink-muted">No live passkey.</li>
		{/each}
	</ul>
{/snippet}

<div class="mt-6 flex flex-col gap-6 pb-12">
	<header class="flex flex-col gap-2">
		<Heading level={1} eyebrow="System">Staff</Heading>
		<p class="m-0 max-w-prose text-sm text-ink-muted">
			Who can sign in to this console, and with which passkeys. Every change here needs a passkey
			check from the last 10 minutes and writes an audit row.
		</p>
	</header>

	{#if loadError}
		<Notice variant="error" heading="Staff list could not load.">
			<p class="m-0">{loadError}</p>
			<p class="mt-3 mb-0"><Button onclick={load}>Try again</Button></p>
		</Notice>
	{:else if !list}
		<div class="flex flex-col gap-3" aria-busy="true">
			<Skeleton class="h-28 w-full" />
			<Skeleton class="h-40 w-full" />
		</div>
	{:else}
		<div class="grid items-start gap-6 xl:grid-cols-2">
			<Card.Root>
				<Card.Header>
					<Card.Title>Your passkeys</Card.Title>
					<Card.Description>
						Add one per device you sign in from. Sign-in needs no user ID: the browser offers the
						passkeys it holds for this console.
					</Card.Description>
				</Card.Header>
				<Card.Content class="flex flex-col gap-4">
					{#if me}
						{@render passkeyRows(me, true)}
					{/if}
					<form class="grid gap-2" onsubmit={submitPasskey}>
						<label class="text-sm font-semibold" for="passkey-label">Label for the new passkey (optional)</label>
						<div class="flex flex-col gap-2 sm:flex-row">
							<Input id="passkey-label" class="h-11 sm:max-w-xs" bind:value={passkeyLabel} maxlength={64} placeholder="Work phone" />
							<Button type="submit" size="lg" class="min-h-11 px-4" disabled={busyKey !== null}>
								<KeyRoundIcon aria-hidden="true" />
								{busyKey === 'add' ? 'Waiting for your passkey…' : 'Add a passkey'}
							</Button>
						</div>
					</form>
					{@render refusal('add', 'Adding a passkey failed.')}
					{#if added}
						<Notice variant="success" heading="Passkey added.">
							<p class="m-0">It works for sign-in on that device from now on.</p>
						</Notice>
					{/if}
				</Card.Content>
			</Card.Root>

			{#if isAdmin}
				<Card.Root>
					<Card.Header>
						<Card.Title>Invite staff</Card.Title>
						<Card.Description>
							Creates a single-use enrolment link. Send it on a separate channel; it is shown once.
						</Card.Description>
					</Card.Header>
					<Card.Content class="flex flex-col gap-4">
						<form class="grid gap-3 sm:grid-cols-[1fr_auto_auto] sm:items-end" onsubmit={submitInvite}>
							<div class="grid gap-1">
								<label class="text-sm font-semibold" for="invite-cn">Name</label>
								<Input id="invite-cn" class="h-11" bind:value={inviteCn} maxlength={64} placeholder="jdoe" autocomplete="off" spellcheck={false} required />
							</div>
							<div class="grid gap-1">
								<label class="text-sm font-semibold" for="invite-role">Role</label>
								<select id="invite-role" class="h-11 rounded-sm border border-line bg-bg px-3 text-sm" bind:value={inviteRole}>
									<option value="staff">Staff</option>
									<option value="admin">Admin</option>
								</select>
							</div>
							<div class="grid gap-1">
								<label class="text-sm font-semibold" for="invite-days">Valid for (days)</label>
								<Input id="invite-days" class="h-11 w-24" type="number" min={1} max={30} bind:value={inviteDays} />
							</div>
							<Button type="submit" size="lg" class="min-h-11 px-4 sm:col-span-3 sm:justify-self-start" disabled={busyKey !== null || !validCn(inviteCn.trim())}>
								<UserPlusIcon aria-hidden="true" />
								{busyKey === 'invite' ? 'Creating…' : 'Create invite link'}
							</Button>
						</form>
						{#if inviteCn.trim() && !validCn(inviteCn.trim())}
							<p class="m-0 text-sm text-error" role="alert">Name: letters, digits, dots, dashes or underscores, up to 64.</p>
						{/if}
						{@render refusal('invite', 'Invite failed.')}
						{#if invite}
							<Notice variant="success" heading={`Invite for ${invite.cn} (${invite.role}).`}>
								<p class="m-0 text-sm">Works once, until {dateTime(invite.expires_at)}. This is the only time it is shown.</p>
								<p class="mt-2 mb-0 flex flex-wrap items-center gap-2">
									<code class="min-w-0 flex-1 rounded-xs border border-line bg-surface px-2 py-1 font-mono text-xs break-all select-all">{invite.url}</code>
									<Button variant="outline" size="sm" onclick={copyInvite}>
										<CopyIcon aria-hidden="true" />
										{copied ? 'Copied' : 'Copy link'}
									</Button>
								</p>
							</Notice>
						{/if}
						{#if list.invites.length > 0}
							<div class="flex flex-col gap-2">
								<h3 class="m-0 text-sm font-semibold">Pending invites</h3>
								<ul class="m-0 flex list-none flex-col gap-2 p-0">
									{#each list.invites as pending (pending.id)}
										<li class="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-sm border border-line px-3 py-2 text-sm">
											<span class="font-medium">{pending.cn}</span>
											<Badge variant="outline">{pending.role}</Badge>
											<span class="flex-1 text-xs text-ink-muted">expires {dateTime(pending.expires_at)}</span>
											<Button
												variant="outline"
												size="sm"
												disabled={busyKey !== null}
												onclick={() => act(`withdraw:${pending.id}`, () => withdrawInvite(API, pending.id))}
											>
												{busyKey === `withdraw:${pending.id}` ? 'Withdrawing…' : 'Withdraw'}
											</Button>
										</li>
										{@render refusal(`withdraw:${pending.id}`, 'Withdraw failed.')}
									{/each}
								</ul>
							</div>
						{/if}
					</Card.Content>
				</Card.Root>
			{/if}
		</div>

		<section aria-labelledby="staff-accounts" class="flex flex-col gap-3">
			<h2 id="staff-accounts" class="m-0 text-h3 font-semibold">Accounts ({list.accounts.length})</h2>
			<ul class="m-0 flex list-none flex-col gap-3 p-0">
				{#each [...(me ? [me] : []), ...others] as account (account.user_id)}
					{@const self = account.user_id === list.me.user_id}
					<li>
						<Card.Root>
							<Card.Content class="flex flex-col gap-3">
								<div class="flex flex-wrap items-center gap-x-3 gap-y-2">
									<span class="text-base font-semibold break-words">{displayName(account)}</span>
									<Badge variant={account.role === 'admin' ? 'default' : 'secondary'}>{account.role}</Badge>
									{#if self}<Badge variant="outline">you</Badge>{/if}
									{#if account.locked_at}<Badge variant="destructive">revoked {when(account.locked_at)}</Badge>{/if}
									<code class="font-mono text-xs text-ink-muted" title={account.user_id}>{shortId(account.user_id)}</code>
									<span class="text-xs text-ink-muted">since {dateTime(account.created_at)}</span>
									{#if isAdmin && !self && !account.locked_at}
										<span class="ml-auto flex gap-2">
											{#if armed === account.user_id}
												<Button variant="outline" size="sm" onclick={() => (armed = null)}>Keep</Button>
												<Button
													variant="destructive"
													size="sm"
													disabled={busyKey !== null}
													onclick={async () => {
														if (await act(`revoke:${account.user_id}`, () => revokeStaff(API, account.user_id))) armed = null;
													}}
												>
													{busyKey === `revoke:${account.user_id}` ? 'Revoking…' : `Revoke ${displayName(account)}`}
												</Button>
											{:else}
												<Button variant="outline" size="sm" disabled={busyKey !== null} onclick={() => (armed = account.user_id)}>
													Revoke access
												</Button>
											{/if}
										</span>
									{/if}
								</div>
								{#if armed === account.user_id}
									<p class="m-0 text-sm">
										Ends every session, removes every passkey and locks the account. A new invite creates a
										new account.
									</p>
								{/if}
								{@render refusal(`revoke:${account.user_id}`, 'Revoke failed.')}
								{#if !self && !account.locked_at}
									{@render passkeyRows(account, isAdmin)}
								{/if}
							</Card.Content>
						</Card.Root>
					</li>
				{/each}
			</ul>
		</section>
	{/if}
</div>
