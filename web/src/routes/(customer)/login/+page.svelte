<script lang="ts">
	import { resolve } from '$app/paths';
	import { STRINGS, resolveLocale, type Locale } from '../../../lib/i18n';

	let locale = $state<Locale>(resolveLocale());
	let customerRef = $state('');
	let error = $state<string | null>(null);
	let loggedInAs = $state<string | null>(null);

	// Kit 3 types expose no `base`, so the API root derives from the
	// home route id (base + '/'). Same value in both builds' sync.
	const apiRoot = resolve('/(public)').replace(/\/$/, '');
	const api = (path: string) => `${apiRoot}/api/v1${path}`;
	const t = $derived(STRINGS[locale]);

	async function login(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		try {
			const res = await fetch(api('/demo/login'), {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				credentials: 'same-origin',
				body: JSON.stringify({ customer_ref: customerRef })
			});
			if (res.status === 422) {
				error = 'Invalid customer reference (rejected by the server).';
				return;
			}
			if (!res.ok) throw new Error(`HTTP ${res.status}`);
			loggedInAs = customerRef;
		} catch {
			error = 'Login service unreachable — is the API server running? Nothing was logged in.';
		}
	}
</script>

<svelte:head>
	<title>{t.loginLabel} — Apostille Handbook</title>
</svelte:head>

<h1>{t.loginLabel}</h1>
<p class="demo"><strong>DEMO login only.</strong> Real auth (passkey / OAuth / magic link) is not built yet.</p>

{#if loggedInAs}
	<p class="notice ok" role="status">
		Logged in as <code>{loggedInAs}</code>. <a href={resolve('/(customer)/orders')}>Continue to {t.orderLabel}s</a>
	</p>
{:else}
	<form onsubmit={login}>
		<label for="customer-ref">Customer reference</label>
		<input
			id="customer-ref"
			name="customer_ref"
			type="text"
			bind:value={customerRef}
			required
			autocomplete="username"
			aria-describedby={error ? 'login-error' : undefined}
		/>
		{#if error}
			<p class="notice err" id="login-error" role="alert">{error}</p>
		{/if}
		<button type="submit">{t.loginLabel}</button>
	</form>
{/if}

<style>
	.demo {
		background: var(--warning-bg);
		color: var(--text);
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
	form {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		max-width: 28rem;
	}
	label {
		font-weight: 700;
	}
	input {
		min-height: 44px;
		padding: 0.6rem 0.8rem;
		font: inherit;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius-xs);
	}
	button {
		min-height: 44px;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
	}
	:root[data-theme='dark'] button {
		background: var(--accent-fill);
		border-color: var(--accent-fill);
	}
	.notice {
		border: 1px solid var(--border);
		border-radius: var(--radius-sm);
		padding: 0.75rem 1rem;
	}
	.notice.ok {
		background: var(--success-bg);
	}
	.notice.err {
		background: var(--error-bg);
		color: var(--text);
	}
	code {
		font-family: var(--font-mono);
	}
</style>
