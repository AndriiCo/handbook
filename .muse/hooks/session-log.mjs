// Session log v2: the prompt-and-response record, written from Muse's own
// session log instead of by the agent. One Markdown file per session in
// docs/prompts/sessions/ of the MAIN checkout (builder, judge, side chats,
// any other session in this repo): every turn, what started it (you, the
// goal, a scheduled job, a message from another session, a subagent, a
// background task), the prompt, every final answer with its time, your
// answers to its questions, and the commits the turn made.
//
// Source: Muse's own redaction. The raw session log is copied with only the
// records a digest uses (under 2% of it: prompts, answers, inbox deliveries,
// questions, turn ends, HEAD observations, names, spawns), and that copy
// goes through `muse export --redacted`, which is then stream-parsed. A
// whole-session export costs Muse ~8x the raw log in memory (3.3 GB for the
// 228 MB log of 2026-10-27); the copy costs ~35 MB. Tool calls are read raw
// only to count history-writing git commands. Tool output and reasoning are
// left out.
// The repo is public and its history is never rewritten, so a scrub runs on
// every text before it is capped or quoted (pasted terminal output, shell
// prompts, journald lines, PEM blocks, key-shaped strings, IP addresses,
// home and repo paths, sudo rules, key-file paths, and the literals in the
// git-ignored .muse/session-log.scrub.json, which must exist), and a final
// check refuses to write a digest that still holds a PEM block, a
// key-shaped token or long hex string, a non-loopback address, a home
// path, a sudo rule or a scrub-map literal. Refusals and failures go to
// .muse/session-log/errors.log; session-start.mjs reports the last one.
//
//   node .muse/hooks/session-log.mjs                     # every repo session active since --since
//   node .muse/hooks/session-log.mjs --session <uuid>
//   node .muse/hooks/session-log.mjs --export <file>     # an existing `muse export --redacted` file
//   node .muse/hooks/session-log.mjs --audit             # tracked files that hold scrub-map literals
//   run.sh session-log --hook Stop|StopFailure|SessionEnd
//   options: --since <date> (activity time; earlier turns are left out)
//            --workspace <dir> (whose sessions; default: the main checkout)  --out <dir>
//
// A --since before the default cutoff (sessions from before this log
// existed) writes to the git-ignored .muse/session-log/review/ unless --out
// is given: older sessions discussed production in ways no pattern catches,
// so a person reads each digest before moving it into docs/prompts/sessions.
//
// Skipped: sessions named ^(ops|private) (and their side chats), and ids
// listed one per line in .muse/session-log.skip.
import { execFileSync, spawn } from 'node:child_process';
import {
  appendFileSync, closeSync, existsSync, mkdirSync, mkdtempSync, openSync, readdirSync, readFileSync, readSync,
  realpathSync, renameSync, rmSync, statSync, unlinkSync, writeFileSync, writeSync,
} from 'node:fs';
import { isIP } from 'node:net';
import { homedir, setPriority, tmpdir } from 'node:os';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readInput } from './lib.mjs';

const SELF = fileURLToPath(import.meta.url);
const args = parseArgs(process.argv.slice(2));
const DEFAULT_SINCE = Date.parse('2026-10-30T02:00:00Z'); // 001-095 are the per-prompt files
const CAP = { prompt: 4000, answer: 12000, message: 4000, line: 300 };
const GOAL_STEP = 'Continue working toward the active session goal';
const SKIP_NAME = /^(ops|private)/i;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const HOME = homedir();

const mainRoot = gitMainRoot();
const stateDir = join(mainRoot, '.muse/session-log');
const DEFAULT_OUT = join(mainRoot, 'docs/prompts/sessions');
const REVIEW_OUT = join(stateDir, 'review');
// Set per request (a pending hook request keeps the options it was made
// with; see serviceDirty), so these are not constants.
let SINCE = DEFAULT_SINCE;
let outDir = DEFAULT_OUT;
let workspaceRoot = realpath(mainRoot);

if (args.hook) {
  // Stop / StopFailure / SessionEnd: hand the work to a detached copy and
  // return at once, so a large session never delays the end of a turn.
  const input = readInput();
  const id = String(input.session_id || process.env.MUSE_SESSION_ID || '');
  const event = typeof args.hook === 'string' ? args.hook : String(input.hook_event_name || 'Stop');
  if (UUID.test(id)) {
    mkdirSync(stateDir, { recursive: true });
    // Muse writes the turn's terminal record only after this hook returns,
    // so the raw log's size now is a safe place to look for it from.
    const pass = ['--session', id, '--from-hook', event, '--log-offset', String(rawLogSize(id))];
    for (const k of ['workspace', 'out', 'since']) if (typeof args[k] === 'string') pass.push(`--${k}`, args[k]);
    spawn(process.execPath, ['--no-warnings', SELF, ...pass], { cwd: mainRoot, detached: true, stdio: 'ignore' }).unref();
  }
  process.exit(0);
}

const index = openIndex();
const names = new Map(); // session id -> name, filled as exports are read
const rowCache = new Map();
let scrubber;

async function main() {
  const opts = requestOptions({ out: args.out, since: args.since, workspace: args.workspace });
  setOptions(opts);
  if (args.audit) return audit();
  if (args['from-hook']) {
    try {
      setPriority(19);
    } catch {
      // not permitted: run at normal priority
    }
    if (args['from-hook'] !== 'SessionEnd') await waitForTurnEnd(String(args.session), Number(args['log-offset']));
  }
  restorePending();
  if (args.export) {
    await withLockWait(() => processTargets([{ file: resolve(String(args.export)) }]));
    serviceDirty();
    return undefined;
  }
  const wanted = args.session ? [String(args.session)] : ['*'];
  for (const id of wanted) if (id !== '*' && !UUID.test(id)) throw new Error(`not a session id: ${id}`);
  markDirty(wanted, opts);
  if (!serviceDirty() && !args['from-hook']) console.error('session-log: another run holds the lock; it will pick this request up');
  return undefined;
}

// A request's options, made absolute so a request can be serviced later by
// another process: where digests go, from when, whose sessions.
function requestOptions({ out, since, workspace }) {
  const from = typeof since === 'string' ? Date.parse(since) : Number.isFinite(since) ? since : DEFAULT_SINCE;
  if (!Number.isFinite(from)) throw new Error(`--since is not a date: ${since}`);
  const dir = typeof out === 'string' && out ? resolve(out) : from < DEFAULT_SINCE ? REVIEW_OUT : DEFAULT_OUT;
  return { out: dir, since: from, workspace: realpath(typeof workspace === 'string' && workspace ? resolve(workspace) : mainRoot) };
}

function setOptions(o) {
  SINCE = o.since;
  outDir = o.out;
  workspaceRoot = o.workspace;
}

// ------------------------------------------------------------- lock + dirty
// One generator at a time per main checkout. A request first appends its
// session ids, each with the options it was made with, to `dirty`, then
// tries the lock: the holder keeps taking `dirty` until it is empty
// (servicing each request with its own options, so a manual run with --out
// never takes a hook's request into its test folder), releases, and looks
// once more; a request that finds the lock taken just leaves its entries
// for the holder. A request that fails as a whole (the scrub map missing,
// say) goes to `pending` and is retried by the next run. No turn is lost.

function serviceDirty() {
  let held = false;
  for (let round = 0; round < 20 && existsSync(join(stateDir, 'dirty')); round += 1) {
    if (!tryLock()) return held;
    held = true;
    try {
      for (let pass = 0; pass < 5; pass += 1) {
        const groups = takeDirty();
        if (!groups.size) break;
        for (const { opts, ids } of groups.values()) {
          setOptions(opts);
          try {
            processTargets(targetsFor(ids));
          } catch (err) {
            logError('run', err);
            appendFileSync(join(stateDir, 'pending'), dirtyLines([...ids], opts));
            process.exitCode = 1;
          }
        }
      }
    } finally {
      unlock();
    }
  }
  return held;
}

function restorePending() {
  const file = join(stateDir, 'pending');
  const mine = `${file}.${process.pid}`;
  try {
    renameSync(file, mine);
  } catch {
    return;
  }
  appendFileSync(join(stateDir, 'dirty'), readFileSync(mine, 'utf8'));
  unlinkSync(mine);
}

async function withLockWait(fn) {
  const deadline = Date.now() + 10 * 60_000;
  while (!tryLock()) {
    if (Date.now() > deadline) throw new Error('lock held for 10 minutes');
    await sleep(1000);
  }
  try {
    fn();
  } finally {
    unlock();
  }
}

function markDirty(ids, opts) {
  appendFileSync(join(stateDir, 'dirty'), dirtyLines(ids, opts));
}

// One line per request: id <TAB> out dir <TAB> since (ms) <TAB> workspace.
function dirtyLines(ids, opts) {
  return ids.map((i) => `${i}\t${opts.out}\t${opts.since}\t${opts.workspace}\n`).join('');
}

// The pending requests, grouped by their options.
function takeDirty() {
  const file = join(stateDir, 'dirty');
  const mine = `${file}.${process.pid}`;
  const groups = new Map();
  try {
    renameSync(file, mine);
  } catch {
    return groups;
  }
  for (const line of readFileSync(mine, 'utf8').split('\n')) {
    const [id, out, since, workspace] = line.trim().split('\t');
    if (id !== '*' && !UUID.test(id ?? '')) continue;
    // A line without options (written by an older copy) means the defaults.
    const opts = out ? { out, since: Number(since), workspace } : requestOptions({});
    const key = `${opts.out}\t${opts.since}\t${opts.workspace}`;
    if (!groups.has(key)) groups.set(key, { opts, ids: new Set() });
    groups.get(key).ids.add(id);
  }
  unlinkSync(mine);
  return groups;
}

function tryLock() {
  const dir = join(stateDir, 'lock');
  for (let attempt = 0; attempt < 2; attempt += 1) {
    try {
      mkdirSync(dir);
      writeFileSync(join(dir, 'owner'), `${process.pid} ${Date.now()}\n`);
      return true;
    } catch (err) {
      if (err.code !== 'EEXIST') throw err;
    }
    let pid = 0;
    let since = 0;
    try {
      [pid, since] = readFileSync(join(dir, 'owner'), 'utf8').trim().split(' ').map(Number);
    } catch {
      since = statSafe(dir)?.mtimeMs ?? 0; // owner not written yet, or lost
    }
    if (alive(pid) && Date.now() - since < 30 * 60_000) return false;
    if (!pid && Date.now() - since < 60_000) return false;
    rmSync(dir, { recursive: true, force: true }); // stale: holder died or hung
  }
  return false;
}

function unlock() {
  rmSync(join(stateDir, 'lock'), { recursive: true, force: true });
}

function alive(pid) {
  if (!pid) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === 'EPERM';
  }
}

// Stop and StopFailure fire before Muse writes the turn's terminal record
// and its closing HEAD observation; wait for them (read only the new tail of
// the raw log, for that one record kind) so the digest has the whole turn.
async function waitForTurnEnd(id, offset) {
  const path = indexRow(id)?.session_log_path;
  if (!path || !existsSync(path) || !(offset >= 0)) return sleep(15_000);
  const from = offset;
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    await sleep(2000);
    if (/"event":\{"kind":"terminal"/.test(readTail(path, from, 8 << 20))) return sleep(1500);
  }
  return undefined;
}

// ------------------------------------------------------------------ targets

function targetsFor(ids) {
  const targets = ids.has('*') ? repoSessions() : [];
  for (const id of ids) if (id !== '*' && !targets.some((t) => t.id === id)) targets.push({ id });
  return targets;
}

function processTargets(targets) {
  mkdirSync(outDir, { recursive: true });
  cleanTemps();
  scrubber = makeScrubber();
  // Parents first, so a side chat can name its parent from this run.
  const ordered = [...targets].sort((a, b) => Number(isSideChatRow(a.id)) - Number(isSideChatRow(b.id)));
  for (const t of ordered) {
    try {
      writeDigest(t);
    } catch (err) {
      logError(t.id ?? t.file, err);
      process.exitCode = 1;
    }
  }
  sweepLegacy();
  writeIndex();
}

// A digest written by an older copy of this script (a worktree still on
// v1, say) that is not committed and fails today's check is moved out of
// the repo, to the git-ignored .muse/session-log/quarantine/.
function sweepLegacy() {
  for (const f of digestFiles()) {
    const path = join(outDir, f);
    if (digestMeta(path).v >= 2) continue;
    const problems = scrubber.check(readFileSync(path, 'utf8'));
    if (!problems.length) continue;
    if (tracked(path)) {
      // Reported once per file: it stays until a person edits it.
      const warned = join(stateDir, 'legacy-warned');
      const seen = existsSync(warned) ? readFileSync(warned, 'utf8').split('\n') : [];
      if (!seen.includes(f)) {
        appendFileSync(warned, `${f}\n`);
        logError(f, `committed digest from an older session-log fails the privacy check (${problems.join('; ')}); left in place, review it by hand`);
      }
      continue;
    }
    const dir = join(stateDir, 'quarantine');
    mkdirSync(dir, { recursive: true });
    renameSync(path, join(dir, f));
    logError(f, `uncommitted digest from an older session-log failed the privacy check (${problems.join('; ')}); moved to .muse/session-log/quarantine/`);
  }
}

function tracked(path) {
  try {
    execFileSync('git', ['-C', dirname(path), 'ls-files', '--error-unmatch', basename(path)], { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
}

// Sessions belong to this repo when their workspace is the checkout or a
// worktree inside it (subagents, the judge's worktree). A side chat has no
// workspace in the index; it belongs here when its parent does.
function repoSessions() {
  if (!index) return [];
  const rows = index
    .prepare(`select session_id, session_name, workspace_root, session_log_path, msp_fork_source_session_id,
                     max(coalesce(created_at_us, 0), coalesce(updated_at_us, 0), coalesce(msp_last_activity_at_us, 0)) as active_us
              from sessions where coalesce(msp_kind, 'root') = 'root' order by created_at_us`)
    .all();
  const inRepo = (root) => root && (realpath(root) + '/').startsWith(workspaceRoot + '/');
  const repoIds = new Set(rows.filter((r) => inRepo(r.workspace_root)).map((r) => r.session_id));
  const out = [];
  for (const r of rows) {
    if (r.active_us / 1000 < SINCE) continue;
    if (repoIds.has(r.session_id)) out.push({ id: r.session_id });
    else if (!r.workspace_root) {
      const parent = r.msp_fork_source_session_id || parentFromLog(r.session_log_path);
      if (parent && repoIds.has(parent)) out.push({ id: r.session_id });
    }
  }
  return out;
}

function isSideChatRow(id) {
  if (!id) return false;
  const row = indexRow(id);
  return Boolean(row && !row.workspace_root);
}

// ------------------------------------------------------------------- digest

function writeDigest(target) {
  if (target.id && skipList().has(target.id)) return dropDigest(target.id, 'listed in .muse/session-log.skip');
  const pre = target.id ? indexRow(target.id) : null;
  if (pre?.session_name && SKIP_NAME.test(pre.session_name)) return dropDigest(target.id, `named ${pre.session_name}`);

  const s = target.file ? readExport(target.file) : withExport(target.id, readExport);
  if (!s.id) throw new Error('export names no session');
  const row = indexRow(s.id);
  s.name = s.name ?? row?.session_name ?? row?.logged_session_name ?? null;
  if (s.name) names.set(s.id, s.name);
  if (skipList().has(s.id)) return dropDigest(s.id, 'listed in .muse/session-log.skip');
  if (s.name && SKIP_NAME.test(s.name)) return dropDigest(s.id, `named ${s.name}`);
  s.parentId = s.sideOf ?? s.forkOf ?? s.exportParent ?? null;
  s.parentName = s.parentId ? nameOf(s.parentId) : null;
  if (s.parentId && (skipList().has(s.parentId) || (s.parentName && SKIP_NAME.test(s.parentName)))) {
    return dropDigest(s.id, `side chat or fork of a skipped session`);
  }
  s.workspace = s.workspace ?? s.heads.find((h) => h.root)?.root ?? row?.workspace_root ?? null;

  const all = finishRuns(s);
  const runs = all.filter((r) => (r.endAt ?? r.lastAt) >= SINCE);
  if (!runs.length) return undefined;
  attachCommits(s, all);

  const doc = renderDigest(s, all, runs);
  const problems = scrubber.check(doc);
  if (problems.length) throw new Error(`refused ${s.id}: privacy check found ${problems.join('; ')}; digest not written`);
  const label = s.name ?? (s.parentName ? `${s.parentName}-side` : 'session');
  const file = join(outDir, `${iso(runs[0].start).slice(0, 10)}-${slug(scrubber.line(label))}-${s.id}.md`);
  writeAtomic(file, doc);
  for (const old of digestsFor(s.id)) if (old !== file) unlinkSync(old);
  return file;
}

function dropDigest(id, why) {
  for (const f of digestsFor(id)) {
    unlinkSync(f);
    logError(id, `skipped (${why}); removed ${basename(f)}`);
  }
  return undefined;
}

function finishRuns(s) {
  const list = [...s.runs.values()].filter((r) => r.started).sort((a, b) => a.start - b.start);
  for (const [i, r] of list.entries()) {
    r.n = i + 1;
    r.last = i === list.length - 1;
    r.items.sort((a, b) => a.at - b.at);
    const wake = r.items.filter((it) => it.at - r.start <= 3000);
    const prompt = r.prompt.trim();
    const control = r.items.find((it) => it.source === 'goal_control' && /changed by user request/.test(it.text));
    const wakeSources = [...new Set(wake.map((it) => it.source))];
    if (s.sideOf && prompt) [r.kind, r.label] = ['you', `side chat of ${s.parentName ?? 'another session'} (typed by the operator)`];
    else if (control && !prompt) [r.kind, r.label] = ['you', `you: /goal ${control.action ?? 'change'}`];
    else if (prompt.startsWith(GOAL_STEP)) [r.kind, r.label] = ['goal', 'goal: continue'];
    else if (prompt && (s.intents.has(r.id) || prompt.startsWith('/'))) [r.kind, r.label] = ['you', 'you'];
    else if (prompt) [r.kind, r.label] = ['other', 'prompt from outside this session (origin not recorded)'];
    else if (wakeSources.includes('user_steer')) [r.kind, r.label] = ['you', 'you (typed while Muse was busy)'];
    else if (wakeSources.includes('session_message')) {
      const from = [...new Set(wake.filter((it) => it.source === 'session_message').map((it) => it.sender ?? 'another session'))];
      [r.kind, r.label] = ['message', `message from ${from.join(', ')}`];
    } else if (wakeSources.includes('scheduled')) {
      const job = wake.find((it) => it.source === 'scheduled');
      [r.kind, r.label] = ['scheduled', `scheduled: ${job.display || job.prompt || job.summary}`];
    } else if (wakeSources.some((src) => /^goal/.test(src))) [r.kind, r.label] = ['goal', 'goal'];
    else if (wakeSources.length) [r.kind, r.label] = ['auto', `woken by ${wakeSources.map(describeSource).join(', ')}`];
    else [r.kind, r.label] = ['auto', 'automatic'];
  }
  return list;
}

// Commits per turn from Muse's HEAD observations: the HEAD when the turn
// started and when it ended, then `git log --first-parent start..end`. A
// commit is the turn's own when its committer time falls inside the turn
// AND the turn ran a history-writing git command (or got subagent
// results). Older commits in the range arrived by merge, rebase or
// fast-forward (the judge's worktree rebased onto the builder's main, say)
// and are only counted; in-window commits from a turn that ran no git
// command are listed apart, as made by someone else. In a worktree session
// (the judge), a commit that was already on the main checkout's branch when
// the turn ended was made there (by the builder) and reached this branch by
// rebase or merge, whatever its time: it is counted, never credited here.
function attachCommits(s, runs) {
  const heads = s.heads.slice().sort((a, b) => a.at - b.at);
  const dirs = [...new Set([mainRoot, s.workspace].filter((d) => d && existsSync(d)))];
  const mainline = s.workspace && existsSync(s.workspace) ? mainBranchOf(s.workspace) : null;
  const seen = new Set();
  for (const [i, r] of runs.entries()) {
    r.commits = [];
    r.foreign = 0;
    r.unclaimed = [];
    // A turn that ran no git command and heard from no subagent made no
    // commit, whatever landed in its checkout meanwhile (the operator's own
    // commit in the main checkout, say).
    const mayCommit = r.gitWrites > 0 || r.items.some((it) => it.source === 'subagent_result');
    const end = r.endAt ?? r.lastAt;
    const own = heads.filter((h) => h.run === r.id);
    const startObs = own[0] && own[0].at <= r.start + 2000 ? own[0] : heads.filter((h) => h.at <= r.start).at(-1);
    const nextStart = runs[i + 1]?.start ?? Infinity;
    const endObs = own.length && own.at(-1) !== startObs ? own.at(-1) : heads.find((h) => h.at >= end - 1000 && h.at <= nextStart + 2000 && h !== startObs);
    if (!startObs || !endObs || startObs.commit === endObs.commit) continue;
    const range = `${startObs.commit}..${endObs.commit}`;
    const log = gitLog(dirs, range);
    if (log === null) {
      r.rangeMissing = `${startObs.commit.slice(0, 9)}..${endObs.commit.slice(0, 9)}`;
      continue;
    }
    const tip = mainline ? mainline.tipAt(end + 2000) : null;
    const offMain = tip ? new Set((gitLog(dirs, range, tip) ?? []).map((c) => c.hash)) : null;
    for (const { hash, time, subject } of log) {
      if (seen.has(hash)) continue;
      seen.add(hash);
      if (time < r.start - 2000 || time > end + 2000 || (offMain && !offMain.has(hash))) r.foreign += 1;
      else if (mayCommit) r.commits.push({ hash, subject });
      else r.unclaimed.push({ hash, subject });
    }
  }
}

function gitLog(dirs, range, not = null) {
  for (const dir of dirs) {
    try {
      const argv = ['-C', dir, 'log', '--first-parent', '--format=%h%x09%ct%x09%s', range, ...(not ? ['--not', not] : [])];
      const out = execFileSync('git', argv, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 16 << 20 });
      return out.trim().split('\n').filter(Boolean).reverse().map((l) => {
        const [hash, ct, ...rest] = l.split('\t');
        return { hash, time: Number(ct) * 1000, subject: rest.join('\t') };
      });
    } catch {
      // range unknown here; try the session's own workspace
    }
  }
  return null;
}

// For a session in a linked worktree: the branch checked out in the main
// checkout of the same repository, and where its tip was at a given time
// (from that branch's reflog; its oldest entry or the current tip when the
// reflog does not reach back that far). Null for the main checkout itself.
function mainBranchOf(workspace) {
  const git = (dir, ...a) => execFileSync('git', ['-C', dir, ...a], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  try {
    const main = dirname(git(workspace, 'rev-parse', '--path-format=absolute', '--git-common-dir'));
    if (realpath(main) === realpath(git(workspace, 'rev-parse', '--show-toplevel'))) return null;
    const ref = git(main, 'symbolic-ref', '-q', 'HEAD');
    const current = git(main, 'rev-parse', ref);
    let moves = [];
    try {
      moves = git(main, 'log', '-g', '--date=unix', '--format=%H %gd', ref).split('\n').map((l) => {
        const m = l.match(/^([0-9a-f]{40}) .*@\{(\d+)\}$/);
        return m ? { commit: m[1], at: Number(m[2]) * 1000 } : null;
      }).filter(Boolean); // newest first
    } catch {
      // no reflog: the current tip stands in
    }
    return { tipAt: (t) => moves.find((m) => m.at <= t)?.commit ?? moves.at(-1)?.commit ?? current };
  } catch {
    return null;
  }
}

// ----------------------------------------------------------------- render

function renderDigest(s, all, runs) {
  const sc = scrubber;
  const counts = { you: 0, goal: 0, other: 0 };
  for (const r of runs) counts[r.kind === 'you' ? 'you' : r.kind === 'goal' ? 'goal' : 'other'] += 1;
  const commits = runs.reduce((n, r) => n + r.commits.length, 0);
  const start = runs[0].start;
  const end = Math.max(...runs.map((r) => r.endAt ?? r.lastAt));
  const rel = s.workspace ? relative(workspaceRoot, realpath(s.workspace)) : null;
  const inside = rel !== null && !rel.startsWith('..');
  const kind = s.sideOf ? 'side chat' : s.forkOf ? 'fork' : inside && rel ? 'worktree' : 'session';
  const title = s.name ?? (s.parentName ? `${s.parentName} side chat` : 'Session');
  const where = rel === null ? 'unknown' : inside ? rel || 'main checkout' : s.workspace;
  const meta = {
    v: 2, id: s.id, name: s.name, kind, parent: s.parentId, parent_name: s.parentName,
    start: iso(start), end: iso(end), runs: runs.length, ...counts, commits,
  };

  const out = [];
  out.push(`# ${sc.line(title)} · Muse Code session ${s.id}`, '');
  out.push(`<!-- session-log: ${sc.line(JSON.stringify(meta))} -->`);
  out.push(`- ${iso(start)} → ${iso(end)} UTC · ${plural(runs.length, 'turn')} (${counts.you} typed by the operator, ${counts.goal} driven by the goal, ${counts.other} other) · ${plural(commits, 'commit')}`);
  if (s.sideOf) out.push(`- Side chat (\`/side\`) of **${sc.line(s.parentName ?? 'another session')}**, session \`${s.parentId}\`. Its prompts are typed by the operator; it starts from the parent's history, which is not repeated here.`);
  else if (s.forkOf) out.push(`- Fork of **${sc.line(s.parentName ?? 'another session')}**, session \`${s.parentId}\`.`);
  out.push(`- Workspace: ${sc.line(where)} · model ${sc.line(s.model ?? '?')}`);
  if (runs.length < all.length) out.push(`- ${plural(all.length - runs.length, 'earlier turn')}, before ${new Date(SINCE).toISOString().slice(0, 16).replace('T', ' ')} UTC, are left out; turn numbers count from the session's first turn.`);
  out.push(`- Generated by \`.muse/hooks/session-log.mjs\` from Muse's redacted session log (\`muse export --redacted\` of the session's prompt, answer, message and question records). Raw tool calls and output are left out; the model's answers can still quote what tools returned. Pasted terminal output, host and user names, home paths, addresses, sudo rules and key-shaped strings are scrubbed; "[redacted by Muse]" marks text Muse's own redaction removed.`);

  for (const r of runs) {
    const endAt = r.endAt ?? r.lastAt;
    const status = r.terminal === 'completed' ? ''
      : r.terminal ? ` · ${r.terminal}${r.reason ? `: ${sc.line(cap(r.reason, 200))}` : ''}`
        : r.last ? ' · still running when this was written' : ' · no end recorded';
    out.push('', `## ${r.n}. ${iso(r.start)} UTC · ${sc.line(r.label)} · ${duration(endAt - r.start)}${status}`);

    const entries = [];
    const prompt = r.prompt.trim();
    if (prompt && r.kind !== 'goal') {
      const heading = r.kind === 'other' ? '**Prompt (not typed in this session)**' : '**Prompt**';
      entries.push({ at: r.start, text: [heading, '', quote(capped(sc.text(prompt, { terminal: true, display: r.display }), CAP.prompt))] });
    }
    let nudges = 0;
    const subagents = [];
    for (const it of r.items) {
      if (it.source === 'goal_progress_nudge') {
        nudges += 1;
        continue;
      }
      if (it.source === 'subagent_result') {
        subagents.push(it);
        continue;
      }
      const block = renderItem(s, it);
      if (block) entries.push({ at: it.at, text: block });
    }
    if (subagents.length) {
      const lines = [`**Subagent results (${subagents.length})**`, ''];
      for (const it of subagents) {
        const spawn = s.spawns.get(it.subagent);
        const who = spawn ? `**${sc.line(spawn.title)}**${spawn.role ? ` (${sc.line(spawn.role)})` : ''}` : '**subagent**';
        lines.push(`- ${who} · ${hms(it.at)} UTC`);
        const summary = capped(sc.text(it.summary || it.body, { terminal: true }), CAP.message);
        if (summary) lines.push(quote(summary, '  > '));
      }
      entries.push({ at: subagents[0].at, text: lines });
    }
    for (const q of r.questions) entries.push({ at: q.at, text: renderQuestions(q) });
    for (const f of r.finals) {
      entries.push({ at: f.at, text: [`**Response · ${hms(f.at)} UTC**`, '', demoteHeadings(capped(sc.text(f.text), CAP.answer))] });
    }
    entries.sort((a, b) => a.at - b.at);
    for (const e of entries) out.push('', ...e.text);
    if (!r.finals.length && r.note) {
      out.push('', `**Last progress note (no final answer) · ${hms(r.note.at)} UTC**`, '', demoteHeadings(capped(sc.text(r.note.text), CAP.answer)));
    } else if (!r.finals.length) out.push('', '_No message from Muse in this turn._');
    if (nudges) out.push('', `_Goal progress nudges during this turn: ${nudges}._`);
    if (r.commits.length) out.push('', '**Commits**', '', ...r.commits.map((c) => `- \`${c.hash}\` ${sc.line(c.subject)}`));
    if (r.unclaimed.length) {
      out.push('', '**Also committed in this checkout during the turn, by no git command of this session** (by hand or by another session)', '', ...r.unclaimed.map((c) => `- \`${c.hash}\` ${sc.line(c.subject)}`));
    }
    if (r.foreign) out.push('', `_HEAD also moved over ${r.foreign} commit${r.foreign === 1 ? '' : 's'} made outside this turn (merge, rebase or fast-forward); they are credited where they were made._`);
    if (r.rangeMissing) out.push('', `_Commit range ${r.rangeMissing} is not in this checkout; commits not listed._`);
  }
  return out.join('\n') + '\n';
}

function renderItem(s, it) {
  const sc = scrubber;
  const at = `${hms(it.at)} UTC`;
  const msg = (text, terminal = true) => quote(capped(sc.text(text, { terminal }), CAP.message));
  switch (it.source) {
    case 'goal_control': {
      const objective = it.text.match(/<objective>([\s\S]*?)<\/objective>/)?.[1]?.trim();
      const byYou = /changed by user request/.test(it.text);
      const head = `**Goal ${byYou ? `${it.action ?? 'changed'} by you` : it.action ?? 'update'} · ${at}**`;
      return objective ? [head, '', msg(unescapeXml(objective))] : [head];
    }
    case 'session_message':
      return [`**Message from ${sc.line(it.sender ?? 'another session')} · ${at}**`, '', msg(it.body || it.summary)];
    case 'user_steer':
      return [`**You, while Muse was working · ${at}**`, '', msg(it.prompt || it.body || it.summary)];
    case 'scheduled':
      return [`**Scheduled job ${sc.line(it.job ?? '')} fired · ${at}:** \`${sc.line(cap(it.display || it.prompt || it.body || it.summary, CAP.line)).replace(/`/g, "'")}\``];
    case 'monitor_event':
      return [`**Monitor event · ${at}:** ${sc.line(cap(firstLine(it.summary), CAP.line))}`, ...(it.body ? ['', msg(it.body)] : [])];
    case 'subagent_input': {
      const spawn = s.spawns.get(it.subagent);
      return [`**Sent to subagent ${spawn ? sc.line(spawn.title) : ''} · ${at}**`, '', msg(it.body || it.summary)];
    }
    case 'background_task_terminal': {
      const label = it.body.match(/^display_label: (.*)$/m)?.[1] ?? '';
      const code = it.body.match(/^exit_code: (.*)$/m)?.[1];
      return [`**Background task ${it.terminalKind ?? 'finished'}${label ? `: ${sc.line(cap(label, CAP.line))}` : ''}${code ? ` (exit ${sc.line(code)})` : ''} · ${at}**`];
    }
    case 'background_task_overdue':
      return [`**${sc.line(cap(firstLine(it.summary) || 'background task still running', CAP.line))} · ${at}**`];
    default:
      return [`**${describeSource(it.source)} · ${at}:** ${sc.line(cap(firstLine(it.summary), CAP.line))}`];
  }
}

function renderQuestions(q) {
  const sc = scrubber;
  const qs = q.questions ?? [];
  const lines = [`**Muse asked ${qs.length === 1 ? 'a question' : `${qs.length} questions`} · ${hms(q.at)} UTC**`, ''];
  for (const item of qs) {
    lines.push(`- **${sc.line(item.header || item.id || 'Question')}**`);
    if (item.question) lines.push(quote(capped(sc.text(item.question), CAP.message), '  > '));
    const options = (item.options ?? []).map((o) => sc.line(o.label ?? String(o))).filter(Boolean);
    if (options.length) lines.push(`  Options: ${options.join(' · ')}`);
    lines.push(`  ${renderAnswer(q, item)}`);
  }
  return lines;
}

function renderAnswer(q, item) {
  const sc = scrubber;
  if (!q.outcome) return '**Answer:** none recorded (the question was still open)';
  if (q.outcome !== 'answered') return `**Answer:** none (${sc.line(q.outcome)}${q.settledAt ? `, ${hms(q.settledAt)} UTC` : ''})`;
  const a = (q.answers ?? []).find((x) => x.id === item.id) ?? ((q.answers ?? []).length === 1 && (q.questions ?? []).length === 1 ? q.answers[0] : null);
  if (!a) return '**Answer:** none for this question';
  const parts = [];
  if (a.selected_label) parts.push(`selected "${sc.line(a.selected_label)}"`);
  if (a.selected_labels?.length) parts.push(`selected ${a.selected_labels.map((l) => `"${sc.line(l)}"`).join(', ')}`);
  for (const [k, label] of [['free_text', 'free text'], ['text', 'text'], ['note', 'note']]) {
    if (a[k]) parts.push(`${label}: "${capped(sc.text(String(a[k]), { terminal: true }), CAP.message).replace(/\n+/g, ' / ')}"`);
  }
  return `**You answered · ${hms(q.settledAt)} UTC:** ${parts.join('; ') || '(empty answer)'}`;
}

// ------------------------------------------------------------------- index

function writeIndex() {
  const rows = digestFiles()
    .map((f) => ({ f, ...digestMeta(join(outDir, f)) }))
    .filter((r) => r.id)
    .sort((a, b) => String(a.start).localeCompare(String(b.start)) || a.f.localeCompare(b.f));
  const total = (k) => rows.reduce((n, r) => n + (r[k] ?? 0), 0);
  const firstDay = rows.length ? String(rows[0].start).slice(0, 10) : '';
  const lastDay = rows.length ? rows.map((r) => String(r.end).slice(0, 10)).sort().at(-1) : '';
  const heading = !rows.length ? 'Muse Code sessions' : firstDay === lastDay ? `Muse Code sessions, ${firstDay}` : `Muse Code sessions, ${firstDay} to ${lastDay}`;
  const out = [
    `# ${heading}`,
    '',
    `Every Muse Code session in this repo${firstDay ? ` since ${firstDay}` : ''} has one file here, written after each turn by \`.muse/hooks/session-log.mjs\` from Muse's own redacted session log: the builder, the judge, their side chats and any other session. Earlier prompts are the per-prompt files in \`docs/prompts/\`.`,
    '',
    `${rows.length} sessions, ${total('runs')} turns (${total('you')} typed by the operator, ${total('goal')} driven by the goal, ${total('runs') - total('you') - total('goal')} other), ${total('commits')} commits.`,
    '',
    '| Session | Kind | Start (UTC) | End (UTC) | Turns | Operator | Goal | Other | Commits | Session id |',
    '|---|---|---|---|---|---|---|---|---|---|',
    ...rows.map((r) => {
      const name = r.name ?? (r.parent_name ? `${r.parent_name} side chat` : 'session');
      const kind = r.kind ? (r.parent_name ? `${r.kind} of ${r.parent_name}` : r.kind) : 'session';
      const other = (r.runs ?? 0) - (r.you ?? 0) - (r.goal ?? 0);
      return `| [${cell(name)}](${r.f}) | ${cell(kind)} | ${r.start} | ${r.end} | ${r.runs} | ${r.you ?? 0} | ${r.goal ?? 0} | ${other} | ${r.commits} | \`${r.id}\` |`;
    }),
  ];
  const doc = out.join('\n') + '\n';
  const problems = scrubber.check(doc);
  if (problems.length) throw new Error(`refused README.md: privacy check found ${problems.join('; ')}`);
  writeAtomic(join(outDir, 'README.md'), doc);
}

function cell(s) {
  return String(s).replace(/\|/g, '\\|');
}

// --audit: which tracked files outside docs/prompts/sessions already hold a
// scrub-map literal (the digest scrub cannot hide what the tree publishes).
// Prints replacement names and counts, never the literals.
function audit() {
  const literals = loadLiterals();
  const hits = new Map();
  for (const l of literals) {
    let out = '';
    try {
      const argv = ['-C', mainRoot, 'grep', '-I', '-i', '-c', '-F', ...(l.word ? ['-w'] : []), '-e', l.from, '--', '.', ':!docs/prompts/sessions'];
      out = execFileSync('git', argv, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], maxBuffer: 16 << 20 });
    } catch {
      continue; // exit 1: no match
    }
    for (const row of out.trim().split('\n')) {
      const at = row.lastIndexOf(':');
      const file = row.slice(0, at);
      const m = hits.get(file) ?? new Map();
      m.set(l.to, (m.get(l.to) ?? 0) + Number(row.slice(at + 1)));
      hits.set(file, m);
    }
  }
  if (!hits.size) {
    console.log('No tracked file outside docs/prompts/sessions holds a scrub-map literal.');
    return;
  }
  console.log(`${hits.size} tracked file(s) hold scrub-map literals (shown by replacement; git history keeps every earlier version):`);
  for (const [file, m] of [...hits].sort()) console.log(`  ${file}: ${[...m].map(([to, n]) => `${to} x${n}`).join(', ')}`);
  process.exitCode = 2;
}

// --------------------------------------------------------- export reading

function withExport(id, fn) {
  if (!UUID.test(id)) throw new Error(`not a session id: ${id}`);
  const dir = mkdtempSync(join(tmpdir(), 'session-log-'));
  try {
    const file = join(dir, 'export.json');
    const raw = indexRow(id)?.session_log_path;
    let source = id;
    let gitWrites = null;
    if (raw && existsSync(raw)) {
      mkdirSync(join(dir, id));
      source = join(dir, id, 'session.jsonl');
      gitWrites = trimRawLog(raw, source);
    }
    try {
      execFileSync('muse', ['export', '--session', source, '--redacted', '--out', file], { stdio: ['ignore', 'ignore', 'pipe'], timeout: 600_000, maxBuffer: 4 << 20 });
    } catch (err) {
      throw new Error(`muse export failed: ${String(err.stderr || err.message).trim().split('\n').at(-1)}`);
    }
    const s = fn(file);
    if (s.id !== id) throw new Error(`the export of ${id} names session ${s.id}`);
    if (gitWrites) for (const r of s.runs.values()) r.gitWrites = gitWrites.get(r.id) ?? 0;
    return s;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

// The raw-log records a digest reads. Everything else (tool calls and their
// output, reasoning, model traces, task bookkeeping: 98% of the log) stays
// out of the copy that `muse export --redacted` reads. A line whose type
// cannot be told from its start is kept (Muse redacts it like the rest).
const KEEP_TYPES = new Set([
  'session.started', 'session.fork.created', 'session.side.started', 'runtime.session.metadata', 'run.model.configured',
  'session.name.changed', 'session.workspace_branch.observed', 'runtime.user_intent.accepted',
  'subagent.control.spawn_accepted', // subagent titles (accepted_spawns)
]);
const KEEP_RUN_EVENTS = new Set([
  'started', 'user_prompt_display', 'assistant_message_committed', 'inbox_item_drained',
  'user_input_prompt_requested', 'user_input_prompt_settled', 'terminal',
]);

// Copies the kept lines of `src` to `dst` and returns, per run, how many
// history-writing git commands its tool calls held (read raw; only the
// count leaves this function).
function trimRawLog(src, dst) {
  const out = openSync(dst, 'w');
  const gitWrites = new Map();
  const keep = (bytes) => {
    const head = bytes.subarray(0, 4096).toString('utf8');
    if (head.startsWith('{"retained_frame"')) return false; // permission transactions
    let parsed;
    const line = () => parsed ?? (parsed = JSON.parse(bytes.toString('utf8')));
    const type = head.match(/^\{[^]*?"payload_type":"([\w.]+)"/)?.[1];
    if (!type) return true;
    if (KEEP_TYPES.has(type)) return true;
    if (type !== 'runtime.session') return false;
    let kind = head.match(/"payload":\{"kind":"(\w+)"/)?.[1];
    let event = head.match(/"event":\{"kind":"(\w+)"/)?.[1];
    if (!kind || (kind === 'run' && !event)) {
      try {
        kind = line().payload?.kind;
        event = line().payload?.event?.kind;
      } catch {
        return true; // a torn line: let Muse decide
      }
    }
    if (kind === 'subagent_control') return true; // spawn records: subagent titles
    if (kind !== 'run') return false;
    if (event === 'assistant_tool_calls_committed') {
      try {
        const p = line().payload;
        const n = (p.event.tool_calls ?? []).filter((c) => GIT_WRITE.test(String(c.args ?? ''))).length;
        if (n) gitWrites.set(p.run_id, (gitWrites.get(p.run_id) ?? 0) + n);
      } catch {
        // a torn line
      }
      return false;
    }
    return KEEP_RUN_EVENTS.has(event);
  };
  const NL = Buffer.from('\n');
  try {
    eachLineBytes(src, (bytes) => {
      if (bytes.length && keep(bytes)) {
        writeSync(out, bytes);
        writeSync(out, NL);
      }
    });
  } finally {
    closeSync(out);
  }
  return gitWrites;
}

function readExport(file) {
  const s = {
    id: null, name: null, workspace: null, model: null, sideOf: null, forkOf: null, exportParent: null,
    spawns: new Map(), intents: new Set(), heads: [], runs: new Map(),
  };
  const questions = new Map();
  const run = (id) => {
    let r = s.runs.get(id);
    if (!r) {
      r = { id, started: false, start: 0, lastAt: 0, prompt: '', display: null, finals: [], note: null, items: [], questions: [], terminal: null, reason: null, endAt: null, gitWrites: 0 };
      s.runs.set(id, r);
    }
    return r;
  };
  const top = streamExport(file, (e) => {
    if (e.kind === 'unparseable_line') {
      // Muse 1.4.14 cannot decode its own side-chat records; read only the ids.
      const raw = String(e.raw_text ?? '');
      const type = raw.match(/"record_type":"(session\.side\.started|session\.fork\.created)"/)?.[1];
      const src = raw.match(/"source_session_id":"([0-9a-f-]{36})"/)?.[1];
      if (type === 'session.side.started' && src) s.sideOf = src;
      if (type === 'session.fork.created' && src) s.forkOf = src;
      return;
    }
    const env = e.envelope;
    if (!env?.payload_type) return;
    const p = env.payload ?? {};
    const at = Math.round((env.recorded_at ?? 0) / 1000);
    switch (env.payload_type) {
      case 'session.fork.created':
        s.forkOf = p.source_session_id ?? s.forkOf;
        return;
      case 'session.side.started':
        s.sideOf = p.source_session_id ?? s.sideOf;
        return;
      case 'runtime.session.metadata':
        s.workspace = p.record?.workspace_root ?? s.workspace;
        s.model = p.record?.model_id ?? s.model;
        return;
      case 'run.model.configured':
        s.model = s.model ?? p.record?.model_id ?? null;
        return;
      case 'session.name.changed':
        if (p.new_name) s.name = p.new_name;
        return;
      case 'session.workspace_branch.observed':
        if (p.record?.commit) s.heads.push({ at, run: p.record.command_id, commit: p.record.commit, root: p.record.workspace_root });
        return;
      case 'runtime.user_intent.accepted':
        if (p.intent_id) s.intents.add(p.intent_id);
        return;
      default:
    }
    if (p.kind !== 'run' || !p.run_id) return;
    const ev = p.event ?? {};
    const r = run(p.run_id);
    if (at > r.lastAt) r.lastAt = at;
    switch (ev.kind) {
      case 'started':
        r.started = true;
        r.start = at;
        r.prompt = ev.prompt ?? '';
        break;
      case 'user_prompt_display':
        r.display = ev.prompt ?? null;
        break;
      case 'assistant_message_committed': {
        const text = ev.text ?? '';
        if (!text.trim()) break;
        if (ev.phase === 'commentary') r.note = { at, text };
        else r.finals.push({ at, text });
        break;
      }
      case 'inbox_item_drained': {
        // An item that wakes a turn is QUEUED under the previous run; only
        // the drain record says which run consumed it.
        const snap = ev.delivery_snapshot;
        if (!snap || ev.delivery_visible === false) break;
        const target = run(ev.drain_target_run_stream?.id ?? p.run_id);
        if (at > target.lastAt) target.lastAt = at;
        target.items.push(inboxItem(snap, at));
        break;
      }
      case 'user_input_prompt_requested': {
        const q = { at, questions: ev.questions ?? [], outcome: null, answers: null, settledAt: null };
        questions.set(ev.prompt_id, q);
        run(ev.run_stream?.id ?? p.run_id).questions.push(q);
        break;
      }
      case 'user_input_prompt_settled': {
        const q = questions.get(ev.prompt_id);
        if (q) Object.assign(q, { outcome: ev.outcome ?? 'settled', answers: ev.answers ?? [], settledAt: at });
        break;
      }
      case 'assistant_tool_calls_committed':
        // Only whether the turn ran a history-writing git command is kept.
        for (const call of ev.tool_calls ?? []) if (GIT_WRITE.test(String(call.args ?? ''))) r.gitWrites += 1;
        break;
      case 'terminal':
        r.terminal = ev.terminal ?? 'ended';
        r.reason = ev.reason ?? null;
        r.endAt = at;
        break;
      default:
    }
  });
  if (top.redaction !== 'redacted') throw new Error(`export is not --redacted (redaction: ${top.redaction}); refusing it`);
  const meta = top.sessions?.[0] ?? {};
  s.id = meta.session_id ?? null;
  if (meta.parent?.session_id) s.exportParent = meta.parent.session_id;
  for (const sp of meta.accepted_spawns ?? []) {
    const parts = String(sp.agent_path ?? '').split('/');
    const title = parts.length >= 3 ? parts.slice(1, -1).join('/') : parts.at(-1) || 'subagent';
    s.spawns.set(sp.subagent_id, { title, role: sp.role ?? null });
  }
  return s;
}

function inboxItem(snap, at) {
  const src = snap.source ?? {};
  const pl = snap.payload ?? {};
  const source = src.source ?? 'unknown';
  if (source === 'goal_progress_nudge') return { at, source };
  return {
    at,
    source,
    summary: snap.summary ?? '',
    body: snap.body ?? '',
    prompt: pl.prompt ?? '',
    display: pl.display_prompt ?? '',
    text: source === 'goal_control' ? (pl.model_user_messages ?? []).join('\n') || snap.body || '' : '',
    action: src.action ?? null,
    sender: src.sender_session_name ?? null,
    subagent: src.subagent_id ?? null,
    job: src.job_id ?? null,
    terminalKind: src.terminal_kind ?? null,
  };
}

// Stream a `muse export` document: onEvent gets each element of the
// top-level "events" array; every other top-level member is returned,
// parsed. Only one event is held in memory at a time, so a 400 MB export
// never meets V8's ~512 MB string limit.
function streamExport(file, onEvent) {
  const QUOTE = 0x22, BSL = 0x5c, LB = 0x7b, RB = 0x7d, LK = 0x5b, RK = 0x5d, COLON = 0x3a, COMMA = 0x2c;
  const fd = openSync(file, 'r');
  const buf = Buffer.allocUnsafe(1 << 22);
  const top = {};
  let depth = 0;
  let inStr = false;
  let esc = false;
  let d1 = 'key';
  let key = null;
  let eventsNext = false;
  let inEvents = false;
  let capKind = null;
  let capFrom = 0;
  let parts = [];
  const take = (end) => {
    parts.push(buf.subarray(capFrom, end));
    const text = parts.length === 1 ? parts[0].toString('utf8') : Buffer.concat(parts).toString('utf8');
    parts = [];
    capKind = null;
    return text;
  };
  try {
    for (;;) {
      const n = readSync(fd, buf, 0, buf.length, null);
      if (!n) break;
      let nextQ = -1;
      let nextB = -1;
      if (capKind) capFrom = 0;
      let i = 0;
      while (i < n) {
        if (inStr) {
          if (esc) {
            esc = false;
            i += 1;
            continue;
          }
          if (nextQ < i) nextQ = indexIn(buf, QUOTE, i, n);
          if (nextB < i) nextB = indexIn(buf, BSL, i, n);
          if (nextB < nextQ) {
            esc = true;
            i = nextB + 1;
            continue;
          }
          if (nextQ >= n) {
            i = n;
            continue;
          }
          i = nextQ;
          inStr = false;
          if (capKind === 'key' && depth === 1) {
            key = JSON.parse(take(i + 1));
            d1 = 'colon';
          }
          i += 1;
          continue;
        }
        const c = buf[i];
        if (c === QUOTE) {
          if (depth === 1 && d1 === 'key') {
            capKind = 'key';
            capFrom = i;
          }
          inStr = true;
        } else if (c === LB || c === LK) {
          if (depth === 1 && eventsNext && c === LK) {
            inEvents = true;
            eventsNext = false;
          } else if (depth === 2 && inEvents && !capKind) {
            capKind = 'event';
            capFrom = i;
          }
          depth += 1;
        } else if (c === RB || c === RK) {
          depth -= 1;
          if (depth === 2 && capKind === 'event') onEvent(JSON.parse(take(i + 1)));
          else if (depth === 1 && inEvents) inEvents = false;
          else if (depth === 0 && capKind === 'value') top[key] = JSON.parse(take(i));
        } else if (c === COLON) {
          if (depth === 1 && d1 === 'colon') {
            d1 = 'value';
            if (key === 'events') eventsNext = true;
            else {
              capKind = 'value';
              capFrom = i + 1;
            }
          }
        } else if (c === COMMA && depth === 1) {
          if (capKind === 'value') top[key] = JSON.parse(take(i));
          d1 = 'key';
        }
        i += 1;
      }
      if (capKind) parts.push(Buffer.from(buf.subarray(capFrom, n)));
    }
  } finally {
    closeSync(fd);
  }
  if (depth !== 0 || inStr) throw new Error('export ended mid-document');
  return top;
}

function indexIn(buf, byte, from, n) {
  const at = buf.indexOf(byte, from);
  return at < 0 || at >= n ? n : at;
}

// ------------------------------------------------------------------ scrub

const TERM = /^(\w{3} [ \d]\d \d\d:\d\d:\d\d \S+ \S+\[\d+\]:|[a-z0-9_-]+@[a-z0-9_.-]+:\S*[$#]|(LISTEN|ESTAB|UNCONN)\s|[-dlrwxs]{10}[ .+@]?\s+\d|NEXT\s+LEFT|(Mon|Tue|Wed|Thu|Fri|Sat|Sun) \d{4}-\d\d-\d\d|=== |ABORT: |Created symlink |total \d+$)/;
// Lines that read as command output rather than prose (one token, a path,
// a "--- header:", a prompt, aligned columns, a table): a collapsed paste
// takes these neighbours with it.
const OUTPUTISH = /^(\S+|(---|===|\+\+\+|\$|#|%|sqlite>|mysql>|psql>|>>>|PS>)(\s.*)?|.*\S {2,}\S.*|.*\S\s*\|\s*\S.*)$/;
const PEM = /-----BEGIN [A-Z0-9 ]+-----[\s\S]*?(?:-----END [A-Z0-9 ]+-----|$)/g;
const SHELL_PROMPT = /\b[a-z_][a-z0-9_-]{0,31}@[A-Za-z0-9][A-Za-z0-9.-]*:[~/][^\s#$]*[#$](?=\s|$)/g;
const JOURNAL = /^([ \t>]*)([A-Z][a-z]{2} [ \d]\d \d\d:\d\d:\d\d) \S+ ([\w@.-]+)\[\d+\]:/gm;
const V4 = /(?<![\d.])\d{1,3}(?:\.\d{1,3}){3}(?!\d|\.\d)(?:\/\d{1,2}(?!\d))?/g;
// IPv6 candidates are maximal runs of hex digits, colons and dots (see
// eachV6), so no address is missed for what touches it.
const V6_RUN = /[0-9A-Fa-f:.]+/g;
const KEYISH = [
  /\bsetup_[0-9a-f]{16,}\b/g,
  /\b(?:sk|pk|rk)-(?:[A-Za-z0-9]+-)*[A-Za-z0-9_-]{20,}/g,
  /\b(?:sk|pk|rk)_(?:live|test)_[A-Za-z0-9]{16,}/g,
  /\bgh[pousr]_[A-Za-z0-9]{20,}\b/g,
  /\bgithub_pat_[A-Za-z0-9_]{20,}/g,
  /\bxox[abprs]-[A-Za-z0-9-]{10,}/g,
  /\bglpat-[A-Za-z0-9_-]{20,}/g,
  /\bAKIA[0-9A-Z]{16}\b/g,
  /\bAIza[0-9A-Za-z_-]{30,}/g,
  /\bAGE-SECRET-KEY-1[0-9A-Z]{20,}/g,
  /\beyJ[\w-]{10,}\.[\w-]{10,}\.[\w-]{10,}/g,
  /\bhooks\.slack\.com\/(?:services|workflows|triggers)\/[^\s)'"`\]>]+/g,
  /\bdiscord(?:app)?\.com\/api\/webhooks\/[^\s)'"`\]>]+/g,
];
// Runs a secret can hide in once the named shapes above are gone: 32 or
// more of [A-Za-z0-9+/=_-]. tokenLike() keeps the ones that read as paths,
// identifiers or ids; hex runs are judged by hexSecret().
const TOKEN_RUN = /(?<![A-Za-z0-9+/=_-])[A-Za-z0-9+/=_-]{32,}(?![A-Za-z0-9+/=_-])/g;
const HEX_RUN = /(?<![0-9A-Za-z])(?:[0-9a-f]{32,}|[0-9A-F]{32,})(?![0-9A-Za-z])/g;
const KV = /\b([A-Za-z0-9_.-]*(?:api[_-]?key|apikey|secret|token|passw(?:or)?d|passwd|pwd|private[_-]?key|access[_-]?key|client[_-]?secret|credentials?)[A-Za-z0-9_.-]*)(["']?\s*[=:]\s*["']?)([^\s"'`,;)\]}>]{6,})/gi;
const KV_KEEP = /^(<|\$|\{|\[|\/|\.\/|~\/|https?:|process\.|env\.|os\.environ)|^\d+$|^(string|number|boolean|null|undefined|true|false|none|required|optional|redacted|example|placeholder|changeme|unset|empty|secret|token|password)$/i;
const URL_PASSWORD = /\b([a-z][a-z0-9+.-]*:\/\/[^\s:@/]+):([^\s@/]+)@/gi;
// Home directories other than this one's ($HOME is "~"), and root's.
const HOME_PATH = /(?<![\w.~-])\/(?:Users|home)\/[^/\s`'"()[\]<>]+/g;
const ROOT_HOME = /(?<![\w.~-])\/root(?=\/)/g;
const WIN_HOME = /\b[A-Za-z]:\\Users\\[^\\\s`'"]+/g;
// A sudo rule says who may run what as root on which host: never published.
const SUDO_RULE = /^.*\bNOPASSWD\b.*$/gm;
// Where private keys (and key-encryption keys) live: the path becomes "<key file>".
const KEY_FILE = /(?:~|\.{1,2}|<[\w-]+>)?(?:\/[\w.@-]+)+\/[\w.@-]*(?:[-_.]key\.pem|\/key\.pem|\.key|\.p12|\.pfx|[-_.]kek|id_(?:rsa|dsa|ecdsa|ed25519))(?![\w.-])/g;
// `git [-C dir] [-c k=v] [--opt] <subcommand>` for the subcommands that write history
const GIT_WRITE = /\bgit(?:\s+(?:-[Cc]\s+(?:\\?"[^"\\]*\\?"|'[^']*'|\S+)|--[\w-]+(?:=\S+)?))*\s+(?:commit|merge|cherry-pick|rebase|am|revert|pull|reset)\b/;
const EMAIL = /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}\b/g;

function v4ok(a) {
  return /^(127\.|0\.0\.0\.0$|192\.0\.2\.|198\.51\.100\.|203\.0\.113\.)/.test(a);
}

function v6ok(a) {
  return /^(::1?|0:0:0:0:0:0:0:[01])$/i.test(a) || /^2001:0?db8:/i.test(a);
}

function ip4(m) {
  const a = m.replace(/\/\d+$/, '');
  return isIP(a) === 4 && !v4ok(a);
}

// Calls fn(start, end) for each IPv6 address in s that is neither
// loopback nor documentation. A candidate is a maximal run of hex digits,
// colons and dots with two colons or more, plus a %zone and /prefix. One
// glued to a word on its left is code ("std::fs"), unless a single colon
// separates them ("addr:fd57::1"). Trailing '.' and ':' (the end of a
// sentence, "at fd57::1.") are tried off before net.isIP judges it.
function eachV6(s, fn) {
  for (const m of s.matchAll(V6_RUN)) {
    if (m[0].split(':').length < 3) continue;
    let a = m.index;
    const b = a + m[0].length;
    if (/[0-9A-Za-z_]/.test(s[a - 1] ?? '')) {
      if (m[0][0] !== ':' || m[0][1] === ':') continue;
      a += 1;
    }
    const tail = s.slice(b).match(/^(?:%[0-9A-Za-z_.-]*[0-9A-Za-z_-])?(?:\/\d{1,3}(?!\d))?/)[0];
    let end = b + tail.length;
    for (;;) {
      const tok = s.slice(a, end);
      const addr = tok.replace(/\/\d{1,3}$/, '').replace(/%[0-9A-Za-z_.-]+$/, '');
      if (isIP(addr) === 6) {
        if (!v6ok(addr)) fn(a, end);
        break;
      }
      if (end - a > 2 && /[.:]$/.test(tok)) end -= 1;
      else break;
    }
  }
}

function replaceV6(s) {
  const spans = [];
  eachV6(s, (a, b) => spans.push([a, b]));
  for (const [a, b] of spans.reverse()) s = `${s.slice(0, a)}<ipv6>${s.slice(b)}`;
  return s;
}

// Whether a TOKEN_RUN match is secret-shaped rather than a path, an
// identifier, a URL or an id. Base64 with '=' padding is; so is an
// all-caps-and-digits piece of 24+ with a digit in it (base32), and a run
// of mixed case that switches case every char or two (one- and two-letter
// humps; words and CamelCase have long ones), has a piece with a long
// capital run and digits, or has digits wedged between letters twice.
function tokenLike(run) {
  if (/[A-Za-z0-9]={1,2}$/.test(run)) return true;
  const pieces = run.split(/[/_=+-]+/).filter(Boolean);
  if (pieces.some((p) => p.length >= 24 && /^[A-Z0-9]+$/.test(p) && /\d/.test(p) && /[A-Z]/.test(p))) return true;
  if (!/[A-Z]/.test(run) || !/[a-z]/.test(run)) return false;
  let letters = 0;
  let humps = 0;
  let singles = 0;
  for (const p of pieces) {
    if (/^(?:[0-9a-f]+|\d+)$/.test(p)) continue; // lowercase hex and numbers: ids, judged by hexSecret
    for (const h of p.match(/[A-Z]+(?![a-z])|[A-Z]?[a-z]+|\d+/g) ?? []) {
      letters += h.length;
      humps += 1;
      if (h.length === 1) singles += 1;
    }
  }
  if (letters < 20) return false;
  if (letters / humps < 3 || singles / humps >= 0.3) return true;
  const wedged = pieces.filter((p) => !/^[0-9a-f]+$/.test(p)).reduce((n, p) => n + (p.match(/[A-Za-z]\d+(?=[A-Za-z])/g) ?? []).length, 0);
  return wedged >= 2 || pieces.some((p) => /\d/.test(p) && /[A-Z]{5,}/.test(p) && /[a-z]/.test(p));
}

const commitCache = new Map();
// A 32+ hex run is a secret unless it is a full commit id of this repo.
function hexSecret(hex) {
  if (hex.length !== 40) return true;
  if (!commitCache.has(hex)) {
    let known = false;
    try {
      execFileSync('git', ['-C', mainRoot, 'cat-file', '-e', `${hex}^{commit}`], { stdio: 'ignore' });
      known = true;
    } catch {
      // not an object here
    }
    commitCache.set(hex, known);
  }
  return !commitCache.get(hex);
}

function makeScrubber() {
  const literals = loadLiterals();
  const litRe = (l, flags) => new RegExp(l.word ? `(?<![A-Za-z])${escapeRe(l.from)}(?![A-Za-z])` : escapeRe(l.from), flags);
  const litRes = literals.map((l) => [litRe(l, 'gi'), l.to, litRe(l, 'i')]);
  const roots = [...new Set([workspaceRoot, mainRoot, realpath(mainRoot)])].sort((a, b) => b.length - a.length);
  const rootRe = roots.map(escapeRe).join('|');
  const tree = `(?:${rootRe})(?:/\\.muse/worktrees/[^/\\s)\\]'"\`]+)?`;
  const linkRe = new RegExp(`\\]\\(${tree}(/[^)\\s]*)?\\)`, 'g');
  const pathRe = new RegExp(`${tree}/`, 'g');
  const bareRe = new RegExp(`${tree}(?![\\w/.-])`, 'g');
  const homeRe = new RegExp(`${escapeRe(HOME)}(?=[/\\s\`'")\\]:,;]|$)`, 'gm');

  const line = (input) => {
    let s = String(input ?? '');
    s = s.replace(PEM, '[PEM block removed]');
    s = s.replace(SUDO_RULE, '[sudo rule omitted]');
    s = s.replace(SHELL_PROMPT, (m) => (m.endsWith('#') ? '#' : '$'));
    s = s.replace(JOURNAL, '$1$2 <host> $3:');
    s = s.replace(linkRe, (m, rest) => `](../../../${(rest ?? '/').slice(1)})`);
    s = s.replace(pathRe, '');
    s = s.replace(bareRe, '<repo>');
    s = s.replace(homeRe, '~');
    s = s.replace(HOME_PATH, '<home>').replace(ROOT_HOME, '<root-home>').replace(WIN_HOME, '<home>');
    s = s.replace(KEY_FILE, '<key file>');
    for (const [re, to] of litRes) s = s.replace(re, to);
    s = s.replace(/\b(Bearer|Basic|Token)\s+[A-Za-z0-9._~+/=-]{8,}/g, '$1 <redacted>');
    s = s.replace(URL_PASSWORD, '$1:<redacted>@');
    s = s.replace(KV, (m, k, sep, v) => (KV_KEEP.test(v) ? m : `${k}${sep}<redacted>`));
    for (const re of KEYISH) s = s.replace(re, (m) => (m.startsWith('setup_') ? 'setup_<redacted>' : '<redacted-token>'));
    s = s.replace(HEX_RUN, (m) => (hexSecret(m) ? '<redacted-hex>' : m));
    s = s.replace(TOKEN_RUN, (m) => (tokenLike(m) ? '<redacted-token>' : m));
    s = s.replace(V4, (m) => (ip4(m) ? '<ipv4>' : m));
    s = replaceV6(s);
    s = s.replace(EMAIL, (m) => (/@(example\.(com|org|net)|users\.noreply\.github\.com)$/i.test(m) ? m : '<email>'));
    return s.replace(/\[redacted\]/g, '[redacted by Muse]');
  };

  return {
    line,
    // Operator text (prompts, steers, answers, peer messages) also loses
    // pasted terminal output.
    text(input, { terminal = false, display = null } = {}) {
      let s = String(input ?? '').replace(/\r\n?/g, '\n').replace(PEM, '[PEM block removed]');
      if (terminal) s = collapseTerminal(s, display);
      return line(s);
    },
    // What must never reach the repo, whatever the scrub missed. Counts and
    // line numbers only: the values themselves are not logged.
    check(doc) {
      const found = new Map();
      const lines = doc.split('\n');
      const hit = (kind, n) => {
        const e = found.get(kind) ?? { count: 0, lines: [] };
        e.count += 1;
        if (e.lines.length < 5) e.lines.push(n);
        found.set(kind, e);
      };
      const all = (re, l) => l.matchAll(new RegExp(re.source, re.flags.includes('g') ? re.flags : `${re.flags}g`));
      for (const [i, l] of lines.entries()) {
        const n = i + 1;
        if (/-----BEGIN [A-Z0-9 ]*-----|-----END [A-Z0-9 ]*-----/.test(l)) hit('PEM block', n);
        if (/\bNOPASSWD\b/.test(l)) hit('sudo rule', n);
        for (const re of KEYISH) for (const m of all(re, l)) if (m[0] !== 'setup_<redacted>') hit('key-shaped token', n);
        for (const m of all(HEX_RUN, l)) if (hexSecret(m[0])) hit('long hex string', n);
        for (const m of all(TOKEN_RUN, l)) if (tokenLike(m[0])) hit('key-shaped token', n);
        for (const m of l.matchAll(/\b([A-Za-z0-9_.-]*(?:api[_-]?key|secret|token|passw(?:or)?d)[A-Za-z0-9_.-]*)(["']?\s*[=:]\s*["']?)([^\s"'`,;)\]}>]{6,})/gi)) {
          if (!KV_KEEP.test(m[3]) && m[3] !== '<redacted>') hit('key=value secret', n);
        }
        for (const m of l.matchAll(/\d{1,3}(?:\.\d{1,3}){3}/g)) {
          const prev = l[m.index - 1] ?? '';
          const next = l.slice(m.index + m[0].length, m.index + m[0].length + 2);
          if (/[\d.]/.test(prev) || /^(\d|\.\d)/.test(next)) continue;
          if (ip4(m[0])) hit('IPv4 address', n);
        }
        eachV6(l, () => hit('IPv6 address', n));
        if (/(?<![\w.~-])\/(?:Users|home)\/[A-Za-z0-9._-]+(?:\/|$|[\s`'")\]])/.test(l) || /\b[A-Za-z]:\\Users\\[^\\\s]+/.test(l)) hit('home directory path', n);
        for (const [, , test] of litRes) if (test.test(l)) hit('scrub-map literal', n);
      }
      return [...found].map(([k, v]) => `${v.count} ${k} (line ${v.lines.join(', ')})`);
    },
  };
}

// The literal map lives in the main checkout, git-ignored:
// .muse/session-log.scrub.json = {"replace": {"literal": "replacement"},
// "words": ["literal", ...]}. A literal is replaced wherever it occurs, any
// case, glued to other letters or not ("BuildboxVM" too); one listed in
// "words" (an ordinary word such as a short user name) only where no letter
// touches it ("pat" leaves "path" alone). The final check uses the same
// rule, so a literal is never left in a written digest.
// Missing, unreadable, malformed or still the example: refuse everything,
// never "no map". {"replace": {}} is the one way to say there is nothing.
function loadLiterals() {
  const file = join(mainRoot, '.muse/session-log.scrub.json');
  if (!existsSync(file)) {
    throw new Error('.muse/session-log.scrub.json is missing; copy .muse/session-log.scrub.example.json to it and fill in your host, user and project names ({"replace": {}} if there are none); no digest written');
  }
  let cfg;
  try {
    cfg = JSON.parse(readFileSync(file, 'utf8'));
  } catch (err) {
    throw new Error(`.muse/session-log.scrub.json is unreadable (${err.message}); no digest written`);
  }
  const map = cfg?.replace;
  if (!map || typeof map !== 'object' || Array.isArray(map)) throw new Error('.muse/session-log.scrub.json has no "replace" object; no digest written');
  const words = new Set((Array.isArray(cfg.words) ? cfg.words : []).map((w) => String(w).toLowerCase()));
  const entries = Object.entries(map);
  for (const [n, [from, to]] of entries.entries()) {
    if (!from.trim() || typeof to !== 'string') throw new Error(`.muse/session-log.scrub.json entry ${n + 1} is not "literal": "replacement"; no digest written`);
    if (from.length < 4 && !words.has(from.toLowerCase())) throw new Error(`.muse/session-log.scrub.json entry ${n + 1} is shorter than 4 characters; list it in "words" too; no digest written`);
    for (const [other] of entries) {
      if (to.toLowerCase().includes(other.toLowerCase())) throw new Error(`.muse/session-log.scrub.json entry ${n + 1}: the replacement contains a literal; no digest written`);
    }
  }
  const example = join(mainRoot, '.muse/session-log.scrub.example.json');
  try {
    const placeholders = new Set(Object.keys(JSON.parse(readFileSync(example, 'utf8')).replace ?? {}));
    if (entries.length && entries.every(([from]) => placeholders.has(from))) {
      throw new Error('.muse/session-log.scrub.json still holds the example placeholders; put your own host, user and project names in it; no digest written');
    }
  } catch (err) {
    if (/no digest written/.test(err.message)) throw err;
    // no example to compare with
  }
  return entries
    .map(([from, to]) => ({ from, to, word: words.has(from.toLowerCase()) }))
    .sort((a, b) => b.from.length - a.from.length);
}

// A paste of terminal output becomes "[N lines of terminal output omitted]".
// When Muse kept the composer form ("[Pasted Content 1706 chars]"), each
// pasted region is judged on its own; otherwise blocks of TERM lines are,
// with indentation and quote marks ignored, and a collapsed block takes the
// output-like lines around it (the paths, key names and headers of the same
// paste) up to a blank line or a line of prose.
function collapseTerminal(text, display) {
  const marker = (n) => `[${n} line${n === 1 ? '' : 's'} of terminal output omitted]`;
  const regions = display ? alignPastes(display, text) : null;
  if (regions) {
    let out = '';
    let pos = 0;
    for (const [a, b] of regions) {
      const chunk = text.slice(a, b);
      const lines = chunk.split('\n');
      out += text.slice(pos, a) + (isTerminal(lines) ? marker(lines.filter((l) => l.trim()).length) : chunk);
      pos = b;
    }
    text = out + text.slice(pos);
  }
  const lines = text.split('\n');
  const bare = lines.map(unquote);
  const term = bare.map((l) => TERM.test(l));
  const total = term.filter(Boolean).length;
  if (!total) return text;
  const heavy = total / Math.max(1, lines.filter((l) => l.trim()).length) >= 0.3;
  const blocks = [];
  for (const [i, t] of term.entries()) {
    if (!t) continue;
    const cur = blocks.at(-1);
    if (cur && i - cur.last <= 4) {
      cur.last = i;
      cur.n += 1;
    } else blocks.push({ first: i, last: i, n: 1 });
  }
  const picked = heavy ? [{ first: blocks[0].first, last: blocks.at(-1).last }] : blocks.filter((b) => b.n >= 3);
  if (!picked.length) return text;
  const outputish = (i) => bare[i].trim() !== '' && (term[i] || OUTPUTISH.test(bare[i].trim()));
  for (const sp of picked) {
    while (sp.first > 0 && outputish(sp.first - 1)) sp.first -= 1;
    while (sp.last < lines.length - 1 && outputish(sp.last + 1)) sp.last += 1;
  }
  const spans = [];
  for (const sp of picked) {
    const prev = spans.at(-1);
    if (prev && sp.first <= prev.last + 1) prev.last = Math.max(prev.last, sp.last);
    else spans.push({ ...sp });
  }
  const out = [];
  let i = 0;
  for (const sp of spans) {
    out.push(...lines.slice(i, sp.first));
    out.push(marker(lines.slice(sp.first, sp.last + 1).filter((l) => l.trim()).length));
    i = sp.last + 1;
  }
  out.push(...lines.slice(i));
  return out.join('\n');
}

function unquote(l) {
  return l.replace(/^[\s>]+/, '');
}

function isTerminal(lines) {
  const t = lines.filter((l) => TERM.test(unquote(l))).length;
  return t >= 3 || (t > 0 && t / Math.max(1, lines.filter((l) => l.trim()).length) >= 0.3);
}

function alignPastes(display, text) {
  const PH = /\[Pasted Content \d+ chars\]/g;
  if (!PH.test(display)) return null;
  const parts = display.split(PH);
  if (!text.startsWith(parts[0]) || !text.endsWith(parts.at(-1))) return null;
  const regions = [];
  let pos = parts[0].length;
  for (let j = 1; j < parts.length; j += 1) {
    if (j === parts.length - 1) {
      const end = text.length - parts[j].length;
      if (end < pos) return null;
      regions.push([pos, end]);
    } else if (parts[j]) {
      const at = text.indexOf(parts[j], pos);
      if (at < 0) return null;
      regions.push([pos, at]);
      pos = at + parts[j].length;
    }
    // an empty middle part: two pastes back to back; judged as one region
  }
  return regions;
}

// ------------------------------------------------------------ muse plumbing

// For the hook process only (runs before the module's constants exist).
function rawLogSize(id) {
  try {
    const { DatabaseSync } = process.getBuiltinModule('node:sqlite');
    const db = new DatabaseSync(join(HOME, '.local/share/muse/session-index.db'), { readOnly: true });
    try {
      const path = db.prepare('select session_log_path from sessions where session_id = ?').get(id)?.session_log_path;
      return path ? statSync(path).size : -1;
    } finally {
      db.close();
    }
  } catch {
    return -1;
  }
}

// Muse's session index (SQLite, read-only). Absent or unreadable: names
// and workspaces come from the export alone and a full run finds nothing.
function openIndex() {
  try {
    const { DatabaseSync } = process.getBuiltinModule('node:sqlite');
    const path = join(HOME, '.local/share/muse/session-index.db');
    return existsSync(path) ? new DatabaseSync(path, { readOnly: true }) : null;
  } catch {
    return null;
  }
}

function indexRow(id) {
  if (!index || !id) return null;
  if (!rowCache.has(id)) {
    rowCache.set(id, index.prepare('select session_id, session_name, logged_session_name, workspace_root, session_log_path, msp_fork_source_session_id from sessions where session_id = ?').get(id) ?? null);
  }
  return rowCache.get(id);
}

// A side chat's first raw records name its parent (session.fork.created).
function parentFromLog(path) {
  if (!path || !existsSync(path)) return null;
  const head = readTail(path, 0, 256 << 10);
  for (const l of head.split('\n')) {
    if (!/"(session\.fork\.created|session\.side\.started)"/.test(l)) continue;
    const m = l.match(/"source_session_id":"([0-9a-f-]{36})"/);
    if (m) return m[1];
  }
  return null;
}

// The parent's name: from this run, an existing digest, or the parent's raw
// log (only its session.name.changed lines are parsed).
function nameOf(id) {
  if (names.has(id)) return names.get(id);
  let name = null;
  for (const f of digestFiles()) {
    const m = digestMeta(join(outDir, f));
    if (m.id === id && m.name) name = m.name;
  }
  const row = indexRow(id);
  if (!name && row?.session_log_path && existsSync(row.session_log_path)) {
    eachLine(row.session_log_path, (l) => {
      if (!l.includes('"session.name.changed"')) return;
      try {
        const n = JSON.parse(l).payload?.new_name;
        if (n) name = n;
      } catch {
        // a torn last line
      }
    });
  }
  name = name ?? row?.session_name ?? row?.logged_session_name ?? null;
  names.set(id, name);
  return name;
}

// --------------------------------------------------------------- helpers

function gitMainRoot() {
  const common = execFileSync('git', ['rev-parse', '--path-format=absolute', '--git-common-dir'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  return dirname(common);
}

function skipList() {
  const file = join(mainRoot, '.muse/session-log.skip');
  if (!existsSync(file)) return new Set();
  return new Set(readFileSync(file, 'utf8').split('\n').map((l) => l.replace(/#.*/, '').trim()).filter(Boolean));
}

function digestFiles() {
  if (!existsSync(outDir)) return [];
  return readdirSync(outDir).filter((f) => /^\d{4}-\d\d-\d\d-.+\.md$/.test(f)).sort();
}

function digestMeta(path) {
  try {
    const head = readTail(path, 0, 8192);
    const m = head.match(/<!-- session-log: (\{.*\}) -->/);
    return m ? JSON.parse(m[1]) : {};
  } catch {
    return {};
  }
}

// This session's digests: v2 names end in the full id; a v1 name ends in
// the first 8 hex, which other sessions can share, so it must say the id.
function digestsFor(id) {
  return digestFiles()
    .filter((f) => f.endsWith(`-${id}.md`) || (f.endsWith(`-${id.slice(0, 8)}.md`) && digestMeta(join(outDir, f)).id === id))
    .map((f) => join(outDir, f));
}

function writeAtomic(file, text) {
  const tmp = join(dirname(file), `.${basename(file)}.${process.pid}.tmp`);
  writeFileSync(tmp, text);
  renameSync(tmp, file);
}

function cleanTemps() {
  for (const f of readdirSync(outDir)) {
    if (!/^\..+\.\d+\.tmp$/.test(f)) continue;
    const st = statSafe(join(outDir, f));
    if (st && Date.now() - st.mtimeMs > 3600_000) rmSync(join(outDir, f), { force: true });
  }
}

function logError(what, err) {
  try {
    mkdirSync(stateDir, { recursive: true });
    const file = join(stateDir, 'errors.log');
    const msg = String(err?.message ?? err).replace(/\s+/g, ' ').slice(0, 600);
    appendFileSync(file, `${new Date().toISOString()} ${what ?? '-'} ${msg}\n`);
    const lines = readFileSync(file, 'utf8').split('\n');
    if (lines.length > 600) writeAtomic(file, lines.slice(-500).join('\n'));
    console.error(`session-log: ${what ?? '-'}: ${msg}`);
  } catch {
    // nowhere left to report
  }
}

function readTail(path, from, max) {
  const fd = openSync(path, 'r');
  try {
    const buf = Buffer.alloc(max);
    const n = readSync(fd, buf, 0, max, from);
    return buf.subarray(0, n).toString('utf8');
  } finally {
    closeSync(fd);
  }
}

// Each line of a file as a string. Lines are cut on bytes and decoded whole,
// so a character split across two reads is never mangled.
function eachLine(path, fn) {
  eachLineBytes(path, (b) => fn(b.toString('utf8')));
}

function eachLineBytes(path, fn) {
  const fd = openSync(path, 'r');
  const buf = Buffer.allocUnsafe(1 << 22);
  let rest = [];
  try {
    for (;;) {
      const n = readSync(fd, buf, 0, buf.length, null);
      if (!n) break;
      let from = 0;
      for (let nl = buf.indexOf(0x0a, 0); nl >= 0 && nl < n; nl = buf.indexOf(0x0a, from)) {
        const piece = buf.subarray(from, nl);
        fn(rest.length ? Buffer.concat([...rest, piece]) : piece);
        rest = [];
        from = nl + 1;
      }
      if (from < n) rest.push(Buffer.from(buf.subarray(from, n)));
    }
    if (rest.length) fn(Buffer.concat(rest));
  } finally {
    closeSync(fd);
  }
}

function statSafe(p) {
  try {
    return statSync(p);
  } catch {
    return null;
  }
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function describeSource(s) {
  return {
    subagent_result: 'subagent results',
    subagent_input: 'subagent input',
    goal_progress_nudge: 'the goal',
    goal_control: 'a goal change',
    background_task_terminal: 'a finished background task',
    background_task_overdue: 'an overdue background task',
    monitor_event: 'a monitor event',
    session_message: 'a session message',
    scheduled: 'a scheduled job',
    user_steer: 'you',
  }[s] ?? String(s).replace(/_/g, ' ');
}

function realpath(p) {
  try {
    return realpathSync(p);
  } catch {
    return p;
  }
}

function iso(ms) {
  return new Date(ms).toISOString().slice(0, 16).replace('T', ' ');
}

function hms(ms) {
  return new Date(ms).toISOString().slice(11, 19);
}

function duration(ms) {
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)}m ${s % 60}s` : `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

// Cut on a line boundary; never leave a code fence open.
function capped(s, n) {
  const t = String(s).trim();
  if (t.length <= n) return t;
  let cut = t.lastIndexOf('\n', n);
  if (cut < n / 2) cut = t.lastIndexOf(' ', n);
  if (cut < n / 2) cut = n;
  let kept = t.slice(0, cut).trimEnd();
  if ((kept.match(/^\s*(```|~~~)/gm) ?? []).length % 2) kept += '\n```';
  return `${kept}\n\n[... ${t.length - cut} more characters]`;
}

function plural(n, word) {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}

function cap(s, n) {
  const t = String(s ?? '').replace(/\s+/g, ' ').trim();
  return t.length <= n ? t : `${t.slice(0, n - 1)}…`;
}

function quote(s, prefix = '> ') {
  return demoteHeadings(s).split('\n').map((l) => (l ? `${prefix}${l}` : prefix.trimEnd())).join('\n');
}

// A response's own "## Steps" must not read as the next turn.
function demoteHeadings(s) {
  let fenced = false;
  return String(s).split('\n').map((l) => {
    if (/^\s*(```|~~~)/.test(l)) fenced = !fenced;
    return fenced ? l : l.replace(/^#{1,3}(?=\s)/, '####');
  }).join('\n');
}

function unescapeXml(s) {
  return s.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, '&');
}

function firstLine(s) {
  return String(s ?? '').split('\n').map((l) => l.replace(/^#+\s*/, '').trim()).find(Boolean) ?? '';
}

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function slug(s) {
  return String(s).toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 60) || 'session';
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const m = argv[i].match(/^--([a-z-]+)(?:=(.*))?$/);
    if (!m) continue;
    out[m[1]] = m[2] ?? (argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[++i] : true);
  }
  return out;
}

// Last, so every constant above is initialised before the work starts.
try {
  mkdirSync(stateDir, { recursive: true });
  await main();
} catch (err) {
  logError('run', err);
  process.exitCode = 1;
} finally {
  index?.close();
}
