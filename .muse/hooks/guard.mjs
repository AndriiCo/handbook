// PreToolUse: a seatbelt for the goal brief, the scoreboard, the skills and
// these hooks. Muse already gates writes to .agents/ and .muse/, but not
// under --yolo or the Unrestricted profile, so this refuses them in every
// profile. Reads (cat, grep, running the scoreboard) pass. Subagent
// worktrees live under .muse/worktrees/ and are ordinary files.
import { emit, readInput } from './lib.mjs';

const input = readInput();
const tool = String(input.tool_name ?? '');
const blob = JSON.stringify(input.tool_input ?? {}).replace(/\.muse\/worktrees\/[^/"'\s]+\//g, '');

const PROTECTED = /(^|[\s"'=/:])\.(agents|muse)\//;
const WRITE_TOOL = /edit|write|patch|create|delete|move|rename/i;
const SHELL_WRITES = [
  /(>>?|\btee\b(\s+-a)?)\s*["']?[^\s"'|;&]*\.(agents|muse)\//,
  /\b(sed\s+-i|perl\s+-\w*i|rm|mv|cp|truncate|chmod|ln|touch|git\s+(checkout|restore|rm|mv|stash))\b[^;&|\n]*\.(agents|muse)\//,
];

if (PROTECTED.test(blob) && (WRITE_TOOL.test(tool) || SHELL_WRITES.some((re) => re.test(blob)))) {
  emit({
    hookSpecificOutput: {
      hookEventName: 'PreToolUse',
      permissionDecision: 'deny',
      permissionDecisionReason:
        '.agents/ and .muse/ hold the goal, the scoreboard, the skills and the hooks; only the operator edits them. Fix the product instead, or propose the change in your final message.',
    },
  });
}
