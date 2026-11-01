// PreToolUse: a seatbelt for the goal brief, the scoreboard, the skills and
// these hooks. Muse already gates writes to .agents/ and .muse/, but not
// under --yolo or the Unrestricted profile, so this refuses them in every
// profile. Reads (cat, grep, running the scoreboard) pass. Subagent
// worktrees live under .muse/worktrees/ and are ordinary files.
//
// It checks a file tool's target fields and a shell tool's raw command,
// never a file's text: a doc may say how to run the scoreboard. A script
// that writes .agents/ when run later is outside what a seatbelt can see.
import { emit, readInput } from './lib.mjs';

const input = readInput();
const tool = String(input.tool_name ?? '');
const toolInput = input.tool_input ?? {};
// `.muse/worktrees/<id>/` prefixes are ordinary files; `..` is not an id.
const strip = (s) => s.replace(/\.muse\/worktrees\/(?!\.\.?[/"'\s])[^/"'\s]+\//g, '');

const P = String.raw`\.(agents|muse)(?![\w.-])`;
const PROTECTED = new RegExp(String.raw`(^|[^\w.-])` + P);
const WRITE_TOOL = /edit|write|patch|create|delete|move|rename/i;
const SHELL_TOOL = /bash|shell|exec|command|terminal|run/i;
const SHELL_WRITES = [
  new RegExp(String.raw`(>>?|\btee\b(\s+-a)?)\s*["']?[^\s"'|;&]*` + P),
  new RegExp(String.raw`\b(sed\s+-i|perl\s+-\w*i|rm|mv|cp|rsync|install|truncate|chmod|ln|touch|unlink|git\s+(checkout|restore|rm|mv|stash|clean))\b[^;&|\n]*` + P),
];
// A tool's text fields; every other field, known or not, is a target. A
// prompt, goal or message that names .agents/ (a scheduled job, a subagent's
// task) is text, not a write.
const CONTENT = /^(content|find|replace|old_string|new_string|old_str|new_str|text|body|prompt|instructions|description|message|task|objective|summary|reason|note)$/i;
const targets = strip(JSON.stringify(Object.fromEntries(Object.entries(toolInput).filter(([k]) => !CONTENT.test(k)))));
// Raw strings keep real newlines, which end a command for SHELL_WRITES.
const command = SHELL_TOOL.test(tool)
  ? strip(Object.values(toolInput).filter((v) => typeof v === 'string').join('\n'))
  : '';

const fileWrite = WRITE_TOOL.test(tool) && PROTECTED.test(targets);
const shellWrite = PROTECTED.test(command) && SHELL_WRITES.some((re) => re.test(command));
if (fileWrite || shellWrite) {
  emit({
    hookSpecificOutput: {
      hookEventName: 'PreToolUse',
      permissionDecision: 'deny',
      permissionDecisionReason:
        '.agents/ and .muse/ hold the goal, the scoreboard, the skills and the hooks; only the operator edits them. Fix the product instead, or propose the change in your final message.',
    },
  });
}
