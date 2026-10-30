// SessionStart: every new, resumed or compacted session starts knowing the
// scoreboard, the next red row and what waits on the operator, so nobody
// pastes status back in.
import { existsSync, readFileSync, statSync } from 'node:fs';
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

emit({ hookSpecificOutput: { hookEventName: 'SessionStart', additionalContext: lines.join('\n') } });
