// Port-0 static file server for the E2E suite. Serves one build
// dir over 127.0.0.1 with the adapter-static SPA fallback
// (unknown paths → index.html, mirroring the axum shell).
// API traffic never reaches it: specs intercept API routes.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { extname, join, normalize, sep } from 'node:path';

const TYPES = {
	'.html': 'text/html; charset=utf-8',
	'.js': 'text/javascript; charset=utf-8',
	'.css': 'text/css; charset=utf-8',
	'.json': 'application/json; charset=utf-8',
	'.svg': 'image/svg+xml',
	'.png': 'image/png',
	'.ico': 'image/x-icon',
	'.txt': 'text/plain; charset=utf-8',
	'.webmanifest': 'application/manifest+json'
};

/**
 * @param {string} dir build dir to serve (e.g. build-admin)
 * @param {{ base?: string }} [opts] mount point (public build: '/apostille')
 * @returns {Promise<{ url: string, close: () => Promise<void> }>}
 */
export function serveDir(dir, opts = {}) {
	const root = normalize(dir);
	const base = (opts.base ?? '').replace(/\/$/, '');
	const server = createServer(async (req, res) => {
		try {
			let path = new URL(req.url ?? '/', 'http://x').pathname;
			if (base) {
				if (path !== base && !path.startsWith(`${base}/`)) {
					res.writeHead(404, { 'content-type': 'text/plain' });
					res.end('not found');
					return;
				}
				path = path.slice(base.length) || '/';
			}
			const rel = normalize(path).replace(/^(\.\.[/\\])+/, '').replace(/^[/\\]/, '');
			let file = join(root, rel);
			if (!file.startsWith(root + sep) && file !== root) file = join(root, 'index.html');
			else if (!existsSync(file) || !rel || rel.endsWith('/')) {
				const html = join(root, `${rel.replace(/\/$/, '')}.html`);
				file = existsSync(html) && rel ? html : join(root, 'index.html');
			}
			const body = await readFile(file);
			res.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' });
			res.end(body);
		} catch {
			res.writeHead(404, { 'content-type': 'text/plain' });
			res.end('not found');
		}
	});
	return new Promise((resolve) => {
		server.listen(0, '127.0.0.1', () => {
			const { port } = server.address();
			resolve({
				url: `http://127.0.0.1:${port}`,
				close: () => new Promise((done) => server.close(() => done()))
			});
		});
	});
}
