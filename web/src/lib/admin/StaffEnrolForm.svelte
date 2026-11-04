<script lang="ts">
	import { Button } from '#lib/components/ui/button/index.js';
	import { Input } from '#lib/components/ui/input/index.js';
	import {
		CANCELLED_COPY,
		cookieValue,
		enrolErrorCopy,
		enrolPasskey,
		isTransportFailure
	} from './staff-auth.js';

	// One staff passkey enrolment: display name + the UV ceremony.
	// Used by /enrol (invite link or typed setup key) and by the /config
	// wizard, which already holds the setup key, so the key is pasted
	// once. The caller keeps its gate (key or invite) until `onenrolled`
	// fires: a cancelled or failed passkey prompt can simply retry.
	// Admin-only: never imported from (public) or (customer) routes.
	interface Grant {
		role: string;
		cn: string;
		userId: string;
	}

	interface Props {
		/** Exactly one of setupKey / invite. */
		setupKey?: string;
		invite?: string;
		submitLabel?: string;
		/** Runs before the ceremony; returning false stops it (shape checks). */
		precheck?: () => boolean;
		onenrolled: (grant: Grant) => void;
	}

	let { setupKey = '', invite = '', submitLabel = 'Enrol this device', precheck, onenrolled }: Props =
		$props();

	const setupMode = $derived(invite === '');
	let displayName = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);

	async function post(path: string, body: unknown) {
		const res = await fetch(`/apostille/api/v1${path}`, {
			method: 'POST',
			headers: {
				'content-type': 'application/json',
				'x-csrf': cookieValue(document.cookie, 'hb_csrf')
			},
			credentials: 'same-origin',
			body: JSON.stringify(body)
		});
		const parsed = await res.json().catch(() => ({}));
		return { ok: res.ok, status: res.status, body: parsed };
	}

	async function enrol() {
		error = null;
		if (precheck && !precheck()) return;
		busy = true;
		try {
			const res = await enrolPasskey(
				setupMode
					? { setupKey: setupKey.trim(), displayName: displayName.trim() }
					: { invite, displayName: displayName.trim() },
				{ post, credentials: navigator.credentials }
			);
			if (!res.ok) {
				error = enrolErrorCopy(res.status, res.body?.error ?? '', setupMode);
				return;
			}
			onenrolled({ role: res.body.role, cn: res.body.cn, userId: res.body.user_id ?? '' });
		} catch (err) {
			if (err instanceof DOMException && err.name === 'NotAllowedError') {
				error = CANCELLED_COPY;
			} else if (isTransportFailure(err)) {
				error =
					'Could not reach the admin host — check the connection and reload. (Refused networks answer with a 403 page naming the address instead.)';
			} else {
				error = 'Enrolment failed — try again.';
			}
		} finally {
			busy = false;
		}
	}
</script>

<form
	class="grid gap-3"
	onsubmit={(e) => {
		e.preventDefault();
		enrol();
	}}
>
	<label class="font-bold" for="enrol-display-name">Display name</label>
	<Input
		id="enrol-display-name"
		class="h-11 text-base"
		type="text"
		bind:value={displayName}
		autocomplete="name"
		placeholder="Jordan Doe"
		maxlength={128}
		required
	/>
	<Button
		type="submit"
		size="lg"
		class="min-h-11 w-full px-4 text-base sm:w-auto sm:justify-self-start"
		disabled={busy || displayName.trim() === ''}
	>
		{busy ? 'Waiting for your passkey…' : submitLabel}
	</Button>
	{#if error}
		<p class="m-0 font-bold" role="alert">{error}</p>
	{/if}
</form>
