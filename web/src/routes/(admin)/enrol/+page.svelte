<script lang="ts">
	import { onMount } from 'svelte';
	import SecretInput from '../../../lib/admin/SecretInput.svelte';
	import StaffEnrolForm from '../../../lib/admin/StaffEnrolForm.svelte';
	import { resolve } from '$app/paths';
	import Notice from '../../../lib/public-ui/Notice.svelte';
	import { Button } from '#lib/components/ui/button/index.js';
	import { validInviteShape, validSetupKeyShape } from '../../../lib/admin/staff-auth.js';
	import { fetchSetupStatus } from '../../../lib/admin/setup-client.js';

	// Staff enrolment (spec customer-auth.md §5.2): single-use invite
	// URL + passkey UV ceremony (no client cert: the IP allowlist gates
	// the branch instead). The invite token is never rendered or logged;
	// it stays in the address bar until enrolment succeeds (so a reload
	// or a cancelled passkey prompt can retry) and is stripped then.
	// With ?setup=1 (secure-config.md §9.2) the setup key replaces the
	// invite for the first staffer and burns on success; /config runs
	// the same enrolment with the key it already holds.
	let linkState = $state<'checking' | 'invalid' | 'setup' | 'closed' | 'ready'>('checking');
	let grant = $state<{ role: string; cn: string; userId: string } | null>(null);
	let invite = $state('');
	let keyInput = $state('');
	let keyTouched = $state(false);
	const webauthnOk = typeof navigator !== 'undefined' && !!navigator.credentials?.create;

	const setupMode = $derived(linkState === 'setup');

	onMount(async () => {
		const params = new URLSearchParams(window.location.search);
		const setup = params.get('setup') === '1';
		if (setup) {
			try {
				const status = await fetchSetupStatus('/apostille/api/v1');
				linkState = status.setup_required ? 'setup' : 'closed';
			} catch {
				linkState = 'closed';
			}
			return;
		}
		invite = params.get('invite') ?? '';
		linkState = validInviteShape(invite) ? 'ready' : 'invalid';
	});

	function keyShapeOk(): boolean {
		keyTouched = true;
		return validSetupKeyShape(keyInput.trim());
	}

	function enrolled(g: { role: string; cn: string; userId: string }) {
		grant = g;
		keyInput = '';
		invite = '';
		// The credential is spent: drop it from the address bar now.
		history.replaceState(null, '', window.location.pathname);
	}
</script>

<svelte:head>
	<title>Staff enrolment — Apostille Handbook</title>
</svelte:head>

<h1 class="m-0 mb-4 text-h2 font-semibold">Staff enrolment</h1>

{#if linkState === 'checking' || !webauthnOk}
	{#if !webauthnOk}
		<Notice variant="warning" heading="Passkeys unavailable.">
			<p>This browser cannot do passkeys. Use a current Safari, Chrome, Edge, or Firefox.</p>
		</Notice>
	{:else}
		<p>Checking invite link…</p>
	{/if}
{:else if grant}
	<Notice variant="success" heading="Enrolled.">
		<p>
			Signed in as {grant.role}. Next time, sign in with this passkey: no user ID needed.
		</p>
	</Notice>
	<p class="mt-4">
		<Button href={resolve('/(admin)/review')} size="lg" class="min-h-11 w-full px-4 text-base sm:w-auto">
			Continue to the review queue →
		</Button>
	</p>
{:else if linkState === 'invalid'}
	<Notice variant="error" heading="Invalid invite link.">
		<p>This link is malformed or missing its token — ask your admin for a new invite link.</p>
	</Notice>
{:else if linkState === 'closed'}
	<Notice variant="error" heading="Setup window is closed.">
		<p>
			A staff passkey already exists, so the setup key no longer works — <a
				class="font-semibold underline"
				href={resolve('/(admin)/staff-login')}>sign in</a
			> or ask your admin for an invite link.
		</p>
	</Notice>
{:else}
	<p class="mb-4">
		{#if setupMode}
			Create the first staff passkey. No invite or certificate needed — but enrolment burns the setup key.
		{:else}
			Create your staff passkey. Single-use link — it stops working after enrolment.
		{/if}
	</p>
	{#if setupMode}
		<div class="mb-3 grid gap-1">
			<label class="font-bold" for="enrol-setup-key">Setup key</label>
			<SecretInput
				id="enrol-setup-key"
				class="min-h-11 rounded-sm border border-line bg-bg px-[0.8rem] py-[0.6rem] font-mono text-base"
				bind:value={keyInput}
				placeholder="setup_…"
				required
				hint="One-time key from the server console. Your browser will not offer to save it."
			/>
			{#if keyTouched && !validSetupKeyShape(keyInput.trim())}
				<p class="m-0 font-bold text-error" role="alert">
					That key is malformed — `setup_` + 64 hex chars.
				</p>
			{/if}
		</div>
		<StaffEnrolForm setupKey={keyInput} precheck={keyShapeOk} onenrolled={enrolled} />
	{:else}
		<StaffEnrolForm {invite} onenrolled={enrolled} />
	{/if}
{/if}
