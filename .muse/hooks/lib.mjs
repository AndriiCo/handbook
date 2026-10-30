// Shared helpers for the hooks in this directory.
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

// Hook input arrives as one JSON document on stdin.
export function readInput() {
  try {
    return JSON.parse(readFileSync(0, 'utf8') || '{}');
  } catch {
    return {};
  }
}

export function git(...args) {
  return execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
}

export function emit(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n');
}
