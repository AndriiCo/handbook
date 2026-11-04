// GP2-page v2 (operator-approved 2026-11-03): the guide's values are
// traceable. Helpers shared by `scoreboard.mjs` (the live row), its
// node test (`web/test/gp2-v2.test.mjs`) and the CI port; only
// probeValueNodes touches the page (it renders auto subtrees).
//
// v1 asked for the first 40 characters of raw statements on the page,
// which kept drafter keys (`standard_fee: …`) in front of customers.
// v2 asks instead that:
//   - at least min(8, n) public facts (accepted, not `phone_script`)
//     sit on a visible value node, `[data-fact-ids~="<id>"]`;
//   - `body.innerText` holds no raw key: none at the start of a line
//     (RAW_KEY, /^\s*[a-z][a-z0-9_]*:\s/m) and none anywhere in a line
//     (plan §2's RAW_KEY_ANY, /\b[a-z][a-z0-9]*_[a-z0-9_]+:\s/, which
//     catches "#20 · authorized_issuer: …" record lines and a key
//     after a label on one line; the closed "How this page was made"
//     records are not rendered, so they are not in it);
//   - every visible `[data-fact-ids]` node shows some text.
// Both text checks read the page after `content-visibility:auto`
// subtrees are rendered (see probeValueNodes): Chromium leaves an
// off-screen auto subtree out of innerText while its boxes still
// measure, which would hide raw keys there and report its values empty.

/** A raw drafter key at the start of a line: `standard_fee: $15`. */
export const RAW_KEY = /^\s*[a-z][a-z0-9_]*:\s/m;

/**
 * Plan §2's raw-key rule: a drafter key (with an underscore) anywhere in
 * the text, `#14 · standard_fee: …` included. The CUJ specs read it too.
 */
export const RAW_KEY_ANY = /\b[a-z][a-z0-9]*_[a-z0-9_]+:\s/;

/** One line at a time, for the report only (`$` stands in for the newline). */
const RAW_KEY_LINE = /^\s*[a-z][a-z0-9_]*:(?:\s|$)/;
const RAW_KEY_ANY_LINE = /\b[a-z][a-z0-9]*_[a-z0-9_]+:(?:\s|$)/;

/**
 * Facts the guide must show: accepted (a fact without a status counts
 * as accepted, as the public read and the J1 fixture send them) and not
 * call-console scripts.
 * @param {unknown} facts the `/facts` array
 * @returns {{ id: unknown, topic?: unknown, status?: unknown }[]}
 */
export function publicFacts(facts) {
  return (Array.isArray(facts) ? facts : []).filter(
    (f) => f && typeof f === 'object' && (f.status ?? 'accepted') === 'accepted' && f.topic !== 'phone_script',
  );
}

/**
 * Lines of `text` that hold a raw key (at their start, or a drafter key
 * anywhere in them), trimmed.
 * @param {unknown} text `document.body.innerText`
 * @returns {string[]}
 */
export function rawKeyLines(text) {
  return String(text ?? '')
    .split('\n')
    .filter((line) => RAW_KEY_LINE.test(line) || RAW_KEY_ANY_LINE.test(line))
    .map((line) => line.trim());
}

/**
 * True when `text` holds a raw key by either rule.
 * @param {unknown} text
 */
export function hasRawKey(text) {
  const body = String(text ?? '');
  return RAW_KEY.test(body) || RAW_KEY_ANY.test(body);
}

/**
 * The in-page predicate: which fact ids sit on a visible value node,
 * which visible value nodes show no text, and the page text to check
 * for raw keys.
 *
 * It runs in the browser through `page.evaluate(probeValueNodes)`, so it
 * must not close over module scope: everything it uses is an argument
 * or a browser global. Node tests pass a small fake as `env`.
 *
 * First it renders every `content-visibility:auto` element (an inline
 * `content-visibility:visible !important`), as a reader who scrolls
 * there sees it; `revealed` counts them. `content-visibility:hidden`
 * (closed `<details>`, `hidden=until-found`) stays hidden. Then:
 *
 * Visible means rendered (not `display:none`, not inside a closed
 * `<details>`), a box larger than 1 px each way (an `sr-only` clip is
 * 1 px), not `visibility:hidden`/`collapse`, not opacity 0, on the page
 * (not pushed to negative coordinates), and no ancestor collapsed to a
 * zero-size box that clips its overflow.
 *
 * `text` is `document.body.innerText` read after that, for the raw-key
 * rule.
 *
 * @param {unknown} [_arg] unused (`page.evaluate` passes its argument here)
 * @param {any} [env] the window: `document`, `getComputedStyle`, `scrollX`, `scrollY`
 * @returns {{ nodes: number, visible: number, ids: string[], empty: string[], revealed: number, text: string }}
 */
export function probeValueNodes(_arg, env = globalThis) {
  const doc = env.document;
  const styleOf = (el) => env.getComputedStyle(el);
  let revealed = 0;
  for (const el of Array.from(doc.querySelectorAll('*'))) {
    if (styleOf(el).contentVisibility !== 'auto' || typeof el.style?.setProperty !== 'function') continue;
    el.style.setProperty('content-visibility', 'visible', 'important');
    revealed += 1;
  }
  const sized = (r) => Boolean(r) && r.width > 1 && r.height > 1;
  const clips = (s) => /hidden|clip|scroll|auto/.test(`${s.overflow ?? ''} ${s.overflowX ?? ''} ${s.overflowY ?? ''}`);
  const visible = (el) => {
    if (typeof el.checkVisibility === 'function' && !el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
    const r = el.getBoundingClientRect();
    if (!sized(r)) return false;
    const s = styleOf(el);
    if (s.display === 'none' || s.visibility === 'hidden' || s.visibility === 'collapse') return false;
    if (Number(s.opacity) === 0) return false;
    if (r.right + (env.scrollX || 0) <= 0 || r.bottom + (env.scrollY || 0) <= 0) return false;
    for (let up = el.parentElement; up && up !== doc.body && up !== doc.documentElement; up = up.parentElement) {
      const us = styleOf(up);
      if (us.display === 'none' || Number(us.opacity) === 0) return false;
      if (!sized(up.getBoundingClientRect()) && clips(us)) return false;
    }
    return true;
  };
  const all = Array.from(doc.querySelectorAll('[data-fact-ids]'));
  const ids = new Set();
  const empty = [];
  let shown = 0;
  for (const el of all) {
    if (!visible(el)) continue;
    shown += 1;
    const own = String(el.getAttribute('data-fact-ids') ?? '').split(/\s+/).filter(Boolean);
    for (const id of own) ids.add(id);
    if (!String(el.innerText ?? el.textContent ?? '').trim()) empty.push(own.join(' ') || '(none)');
  }
  const text = String(doc.body?.innerText ?? '');
  return { nodes: all.length, visible: shown, ids: Array.from(ids), empty, revealed, text };
}

/**
 * The v2 problems for one viewport. "no accepted facts to show" carries
 * no label, so a caller checking two widths can report it once.
 * @param {{ label: string, probe: Pick<ReturnType<typeof probeValueNodes>, 'ids' | 'empty'>, text: string, facts: unknown, min?: number }} input
 *   `facts` is the raw `/facts` array; `text` is `document.body.innerText`
 *   as the probe read it (`probe.text`, after rendering auto subtrees).
 * @returns {{ problems: string[], shown: number, want: number, total: number }}
 */
export function gp2PageProblems({ label, probe, text, facts, min = 8 }) {
  const problems = [];
  const wanted = publicFacts(facts);
  const want = Math.min(min, wanted.length);
  const onPage = new Set((probe?.ids ?? []).map(String));
  const missing = wanted.filter((f) => !onPage.has(String(f.id)));
  const shown = wanted.length - missing.length;
  if (!wanted.length) problems.push('no accepted facts to show');
  else if (shown < want) {
    const list = missing.slice(0, 6).map((f) => `#${f.id}`).join(' ');
    problems.push(`${label}: ${shown}/${want} facts on a visible [data-fact-ids] value (missing ${list}${missing.length > 6 ? ' …' : ''})`);
  }
  const body = String(text ?? '');
  if (hasRawKey(body)) {
    const raw = rawKeyLines(body);
    const shown = raw.slice(0, 3).map((line) => `"${line.slice(0, 60)}"`).join(', ');
    problems.push(`${label}: raw key text visible (${raw.length} line(s)): ${shown}`);
  }
  const empty = probe?.empty ?? [];
  if (empty.length) problems.push(`${label}: ${empty.length} visible [data-fact-ids] node(s) without text (ids ${empty.slice(0, 4).join(', ')})`);
  return { problems, shown, want, total: wanted.length };
}
