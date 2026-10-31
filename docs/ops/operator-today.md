# Operator — today (2026-10-31, production root steps)

Production runs `0.1.0+d93a58c` (deployed today under your explicit
override of the one-a-day rule; it carries the staging-proven live-draft
fixes: user-role chat, 32k budget, 8 records/call). Staging proof
2026-10-31: 9/10, 17 facts via review flow, A2 pass/retake/retake, A5
quote_found 0.7 (muse-spark-1.3). Run the four lines below in order, in
one root shell, and paste each command's whole output back.

```sh
ssh andrii.ai
sudo -i
```

1. Proposals (server sha256
   `579f655262f0f42c2e2657f766920fdf2029fbc1588fe2f48c7eb54248af3322`):

```sh
bash ~/apostille/scripts/ops-gp2-proposals.sh
```

Seeds packs, fetches slugs, live-drafts with the Meta key. Expect ~8
vietnam + ~9 usa-wa-sos proposed (staging counts). Paste back: the
before/after counts.
2. Approvals — no script: open the admin `/review` in a browser with your
   client certificate, accept every proposed fact EXCEPT 2 per slug
   (leave 2 vietnam + 2 usa-wa-sos proposed for the video's approve
   scene). GP2-data needs >= 8 accepted each. Paste back: accepted counts
   per slug.
3. A2 (server sha256
   `01b87122e6321cb8d38553ea3a3fb089ea738a3e8ffbc169957b321e7068e1fe`
   and
   `38cc23411ee0d99ffbf6f162fb886a7f71d261bef2f1e0961bd759f3120ebc40`):

```sh
bash ~/apostille/scripts/ops-a2-record.sh
```

One SAM 3.1 + one Spark call per synthetic sample (expect clean=pass,
dark/wash=retake, 0 SAM boxes on flat shapes — the validator accepts
that). Paste back: the three verdict lines + the DONE token totals.
4. A5 (server sha256
   `5d553a90d046cfbd5452dd26481d3de33d00be162581fa8ec4f00fd0c6844f4a`):

```sh
bash ~/apostille/scripts/ops-a5-eval.sh
```

Live eval on the 26-case gold set (staging: quote_found 0.7). Paste
back: the quality.json lines.

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

Inference workstream (side-chat built, main session to adopt as a goal
item — full design at docs/ops/plant-inference.md):
`cargo xtask plant-run` + `ops/plant-inference/` drive Qwen2.5-VL-7B
pilot jobs on plant's RTX 3060 (uncompiled/unproven from the side
chat — needs `cargo clippy -p xtask`, a live `setup`, and the
`job-001` run with its three numbers). Suggested slice order is §5
of that doc (gates → batch plumbing → lease endpoints → Together
pilot → Modal overflow).
