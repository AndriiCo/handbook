// Port-0 static file server for the E2E suite. Serves one build
// dir over 127.0.0.1 with the adapter-static SPA fallback
// (unknown paths → index.html, mirroring the axum shell).
// API traffic never reaches it: specs intercept API routes.
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
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
	// Minimal WS acceptor for the live-bus topics (e.g. /ws/bus):
	// completes the handshake then a clean close, so subscribing pages
	// log no console error under screenshots. It speaks no application
	// frames; the real server owns those. Upgraded sockets detach from
	// the server's connection tracking, so they are tracked here and
	// destroyed on close — otherwise close() waits them out.
	const hijacked = new Set();
	server.on('upgrade', (req, socket) => {
		hijacked.add(socket);
		socket.on('close', () => hijacked.delete(socket));
		try {
			const path = new URL(req.url ?? '/', 'http://x').pathname;
			const key = req.headers['sec-websocket-key'];
			if (!path.includes('/ws/') || !key) {
				socket.destroy();
				return;
			}
			const accept = createHash('sha1')
				.update(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11')
				.digest('base64');
			socket.write(
				'HTTP/1.1 101 Switching Protocols\r\n' +
					'Upgrade: websocket\r\n' +
					'Connection: Upgrade\r\n' +
					`Sec-WebSocket-Accept: ${accept}\r\n\r\n`
			);
			socket.write(Buffer.from([0x88, 0x00])); // clean close, no payload
			socket.end();
		} catch {
			try {
				socket.destroy();
			} catch {}
		}
	});
	function dropHijacked() {
		for (const s of hijacked) {
			try {
				s.destroy();
			} catch {}
		}
		hijacked.clear();
	}
	return new Promise((resolve) => {
		server.listen(0, '127.0.0.1', () => {
			const { port } = server.address();
			resolve({
				url: `http://127.0.0.1:${port}`,
				close: () =>
					new Promise((done) => {
						// Specs close with live pages open: drop every
						// socket first (upgraded WS detaches from server
						// tracking; keep-alive HTTP lingers) so close()
						// never waits out the browser.
						dropHijacked();
						server.closeAllConnections();
						server.close(() => done());
					})
			});
		});
	});
}
