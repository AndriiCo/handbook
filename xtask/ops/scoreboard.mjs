#!/usr/bin/env node
// Apostille Handbook scoreboard: the release checks against a live base URL.
//
//   node xtask/ops/scoreboard.mjs                 # production
//   node xtask/ops/scoreboard.mjs --base http://localhost:5173/apostille
//   node xtask/ops/scoreboard.mjs --only GP3,A1
//
// Exit 0 only when every selected row passes (with --report, exit 0 once the
// report is written: setup's production baselines, where the operator-owned
// S1 row may be red and the two reports are compared). Results go to --out, or by
// default web/shots/scoreboard.json for a full run and
// web/shots/scoreboard-partial.json for an --only run, so a one-row check
// never stands in for the real score. Production runs use --out
// web/shots/prod-before.json / prod-after.json and staging acceptance
// --out web/shots/staging-accept.json (docs/ops/deploy.md). Chromium starts only when a row needs a page, so the
// API rows still run where a sandbox blocks the browser.
//
// A row that fails is fixed in the product, not by loosening the row.

import { createRequire } from 'node:module';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { gp2PageProblems, probeValueNodes } from './gp2-page.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const args = parseArgs(process.argv.slice(2));
const BASE = (args.base || process.env.SCOREBOARD_BASE || 'https://andrii.ai/apostille').replace(/\/$/, '');
const ONLY = args.only ? new Set(String(args.only).split(',')) : null;
const OUT = resolve(ROOT, args.out || (ONLY ? 'web/shots/scoreboard-partial.json' : 'web/shots/scoreboard.json'));

const SLUGS = ['vietnam', 'usa-wa-sos'];
const MIN_FACTS = 8;
const MAX_QUOTE = 400; // a whole-page range is not a quote
const MIN_GOLD = 25;
const QUALITY_MAX_AGE_H = 72;
const BANNED = /\b(placeholder|not wired|skeleton|lorem ipsum|todo|coming soon)\b/i;
// A body font whose first family is one of these is a stand-in, not a typeface choice.
const STAND_IN_FONTS = new Set([
  'georgia', 'times', 'times new roman', 'serif', 'sans-serif', 'system-ui',
  '-apple-system', 'blinkmacsystemfont', 'segoe ui', 'roboto', 'helvetica',
  'helvetica neue', 'arial', 'ui-sans-serif', 'ui-serif',
]);

const CHECKS = [
  ['P0', 'Production answers /healthz', p0],
  ['GP2-data', `vietnam and usa-wa-sos each have >= ${MIN_FACTS} accepted facts`, gp2Data],
  ['A1', 'Every public fact has a verified quote, source and date; Spark drafted them', a1],
  ['GP1', 'Home: real typeface, search, >= 3 authorities, clean at 390 px', gp1],
  ['GP2-page', `Guide: named h1, >= ${MIN_FACTS} facts on visible traced values, no raw keys, live WS`, gp2Page],
  ['GP3', 'Sign-in: passkey autofill + branded buttons for configured providers', gp3],
  ['GP4', 'Order: stepper, clean at 390 px', gp4],
  ['A2', 'Photo precheck: SAM 3.1 + Muse Spark on a public sample, live', a2],
  ['A5', `Quality numbers from an eval run in the last ${QUALITY_MAX_AGE_H} h, shown on home`, a5],
  ['S1', 'Submission: licence file, README with video link, public repo', s1],
];

const { chromium } = loadPlaywright();
let browserLaunch = null; // started by the first row that visits a page
const browser = () => (browserLaunch ??= chromium.launch().catch((err) => {
  throw new Error(`browser unavailable: ${String(err?.message || err).split('\n')[0].slice(0, 120)}`);
}));
const results = [];
for (const [id, title, run] of CHECKS) {
  if (ONLY && !ONLY.has(id)) continue;
  const started = Date.now();
  let problems;
  let note = '';
  try {
    const out = await withTimeout(run(), 90_000, `${id} timed out after 90 s`);
    problems = out.problems;
    note = out.note || '';
  } catch (err) {
    problems = [`error: ${String(err?.message || err).slice(0, 200)}`];
  }
  results.push({ id, title, pass: problems.length === 0, detail: problems.join('; ') || note, ms: Date.now() - started });
}
if (browserLaunch) await browserLaunch.then((b) => b.close(), () => {});

const passing = results.filter((r) => r.pass).length;
const firstRed = results.find((r) => !r.pass);
const report = { base: BASE, at: new Date().toISOString(), passing, total: results.length, next: firstRed?.id ?? null, checks: results };
mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, JSON.stringify(report, null, 2) + '\n');

console.log(`Scoreboard  ${BASE}  ${report.at}`);
for (const r of results) {
  console.log(` ${r.pass ? 'PASS' : 'FAIL'}  ${r.id.padEnd(9)} ${r.title}`);
  if (r.detail) console.log(`        ${r.detail}`);
}
console.log(`${passing}/${results.length} passing.${firstRed ? ` Next row: ${firstRed.id}.` : ' All green.'}  (${OUT})`);
process.exit(args.report || passing === results.length ? 0 : 1);

// ---------------------------------------------------------------- checks

async function p0() {
  const r = await getJson('/healthz');
  return { problems: r.status === 200 ? [] : [`/healthz -> ${r.status}`], note: `version ${r.body?.version ?? '?'}` };
}

async function gp2Data() {
  const r = await getJson('/api/v1/authorities');
  if (r.status !== 200) return { problems: [`/api/v1/authorities -> ${r.status}`] };
  const rows = r.body?.authorities ?? [];
  const problems = [];
  for (const slug of SLUGS) {
    const row = rows.find((a) => a.slug === slug);
    if (!row) problems.push(`${slug}: not in /authorities (run the packs seed)`);
    else if ((row.fact_count ?? 0) < MIN_FACTS) problems.push(`${slug}: ${row.fact_count ?? 0}/${MIN_FACTS} accepted facts`);
  }
  return { problems, note: `${rows.length} authorities public` };
}

async function a1() {
  const problems = [];
  let spark = 0;
  let total = 0;
  for (const slug of SLUGS) {
    const r = await getJson(`/api/v1/authorities/${slug}/facts`);
    if (r.status !== 200) { problems.push(`${slug}/facts -> ${r.status}`); continue; }
    const facts = r.body?.facts ?? [];
    total += facts.length;
    if (facts.length < MIN_FACTS) problems.push(`${slug}: ${facts.length}/${MIN_FACTS} facts`);
    let bad = 0;
    for (const f of facts) {
      const c = Array.isArray(f.citations) ? f.citations[0] : null;
      const ok = c && typeof c.quote === 'string' && c.quote.trim().length >= 8 && c.quote.trim().length <= MAX_QUOTE
        && /^https:\/\//.test(c.source_url ?? '') && !Number.isNaN(Date.parse(c.captured_at ?? ''));
      if (!ok) bad += 1;
      if (/muse-spark/i.test(f.drafted_by ?? f.created_by ?? '')) spark += 1;
    }
    if (bad) problems.push(`${slug}: ${bad} fact(s) without citations[0].{quote 8-${MAX_QUOTE} chars, https source_url, captured_at}`);
  }
  if (total && !spark) problems.push('no fact has drafted_by matching muse-spark');
  return { problems, note: `${total} facts, ${spark} drafted by Spark` };
}

async function gp1() {
  const problems = [];
  const wide = await visit('/', 1440, 900);
  problems.push(...clean(wide, '1440'));
  const family = wide.font.split(',')[0].trim().replace(/["']/g, '').toLowerCase();
  if (STAND_IN_FONTS.has(family)) problems.push(`body font is a stand-in (${family})`);
  const search = await wide.page.locator('[role=combobox], [role=searchbox], input[type=search], [aria-keyshortcuts]').count();
  if (!search) problems.push('no search (role=combobox/searchbox, input[type=search] or [aria-keyshortcuts])');
  const links = await wide.page.locator('a[href*="/authority/"]').count();
  if (links < 3) problems.push(`${links}/3 authority links`);
  await wide.ctx.close();
  const narrow = await visit('/', 390, 844);
  problems.push(...clean(narrow, '390'));
  await narrow.ctx.close();
  return { problems, note: `font ${family}` };
}

// v2 (operator-approved 2026-11-03): facts are traceable to visible
// values, not raw statements; the predicate lives in gp2-page.mjs.
async function gp2Page() {
  const problems = [];
  const add = (list) => { for (const p of list) if (!problems.includes(p)) problems.push(p); };
  const r = await getJson('/api/v1/authorities/vietnam/facts');
  const facts = r.body?.facts ?? [];
  const seen = [];
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    const label = String(width);
    const v = await visit('/authority/vietnam', width, height);
    add(clean(v, label));
    if (width === 1440) {
      const h1 = (await v.page.locator('h1').first().innerText().catch(() => '')).trim();
      if (h1.length < 4 || h1.toLowerCase() === 'vietnam') add([`h1 is "${h1}" (want the authority's name)`]);
    }
    // The probe renders content-visibility:auto subtrees first, so its
    // text (not the visit's) is what the raw-key rule reads.
    const probe = await v.page.evaluate(probeValueNodes);
    const out = gp2PageProblems({ label, probe, text: probe.text, facts, min: MIN_FACTS });
    add(out.problems);
    seen.push(`${out.shown}/${out.total} at ${label}`);
    if (width === 1440) {
      const live = v.sockets.some((u) => /\/ws\/bus/.test(u) && /authority(%3A|:)vietnam/i.test(u));
      if (!live) add(['no WebSocket to /ws/bus?topic=authority:vietnam']);
    }
    await v.ctx.close();
  }
  return { problems, note: `facts on visible values ${seen.join(', ')}, live WS` };
}

async function gp3() {
  const problems = [];
  const v = await visit('/login', 390, 844);
  problems.push(...clean(v, '390'));
  const page = v.page;
  if (!(await page.locator('input[autocomplete*="webauthn"]').count())) problems.push('no input with autocomplete="username webauthn"');
  const passkey = page.getByRole('button', { name: /passkey/i }).first();
  const box = await passkey.boundingBox().catch(() => null);
  if (!box) problems.push('no "passkey" button');
  else if (box.y + box.height > 844) problems.push('passkey button below the fold at 390x844');
  const prov = await getJson('/api/v1/auth/providers');
  if (prov.status !== 200 || !Array.isArray(prov.body?.providers)) {
    problems.push(`/api/v1/auth/providers -> ${prov.status} (want {"providers":[...]})`);
  } else {
    for (const p of prov.body.providers) {
      const btn = page.locator('button, a').filter({ hasText: new RegExp(p, 'i') }).first();
      if (!(await btn.count())) problems.push(`${p}: configured but no button`);
      else if (!(await btn.locator('svg, img').count())) problems.push(`${p}: button has no logo`);
    }
  }
  await v.ctx.close();
  const wide = await visit('/login', 1440, 900);
  problems.push(...clean(wide, '1440'));
  await wide.ctx.close();
  return { problems, note: `providers ${JSON.stringify(prov.body?.providers ?? [])}` };
}

async function gp4() {
  const problems = [];
  for (const [w, h] of [[1440, 900], [390, 844]]) {
    const v = await visit('/order', w, h);
    problems.push(...clean(v, String(w)));
    if (w === 1440 && !(await v.page.locator('[aria-current="step"]').count())) problems.push('no stepper ([aria-current="step"])');
    await v.ctx.close();
  }
  return { problems };
}

async function a2() {
  const problems = [];
  const s = await getJson('/api/v1/demo/precheck/samples');
  const samples = s.body?.samples ?? [];
  if (s.status !== 200 || !samples.length) return { problems: [`/api/v1/demo/precheck/samples -> ${s.status}, ${samples.length} samples`] };
  const t0 = Date.now();
  const r = await postJson('/api/v1/demo/precheck', { sample: samples[0].id });
  const ms = Date.now() - t0;
  const models = (r.body?.models ?? []).join(' ');
  if (r.status !== 200) problems.push(`POST /api/v1/demo/precheck -> ${r.status}`);
  else {
    if (!['pass', 'retake'].includes(r.body?.verdict)) problems.push(`verdict "${r.body?.verdict}"`);
    if (!/sam-3\.1/.test(models)) problems.push('models[] lacks sam-3.1');
    if (!/muse-spark/.test(models)) problems.push('models[] lacks muse-spark');
  }
  const v = await visit('/precheck', 390, 844);
  problems.push(...clean(v, '390'));
  await v.ctx.close();
  return { problems, note: `${ms} ms, ${models}` };
}

async function a5() {
  const problems = [];
  const r = await getJson('/api/v1/quality');
  if (r.status !== 200) return { problems: [`/api/v1/quality -> ${r.status}`] };
  const q = r.body ?? {};
  const age = (Date.now() - Date.parse(q.run_at ?? '')) / 3.6e6;
  if (!(age <= QUALITY_MAX_AGE_H)) problems.push(`run_at "${q.run_at}" older than ${QUALITY_MAX_AGE_H} h`);
  if (!((q.gold_count ?? 0) >= MIN_GOLD)) problems.push(`gold_count ${q.gold_count ?? 0}/${MIN_GOLD}`);
  const rate = q.quote_found_rate;
  if (!/muse-spark/i.test(q.model ?? '')) problems.push(`model "${q.model ?? '?'}" (want a live muse-spark eval)`);
  if (!(typeof rate === 'number' && rate > 0 && rate <= 1)) problems.push(`quote_found_rate ${rate ?? 'missing'} (want 0 < rate <= 1)`);
  else {
    const v = await visit('/', 1440, 900);
    if (!v.text.includes(`${Math.round(rate * 100)}%`)) problems.push(`home does not show ${Math.round(rate * 100)}%`);
    await v.ctx.close();
  }
  return { problems, note: `model ${q.model ?? '?'}, quote_found ${rate}` };
}

async function s1() {
  const problems = [];
  const read = (f) => (existsSync(join(ROOT, f)) ? readFileSync(join(ROOT, f), 'utf8') : '');
  const license = read('LICENSE.md');
  if (!/^# .*licen[cs]e/im.test(license) || !/^Required Notice: /m.test(license)) problems.push('LICENSE.md missing, or has no license heading and Required Notice line');
  const readme = read('README.md');
  if (!/https:\/\/(www\.)?(youtube\.com|youtu\.be|vimeo\.com|loom\.com)\//.test(readme)) problems.push('README has no demo video link');
  if (!/Muse Code/.test(readme)) problems.push('README does not say it was built with Muse Code');
  try {
    const vis = execFileSync('gh', ['repo', 'view', '--json', 'visibility', '-q', '.visibility'], { cwd: ROOT, timeout: 15_000, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
    if (vis !== 'PUBLIC') problems.push(`repo visibility ${vis}`);
  } catch {
    problems.push('could not read repo visibility (gh)');
  }
  return { problems };
}

// --------------------------------------------------------------- helpers

async function visit(path, width, height) {
  const ctx = await (await browser()).newContext({ viewport: { width, height }, colorScheme: 'light' });
  const page = await ctx.newPage();
  const errors = [];
  const sockets = [];
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text().slice(0, 120)); });
  page.on('pageerror', (e) => errors.push(String(e).slice(0, 120)));
  page.on('websocket', (ws) => sockets.push(ws.url()));
  const res = await page.goto(BASE + path, { waitUntil: 'load', timeout: 20_000 });
  await page.waitForTimeout(1500);
  const dom = await page.evaluate(() => ({
    text: document.body.innerText,
    font: getComputedStyle(document.body).fontFamily,
    overflow: document.documentElement.scrollWidth - window.innerWidth,
  }));
  return { ctx, page, status: res?.status() ?? 0, errors, sockets, ...dom };
}

function clean(v, label) {
  const p = [];
  if (v.status !== 200) p.push(`${label}: HTTP ${v.status}`);
  if (v.errors.length) p.push(`${label}: ${v.errors.length} console error(s), first: ${v.errors[0]}`);
  const banned = v.text.match(BANNED);
  if (banned) p.push(`${label}: visible "${banned[0]}"`);
  if (v.overflow > 0) p.push(`${label}: ${v.overflow}px horizontal overflow`);
  return p;
}

async function getJson(path) {
  return request(path, { headers: { accept: 'application/json' } });
}

async function postJson(path, body) {
  return request(path, { method: 'POST', headers: { accept: 'application/json', 'content-type': 'application/json' }, body: JSON.stringify(body) });
}

async function request(path, init) {
  const r = await fetch(BASE + path, { ...init, signal: AbortSignal.timeout(60_000) });
  let body = null;
  try { body = await r.json(); } catch { /* not JSON */ }
  return { status: r.status, body };
}

function withTimeout(promise, ms, message) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(message)), ms); })])
    .finally(() => clearTimeout(timer));
}

// Worktrees (Muse subagents, `muse -w`) have no web/node_modules; fall back
// to the main checkout's.
function loadPlaywright() {
  const roots = [ROOT];
  try {
    const common = execFileSync('git', ['rev-parse', '--path-format=absolute', '--git-common-dir'], { cwd: ROOT, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
    roots.push(dirname(common));
  } catch { /* not a git checkout */ }
  for (const r of roots) {
    if (existsSync(join(r, 'web/node_modules/@playwright/test'))) return createRequire(join(r, 'web/package.json'))('@playwright/test');
  }
  console.error('scoreboard: @playwright/test not found; run `npm --prefix web ci` in the main checkout');
  process.exit(2);
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
