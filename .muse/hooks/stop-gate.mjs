// Stop: a turn that leaves web/src broken does not end. Kept cheap on
// purpose (svelte-check only, only when web/src has uncommitted changes);
// screenshots and the scoreboard belong to the skills' done criteria.
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { emit, git, readInput } from './lib.mjs';

const input = readInput();
if (input.stop_hook_active) process.exit(0);

let dirty = '';
try {
  dirty = git('status', '--porcelain', '--', 'web/src').trim();
} catch {
  process.exit(0);
}
if (!dirty) process.exit(0);

const r = spawnSync('npm', ['--prefix', 'web', 'run', 'check'], { encoding: 'utf8', timeout: 240_000 });
if (r.status === 0) process.exit(0);

const out = `${r.stdout ?? ''}${r.stderr ?? ''}`;
const errors = out.split('\n').filter((l) => /error|✖/i.test(l)).slice(-20).join('\n') || out.slice(-1500);

// After the same failure has blocked two turns, let the next one end and report.
const ledger = join(tmpdir(), `muse-stop-gate-${process.env.MUSE_SESSION_ID || 'session'}`);
const hash = createHash('sha1').update(errors).digest('hex');
const seen = existsSync(ledger) ? readFileSync(ledger, 'utf8').split('\n') : [];
writeFileSync(ledger, [...seen, hash].slice(-20).join('\n'));
if (seen.filter((h) => h === hash).length >= 2) {
  console.error('stop-gate: the same svelte-check failure blocked two turns; letting this one end');
  process.exit(0);
}

emit({
  decision: 'block',
  reason: `web/src has uncommitted changes and \`npm --prefix web run check\` fails. Fix these before ending the turn:\n${errors}`,
});
