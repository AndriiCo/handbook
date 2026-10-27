<script lang="ts">
	import { resolve } from '$app/paths';
	import { DEMO_REFS } from '../../../../lib/mock/fixtures';

	let customerRef = $state('');
	let error = $state<string | null>(null);
	let loggedInAs = $state<string | null>(null);

	function login(event: SubmitEvent) {
		event.preventDefault();
		error = null;
		const ref = customerRef.trim();
		if (!ref) {
			error = 'Enter a customer reference to continue.';
			return;
		}
		if (!DEMO_REFS.includes(ref)) {
			error = `Unknown reference “${ref}” — try ${DEMO_REFS.join(' or ')}. (Fixture check, no server.)`;
			return;
		}
		loggedInAs = ref;
	}

	function logout() {
		loggedInAs = null;
		customerRef = '';
	}
</script>

<svelte:head>
	<title>Mock login — Apostille Handbook</title>
</svelte:head>

<h1>Login</h1>
<p class="demo"><strong>Mock login.</strong> Accepts {DEMO_REFS.join(' / ')}; anything else shows the error state.</p>

{#if loggedInAs}
	<p class="notice ok" role="status">
		Logged in as <code>{loggedInAs}</code>.
		<a href={resolve('/(mock)/mock/orders')}>Continue to orders</a> ·
		<button class="link" onclick={logout}>Log out</button>
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
			placeholder={DEMO_REFS[0]}
			aria-describedby={error ? 'login-error' : undefined}
		/>
		{#if error}
			<p class="notice err" id="login-error" role="alert">{error}</p>
		{/if}
		<button type="submit">Login</button>
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
	button[type='submit'] {
		min-height: 44px;
		font: inherit;
		font-weight: 700;
		color: var(--accent-ink);
		background: var(--accent);
		border: 1px solid var(--accent);
		border-radius: var(--radius-sm);
		cursor: pointer;
	}
	:root[data-theme='dark'] button[type='submit'] {
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
	.link {
		background: none;
		border: none;
		padding: 0;
		font: inherit;
		color: var(--accent);
		text-decoration: underline;
		cursor: pointer;
	}
</style>
