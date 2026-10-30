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
<p class="rounded-sm border border-line bg-warning-bg px-4 py-3 text-ink">
	<strong>Mock login.</strong> Accepts {DEMO_REFS.join(' / ')}; anything else shows the error state.
</p>

{#if loggedInAs}
	<p class="rounded-sm border border-line bg-success-bg px-4 py-3" role="status">
		Logged in as <code class="font-mono">{loggedInAs}</code>.
		<a href={resolve('/(mock)/mock/orders')}>Continue to orders</a> ·
		<button class="cursor-pointer border-0 bg-transparent p-0 text-accent-strong underline" onclick={logout}>
			Log out
		</button>
	</p>
{:else}
	<form class="flex max-w-[28rem] flex-col gap-2" onsubmit={login}>
		<label class="font-bold" for="customer-ref">Customer reference</label>
		<input
			class="min-h-11 rounded-xs border border-line bg-bg px-[0.8rem] py-[0.6rem] text-ink"
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
			<p class="rounded-sm border border-line bg-error-bg px-4 py-3 text-ink" id="login-error" role="alert">
				{error}
			</p>
		{/if}
		<button
			class="min-h-11 cursor-pointer rounded-sm border border-accent-fill bg-accent-fill font-bold text-accent-ink"
			type="submit">Login</button
		>
	</form>
{/if}
