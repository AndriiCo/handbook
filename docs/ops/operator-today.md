# Operator — today (2026-10-31, production root steps)

Deploys are the builder's (one a Pacific day, behind plant + staging,
AGENTS.md §7). Today (Saturday) is spent — next deploy is Sunday's #3.
Everything below runs AFTER Sunday's deploy (it carries the staging-proven
live-draft fixes: user-role chat, 32k budget, 8 records/call). Staging
proof 2026-10-31: 9/10, 17 facts via review flow, A2 pass/retake/retake,
A5 quote_found 0.7 (muse-spark-1.3). Run each as root, paste the whole
output back.

1. Proposals (`579f6552…af3322`):
   `bash ~/apostille/scripts/ops-gp2-proposals.sh` — seeds packs, fetches
   slugs, live-drafts with the Meta key. Expect ~8 vietnam + ~9 usa-wa-sos
   proposed (staging counts). Paste back: the before/after counts.
2. Approvals in `/review` (admin, your client cert): accept every proposed
   fact EXCEPT 2 per slug — leave 2 vietnam + 2 usa-wa-sos proposed for the
   video's approve scene. GP2-data needs >= 8 accepted each. Paste back:
   accepted counts per slug.
3. A2 (`01b87122…8e1fe` + `38cc2341…20ebc40`):
   `bash ~/apostille/scripts/ops-a2-record.sh` — one SAM 3.1 + one Spark
   call per synthetic sample (expect clean=pass, dark/wash=retake, 0 SAM
   boxes on flat shapes — validator accepts that). Paste back: the three
   verdict lines + token totals.
4. A5 (`5d553a90…fd0c6844f4a`):
   `bash ~/apostille/scripts/ops-a5-eval.sh` — live eval on the 26-case
   gold set (staging: quote_found 0.7). Paste back: the quality.json lines.

S1: switch LICENSE to MIT (text ready at opensource.org/licenses/MIT),
make the repo public, and link the demo video in README (video lands 11-03).

GP3 (scoreboard demands a logo inside every configured provider's button):
run `cargo xtask brand-assets` and follow its four download steps (Apple,
Google, Meta, FIDO — each owner's page, review-and-accept as Andrii Co.),
then `cargo xtask brand-assets --check --minutes 60` to confirm what landed
in ~/Downloads. Keep one SVG per provider prod configures (google.svg /
apple.svg; the Meta login button uses Facebook-Login assets, not the company
lockup; Google preferably needs no file — use its rendered button) in
web/src/lib/assets/brand/ (operator-owned, never committed), then redeploy
— the login page bundles them at build time via import.meta.glob and renders
text-only buttons until they land.
