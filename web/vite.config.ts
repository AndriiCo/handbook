import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// Two builds from one source (spa-separation.md §1.2), keys verified
// against @sveltejs/kit 3.1.0 + adapter-static 4 (files.routes,
// paths.base, adapter pages):
// - public: WEB_BUILD unset → routes-public/ ((public)+(customer)
//   only, symlinked), base /apostille, output build/.
// - admin:  WEB_BUILD=admin → full src/routes, base '' (admin host
//   serves from /), output build-admin/.
const isAdmin = process.env.WEB_BUILD === 'admin';

export default defineConfig({
	plugins: [
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},

			// Admin chunks must never exist on the public origin.
			files: { routes: isAdmin ? 'src/routes' : 'src/routes-public' },

			// axum serves the static build; SPA fallback covers deep links.
			adapter: adapter({
				fallback: 'index.html',
				pages: isAdmin ? 'build-admin' : 'build'
			}),
			paths: { base: isAdmin ? '' : '/apostille' }
		})
	]
});
