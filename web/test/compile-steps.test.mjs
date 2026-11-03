import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
	COMPILE_STEPS,
	activeIndex,
	applyStep,
	countChips,
	detailText,
	fmtElapsed,
	initialSteps,
	normalizeRuns,
	parseSeeds,
	sentence,
	settleDone,
	settleFailed,
	startFailKind,
	stepsFromSummary,
	summaryLine
} from '../src/lib/admin/compile-steps.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => JSON.parse(readFileSync(join(HERE, '..', 'e2e', 'fixtures', name), 'utf8'));
const runsBody = fixture('admin-compile-runs.json');
const liveEvents = fixture('admin-ws-compile.json');
const statuses = (steps) => COMPILE_STEPS.map((s) => steps[s.id].status).join(' ');

test('steps render in server order with the staff labels', () => {
	assert.deepEqual(
		COMPILE_STEPS.map((s) => s.id),
		['capture', 'extract', 'cross_check', 'offices', 'compose']
	);
	assert.equal(COMPILE_STEPS[1].label, 'Draft facts (Muse Spark)');
	assert.equal(statuses(initialSteps()), 'pending pending pending pending pending');
});

test('step events fold in order and keep what earlier reports said', () => {
	let steps = initialSteps();
	for (const ev of liveEvents) if (ev.type === 'compile.step') steps = applyStep(steps, ev);
	assert.equal(statuses(steps), 'done running pending pending pending');
	assert.equal(steps.capture.latency_ms, 4820);
	assert.equal(steps.extract.counts.captures, 2);
	assert.equal(activeIndex(steps), 1);

	// Done carries the full counts; the running count is replaced.
	steps = applyStep(steps, {
		step: 'extract',
		status: 'done',
		counts: { captures: 2, proposed: 5, rejected: 1, no_text: 0, failed: 0, waiting: 0 },
		latency_ms: 61200
	});
	assert.deepEqual(steps.extract.counts, { captures: 2, proposed: 5, rejected: 1, no_text: 0, failed: 0, waiting: 0 });
	assert.equal(steps.extract.detail, null);

	// A sub-step string shows only while the step runs.
	steps = applyStep(steps, { step: 'compose', status: 'running', detail: 'entail' });
	assert.equal(steps.compose.detail, 'entail');
	assert.equal(detailText(steps.compose.detail), 'Muse Wonder checking each block');
	steps = applyStep(steps, { step: 'compose', status: 'skipped', reason: 'needs at least 3 accepted facts (has 1)' });
	assert.equal(steps.compose.detail, null);
	assert.equal(steps.compose.reason, 'needs at least 3 accepted facts (has 1)');
});

test('unknown steps and statuses change nothing', () => {
	const steps = initialSteps();
	assert.equal(applyStep(steps, { step: 'publish', status: 'done' }), steps);
	assert.equal(applyStep(steps, { step: 'capture', status: 'exploded' }), steps);
	assert.equal(applyStep(steps, {}), steps);
});

test('a stored summary rebuilds the run (reopened links)', () => {
	const done = runsBody.runs.find((r) => r.run_id === 88);
	const steps = stepsFromSummary(done.summary);
	assert.equal(statuses(steps), 'done done done done done');
	assert.deepEqual(
		countChips('capture', steps.capture).map((c) => c.text),
		['4 pages', '2 new', '2 unchanged']
	);
	assert.deepEqual(
		countChips('compose', steps.compose).map((c) => c.text),
		['plan v5', '6 blocks', '1 dropped']
	);
	assert.deepEqual(countChips('offices', steps.offices).map((c) => c.text), ['1 office proposed']);
	assert.equal(steps.compose.latency_ms, 14630);
	assert.equal(steps.capture.latency_ms, null);

	const thin = stepsFromSummary(runsBody.runs.find((r) => r.run_id === 85).summary);
	assert.equal(statuses(thin), 'done skipped skipped skipped skipped');
	assert.equal(thin.extract.reason, 'no new or undrafted capture');
	assert.equal(thin.cross_check.reason, 'no new proposals');
	assert.equal(thin.compose.reason, 'needs at least 3 accepted facts (has 1)');

	const capped = stepsFromSummary({
		capture: { pages: 1 },
		extract: { captures: 1, proposed: 0 },
		cross_check: { captures: 0 },
		offices: { status: 'skipped' },
		plan: { status: 'skipped' },
		stopped_by_cap: true
	});
	assert.equal(capped.cross_check.reason, 'daily model-call cap reached');
	assert.equal(capped.compose.reason, 'daily model-call cap reached');
	assert.equal(statuses(stepsFromSummary(null)), 'pending pending pending pending pending');
});

test('settleDone keeps live latency and fills what the socket missed', () => {
	let steps = initialSteps();
	steps = applyStep(steps, { step: 'capture', status: 'done', counts: { pages: 4 }, latency_ms: 3000 });
	steps = applyStep(steps, { step: 'extract', status: 'running', counts: { captures: 2 } });
	const done = settleDone(steps, runsBody.runs.find((r) => r.run_id === 88).summary);
	assert.equal(statuses(done), 'done done done done done');
	assert.equal(done.capture.latency_ms, 3000);
	assert.equal(done.extract.counts.proposed, 6);
	// Nothing reported and nothing stored: it did not run.
	assert.equal(statuses(settleDone(initialSteps(), {})), 'skipped skipped skipped skipped skipped');
});

test('settleFailed fails the running step and skips the rest', () => {
	let steps = applyStep(initialSteps(), { step: 'capture', status: 'done', counts: { pages: 2 } });
	steps = applyStep(steps, { step: 'extract', status: 'running' });
	assert.equal(statuses(settleFailed(steps)), 'done failed skipped skipped skipped');
	// Failed between steps: the first unreported one is where it stopped.
	const between = applyStep(initialSteps(), { step: 'capture', status: 'done' });
	assert.equal(statuses(settleFailed(between)), 'done failed skipped skipped skipped');
	assert.equal(statuses(settleFailed(initialSteps())), 'failed skipped skipped skipped skipped');
});

test('count chips skip zeros and flag failures', () => {
	const chips = countChips('extract', {
		...initialSteps().extract,
		counts: { captures: 1, proposed: 0, rejected: 0, no_text: 1, failed: 2, waiting: 3 }
	});
	assert.deepEqual(
		chips.map((c) => `${c.text}${c.warn ? '!' : ''}`),
		['1 capture', '0 proposed', '1 no text', '2 failed!', '3 waiting']
	);
	assert.deepEqual(
		countChips('compose', { ...initialSteps().compose, summary: { version: 2, blocks: 1, dropped: 0, passed: false } }).map(
			(c) => c.text
		),
		['plan v2', '1 block', 'needs staff']
	);
	assert.deepEqual(countChips('offices', initialSteps().offices), []);
});

test('summary line names what landed in review', () => {
	assert.equal(summaryLine(runsBody.runs.find((r) => r.run_id === 88).summary), '4 pages · 6 facts proposed · 1 office · plan v5');
	assert.equal(summaryLine(runsBody.runs.find((r) => r.run_id === 85).summary), '2 pages · no new facts · no plan');
	assert.equal(summaryLine({ error: 'interrupted' }), 'Interrupted');
	assert.equal(summaryLine(undefined), '');
});

test('extra URLs: https only, at most five, duplicates folded', () => {
	assert.deepEqual(parseSeeds(''), { urls: [], errors: [] });
	assert.deepEqual(parseSeeds(' https://www.sos.wa.gov/apostille \n\nhttps://www.sos.wa.gov/apostille'), {
		urls: ['https://www.sos.wa.gov/apostille'],
		errors: []
	});
	assert.deepEqual(parseSeeds('http://example.gov/x').errors, ['Line 1 is not an https:// URL.']);
	assert.deepEqual(parseSeeds('https://a.example\nnot a url').errors, ['Line 2 is not an https:// URL.']);
	const six = Array.from({ length: 6 }, (_, i) => `https://example.gov/${i}`).join('\n');
	assert.deepEqual(parseSeeds(six).errors, ['At most 5 URLs per run (this list has 6).']);
	assert.equal(parseSeeds(`https://example.gov/${'a'.repeat(2048)}`).errors.length, 1);
});

test('runs list: typed rows, counter, bad rows dropped', () => {
	const runs = normalizeRuns(runsBody);
	assert.equal(runs.today, 2);
	assert.equal(runs.daily_runs, 6);
	assert.deepEqual(
		runs.runs.map((r) => `${r.run_id}:${r.status}`),
		['91:running', '88:done', '85:done', '82:failed']
	);
	assert.equal(runs.runs[0].finished_at, null);
	assert.deepEqual(normalizeRuns({ runs: [{ slug: 'x' }, null, { run_id: 3, slug: '' }] }).runs, []);
	assert.deepEqual(normalizeRuns('<html>'), { runs: [], today: 0, daily_runs: 0 });
});

test('start refusals map to one next action', () => {
	assert.equal(startFailKind(404), 'unknown');
	assert.equal(startFailKind(409), 'conflict');
	assert.equal(startFailKind(422), 'seeds');
	assert.equal(startFailKind(429), 'cap');
	assert.equal(startFailKind(500), 'error');
	assert.equal(startFailKind(0), 'error');
	assert.equal(sentence('a Compile run is already going for this authority'), 'A Compile run is already going for this authority');
	assert.equal(sentence('  '), null);
});

test('elapsed timer format', () => {
	assert.equal(fmtElapsed(0), '0:00');
	assert.equal(fmtElapsed(75_000), '1:15');
	assert.equal(fmtElapsed(3_725_000), '1:02:05');
	assert.equal(fmtElapsed(null), '');
	assert.equal(fmtElapsed(-5), '');
});
