# evals/world — recorded HCCH fixtures (public text)

Fixtures for `cargo xtask world-index` (SA-7). No live network in tests:
every unit test in `xtask/src/world.rs` reads these files.

## Provenance

Recorded 2026-11-02 with plain `curl` from the public HCCH site
(`https://www.hcch.net`, English pages only). Files are byte-identical
to the fetch except this README:

- `status-table-cid41.html` — the full 1961 Convention status table
  (`/en/instruments/conventions/status-table/?cid=41`, 130 parties).
- `authority-sid<NN>.html` — one competent-authority index page per
  fixture party (`.../status-table/authority/?sid=<NN>&cid=41`).
- `details-aid<MMM>.html` — the `details3` pages those index pages
  link to (`/en/states/authorities/details3/?aid=<MMM>`), where the
  authority names and website URLs live.

## Fixture parties (8)

Albania (sid 18, aid 305), Belarus (10/309), Cook Islands (211/632),
France (39/321), Japan (47/328), Türkiye (74/350),
United States of America (76/353), Viet Nam (230/1240).

Picked for shape coverage: plain `<em>` lists (Belarus), a lone
parenthetical `<em>` beside plain-text names (Albania), no `<em>` at
all with numbered plain-text designations (Viet Nam, Türkiye),
`<em>`-only single designations (Cook Islands, Japan), a variant
`General website (see also …)` label plus `Useful Links` rows (USA),
extra `Useful Links` URLs (France, Cook Islands), a non-ASCII party
name (Türkiye), and a long formal party name (United States of
America).

## Re-recording

Re-fetch the three URL shapes above (polite: one request per two
seconds, identified user agent) and drop the bytes in here under the
same names. Keep the party set stable unless the parser changes —
test expectations in `xtask/src/world.rs` pin exact names and URLs.
