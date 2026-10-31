# Operator — today (2026-10-30, fixes 2-6 slice)

Deploys are the builder's now (one a Pacific day, behind plant + staging,
AGENTS.md §7). Nothing below needs root, and you run no root scripts today.

Deferred until fixes 2, 3 and 5 ship (Deploy days, `.agents/goal.md`):
- Proposals: `ops-gp2-proposals.sh` (needs fix 2's pack coverage + fix 3's
  live-draft path) — do not run yet.
- A2 recording: `ops-a2-record.sh` (needs fix 5's script repairs) — do not
  run yet.
- A5 eval: `ops-a5-eval.sh` (needs fix 5's staged-`SHA` support) — do not
  run yet.

Then: approve N proposals for vietnam and usa-wa-sos in `/review`
(GP2-data passes at >= 8 accepted facts each; keep at least 2 per slug
proposed for the video's approve scene).

S1: switch LICENSE to MIT (text ready at opensource.org/licenses/MIT),
make the repo public, and link the demo video in README (video lands 11-03).

GP3 (scoreboard demands a logo inside every configured provider's button):
drop the official google.svg / apple.svg / meta.svg (one per provider prod
configures, each owner's current artwork per its guidelines) into
web/src/lib/assets/brand/, then redeploy — the login page bundles them at
build time via import.meta.glob and renders text-only buttons until they land.
