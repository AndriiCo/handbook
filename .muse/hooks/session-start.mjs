// SessionStart: every new, resumed or compacted session starts knowing its
// role file, the real production and staging scores, the newest judge
// verdict and what waits on the operator, so nobody pastes status back in.
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { emit, git, readInput } from './lib.mjs';

const input = readInput();
const lines = [
  'Apostille Handbook, hackathon mode. Rules: AGENTS.md. Brief, state and calendar: .agents/goal.md. Oracle: `node .agents/scoreboard/scoreboard.mjs` (run it; never edit .agents/ or .muse/).',
];

// run.sh starts hooks at the checkout's top level: the main checkout for the
// builder, `.muse/worktrees/<id>` for the judge and the lanes.
const here = process.cwd();
let main = here;
try {
  main = dirname(git('rev-parse', '--path-format=absolute', '--git-common-dir').trim());
} catch {
  // not a git checkout
}

lines.push(roleLine(sessionName(input.session_id ?? process.env.MUSE_SESSION_ID), here !== main));

// Scores come from full runs only: the newer of prod-now.json (the
// builder's session-start check) and prod-after.json (its run after the
// last deploy), and staging-accept.json (its last staging acceptance). A
// partial `--only` run goes to scoreboard-partial.json and is ignored.
lines.push(scoreLine('Production', newest(['web/shots/prod-now.json', 'web/shots/prod-after.json'].map((f) => join(main, f)))));
lines.push(scoreLine('Staging', join(main, 'web/shots/staging-accept.json')));

const verdict = newestJudge(join(main, '.muse/worktrees'));
if (verdict) lines.push(`Newest judge verdict (${verdict.path}, ${age(verdict.mtimeMs)}):\n${verdict.head}`);

try {
  lines.push(`Recent commits on main:\n${git('-C', main, 'log', '--oneline', '-5', 'main').trim()}`);
} catch {
  // not a git checkout
}
if (existsSync(join(main, 'docs/ops/operator-today.md'))) lines.push('Open operator steps: docs/ops/operator-today.md');

// The prompt-and-response log lives in the MAIN checkout. Its last error
// matters only while no digest is newer.
try {
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
  // nothing to report
}

emit({ hookSpecificOutput: { hookEventName: 'SessionStart', additionalContext: lines.join('\n') } });

// ------------------------------------------------------------ helpers

// The `/name` the operator gave this session, from Muse's session index
// (read-only). A brand-new session has none yet: the role then comes from
// where it runs, and from the /goal the operator types.
function sessionName(id) {
  if (!id) return '';
  try {
    const { DatabaseSync } = process.getBuiltinModule('node:sqlite');
    const db = new DatabaseSync(join(process.env.HOME ?? '', '.local/share/muse/session-index.db'), { readOnly: true });
    try {
      return db.prepare('select logged_session_name as n from sessions where session_id = ?').get(id)?.n ?? '';
    } finally {
      db.close();
    }
  } catch {
    return '';
  }
}

function roleLine(name, inWorktree) {
  const label = name ? `Session "${name}"` : 'This session';
  if (/^builder/i.test(name)) return `${label}: role builder. Follow .agents/roles/builder.md, start-of-session steps first.`;
  if (/^judge/i.test(name)) return `${label}: role judge. Follow the judge-eyes skill (/judge-eyes); write only web/shots/judge.md in this worktree.`;
  const lane = name.toLowerCase().replace(/^lane-/, '');
  if (lane && /^[a-z0-9-]+$/.test(lane) && [here, main].some((root) => existsSync(join(root, `.agents/roles/lane-${lane}.md`)))) {
    return `${label}: role lane "${lane}". Follow .agents/roles/lane-${lane}.md (merge main first if this worktree lacks it).`;
  }
  if (!inWorktree) return `${label} runs in the main checkout: the builder's role file is .agents/roles/builder.md. If you are not the builder, follow the role file your /goal names.`;
  return `${label} runs in a worktree: follow the role file your /goal names (.agents/roles/lane-<name>.md) or, as the judge, the judge-eyes skill.`;
}

function scoreLine(label, file) {
  const name = file.split('/').at(-1);
  if (!existsSync(file)) return `${label}: no full scoreboard run yet (${name}).`;
  try {
    const s = JSON.parse(readFileSync(file, 'utf8'));
    const version = /version (\S+)/.exec(s.checks?.find((c) => c.id === 'P0')?.detail ?? '')?.[1] ?? '?';
    const red = (s.checks ?? []).filter((c) => !c.pass).map((c) => c.id);
    return `${label} (${name}, ${age(statSync(file).mtimeMs)}, ${version}): ${s.passing}/${s.total} passing${red.length ? `; red: ${red.join(', ')}` : ''}.`;
  } catch {
    return `${label}: ${name} is unreadable.`;
  }
}

function newestJudge(dir) {
  try {
    const best = readdirSync(dir)
      .map((wt) => ({ wt, file: join(dir, wt, 'web/shots/judge.md') }))
      .filter(({ file }) => existsSync(file))
      .map((j) => ({ ...j, mtimeMs: statSync(j.file).mtimeMs }))
      .sort((a, b) => b.mtimeMs - a.mtimeMs)[0];
    if (!best) return null;
    const head = readFileSync(best.file, 'utf8').split('\n').slice(0, 4).join('\n');
    return { path: `.muse/worktrees/${best.wt}/web/shots/judge.md`, mtimeMs: best.mtimeMs, head };
  } catch {
    return null;
  }
}

function newest(files) {
  const found = files.filter((f) => existsSync(f));
  return found.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0] ?? files.at(-1);
}

function age(ms) {
  const mins = Math.round((Date.now() - ms) / 60000);
  return mins < 90 ? `${mins} min ago` : `${Math.round(mins / 60)} h ago`;
}
