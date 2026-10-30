// Notification: Muse is waiting for you (an approval or a question). In the
// first 80 hours, 46 were idle gaps waiting on the operator; make the wait
// visible at once.
import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { readInput } from './lib.mjs';

const input = readInput();
const msg = String(input.message ?? 'Muse is waiting for you').replace(/\s+/g, ' ').slice(0, 180);
spawnSync('osascript', ['-e', `display notification ${JSON.stringify(msg)} with title "Muse - handbook" sound name "Glass"`]);

// Optional phone push: put an ntfy.sh topic name in ~/.config/muse/ntfy-topic.
// Only a generic line is sent, never the message itself (topics are public).
const topicFile = join(homedir(), '.config/muse/ntfy-topic');
if (existsSync(topicFile)) {
  const topic = readFileSync(topicFile, 'utf8').trim();
  if (/^[\w-]{12,64}$/.test(topic)) {
    await fetch(`https://ntfy.sh/${topic}`, { method: 'POST', body: 'Muse (handbook) is waiting for you' }).catch(() => {});
  }
}
