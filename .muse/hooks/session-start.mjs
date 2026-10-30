// SessionStart: every new, resumed or compacted session starts knowing the
// scoreboard, the next red row and what waits on the operator, so nobody
// pastes status back in.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { emit, git, readInput } from './lib.mjs';

readInput();
const lines = [
  'Apostille Handbook, hackathon mode. Brief: .agents/goal.md. Oracle: `node .agents/scoreboard/scoreboard.mjs` (run it; never edit .agents/ or .muse/).',
];

const board = 'web/shots/scoreboard.json';
if (existsSync(board)) {
  try {
    const s = JSON.parse(readFileSync(board, 'utf8'));
    const mins = Math.round((Date.now() - statSync(board).mtimeMs) / 60000);
    lines.push(`Last scoreboard, ${mins} min ago against ${s.base}: ${s.passing}/${s.total} passing. Next red row: ${s.next ?? 'none'}.`);
    for (const c of s.checks ?? []) if (!c.pass) lines.push(`- ${c.id}: ${c.detail}`);
  } catch {
    lines.push(`${board} is unreadable; run the scoreboard.`);
  }
} else {
  lines.push('No scoreboard run yet. Run it before choosing work.');
}

try {
  lines.push(`Recent commits:\n${git('log', '--oneline', '-5').trim()}`);
} catch {
  // not a git checkout
}
if (existsSync('docs/ops/operator-today.md')) lines.push('Open operator steps: docs/ops/operator-today.md');

// The prompt-and-response log lives in the MAIN checkout (this may be the
// judge's worktree). Its last error matters only while no digest is newer.
try {
  const main = dirname(git('rev-parse', '--path-format=absolute', '--git-common-dir').trim());
  const log = join(main, '.muse/session-log/errors.log');
  if (existsSync(log)) {
    const dir = join(main, 'docs/prompts/sessions');
    const digests = existsSync(dir) ? readdirSync(dir).filter((f) => /^\d{4}-\d\d-\d\d-.+\.md$/.test(f)) : [];
    const newest = Math.max(0, ...digests.map((f) => statSync(join(dir, f)).mtimeMs));
    if (statSync(log).mtimeMs > newest) {
      const last = readFileSync(log, 'utf8').trim().split('\n').at(-1) ?? '';
      lines.push(`Session log (docs/prompts/sessions): .muse/session-log/errors.log has an entry newer than the newest digest, so the log may be behind: ${last.slice(0, 400)}. Tell the operator; do not edit .muse/.`);
    }
  }
} catch {
  // not a git checkout, or nothing to report
}

emit({ hookSpecificOutput: { hookEventName: 'SessionStart', additionalContext: lines.join('\n') } });
