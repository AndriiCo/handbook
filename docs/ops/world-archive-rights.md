# World-archive rights and access review (RO-4)

Operator-ordered addendum work (ADDENDUM 2026-11-02).
Read-only research. No code changed. No production contact. No secrets.

Method: polite one-off fetches with `handbook-rights-review/0.1`
(one request per page, small sleeps, no crawl). Terms notes below are
firsthand where marked [seen]; otherwise secondhand via search and marked
[not seen live] — confirm before relying on them in a dispute.

## 1. HCCH website terms (status table + competent-authority pages)

Source: [HCCH Disclaimer](https://www.hcch.net/en/disclaimer/) [seen 2026-11-02,
HTTP 200]:

> "Reproduction of the information is authorised, except for commercial
> purposes, provided the source is duly acknowledged."

What this covers:

- The Apostille status table (`/en/instruments/conventions/status-table/?cid=41`)
  and the competent-authority pages (`/authorities1/?cid=41`) are website
  *information*, so short attributed reuse (party names, authority names/URLs,
  effective dates) in non-commercial research output is authorised by the
  notice itself. Acknowledgement is mandatory — keep an "HCCH source" credit
  on every derived CSV/page.
- It does NOT cover HCCH *publications* (Guides, handbooks, eBooks): those
  carry "all rights reserved / prior written permission" sale terms
  (July 2025 digital/print terms). Do not bundle publication PDFs or long
  excerpts into the archive package.
- Commercial use is excluded. The submission/judges use is non-commercial
  research; any later commercial reuse needs fresh permission.

HCCH robots (`https://www.hcch.net/robots.txt`, [seen]): `*` is allowed
except `/admin/`, `/WEB-INF/`, and `*/search-results/`; `Crawl-delay: 10`.
The status-table and authority pages are NOT disallowed. Honour the
10-second delay for hcch.net (stricter than the crawler spec's 1 req/2 s).

## 2. Per-country table (packs coverage + method for the rest)

Packs today touch 3 countries plus the HCCH multilateral source:
USA (federal + CA/FL/NY/TX/WA), Egypt (consular posts in the US),
Viet Nam (MOFA). Robots checks below are firsthand [seen 2026-11-02];
terms notes are marked per row.

| # | Country / source | Governing terms / copyright note | Robots status [seen] |
|---|---|---|---|
| 1 | HCCH (multilateral; status table + authority pages) | Disclaimer: non-commercial reproduction OK with acknowledgement [seen]. Publications all-rights-reserved (do not archive) | `*` allowed except /admin/, /WEB-INF/, search-results; Crawl-delay 10 |
| 2 | USA, federal (`travel.state.gov`) | State Dept copyright notice: info in the public domain unless a copyright is indicated; indicated photos/graphics need owner permission [not seen live] | Not retrievable from here: `robots.txt` returns Cloudflare 403 to this egress. Resolve at crawl time from the crawl origin; FAIL CLOSED |
| 3 | USA, California (`www.sos.ca.gov`) | ca.gov standard use policy: site information generally public domain unless indicated; some copyrighted data (e.g. photos) may need permission [not seen live] | 200, empty body = no restrictions stated |
| 4 | USA, Florida (`dos.fl.gov`) | No terms reviewed [not seen live] — treat as unknown/restrictive until read | 404 (no robots file = no stated restriction) |
| 5 | USA, New York (`dos.ny.gov`) | No terms reviewed [not seen live] | Allowed for content paths; admin/search/user/login paths disallowed (standard Drupal set) — pack URIs unaffected |
| 6 | USA, New York (`www.businessexpress.ny.gov`) | No terms reviewed [not seen live] | **`Disallow: /` (whole host). The pack seed `…/app/answers/cms/a_id/3423` is currently DISALLOWED — SA-8 must log it disallowed and skip; consider a replacement URI** |
| 7 | USA, Texas (`www.sos.state.tx.us`) | No terms reviewed [not seen live] | 200, empty body = no restrictions stated |
| 8 | USA, Washington (`www.sos.wa.gov`) | No terms reviewed [not seen live] | Allowed for content paths incl. the apostille FAQ/resource pages; `/form/` and admin paths disallowed — pack URIs unaffected |
| 9 | Egypt (`egyptembassy.net`) | No terms reviewed [not seen live] | Only `/3pv/` disallowed — pack URIs unaffected |
| 10 | Egypt (`www.egyconsulatela.com`) | No terms reviewed [not seen live] | `robots.txt` 302 (redirect, no usable file). Resolve final target at crawl time; FAIL CLOSED |
| 11 | Egypt (`www.egypt-nyc.com`, domains-only, no pack seed) | No terms reviewed [not seen live] | **`Disallow: /` (whole host). No pack seed points here today; any future seed must be skipped** |
| 12 | Viet Nam (`mofa.gov.vn`, `dichvucong.mofa.gov.vn`) | No English terms statement reviewed [not seen live] — assume restrictive; short factual quotes only | Both allow (`Disallow:` empty); sitemap advertised on mofa.gov.vn |

Coverage count (honest): **3 of ~130 Contracting Parties covered**
(USA, Egypt, Viet Nam) + the HCCH source itself; **12 hosts robots-checked
firsthand, 1 terms page read firsthand** (HCCH disclaimer), 2 terms notes
secondhand (US State Dept, California).

Method for the rest: as SA-7 emits each `packs/world/<ISO2>.json`, the
crawler resolves that party's hosts and (a) fetches `robots.txt` at crawl
time with FAIL-CLOSED semantics already in the SA-8 spec, and (b) records
the site's terms/copyright URL (or "none found") into `coverage.csv` notes
before the run. No party publishes openly until its row has both fields.
Spot-check-only updates to this doc; per-party detail lives in the CSV.

## 3. Recommendation: what publishes openly vs behind a judges-only token

Principle: *facts and short attributed quotes travel openly; verbatim
third-party payloads do not.* Only rows 1–3 have reuse terms permissive
enough for open redistribution, and even then only for metadata/quotes —
never full page mirrors.

Publish OPENLY (public repo / hermes site, no token):

- `authorities.csv`, `coverage.csv`, `manifest.csv`, `SHA256SUMS`,
  `index.html`, `cdx/index.cdxj` (URLs + metadata are facts).
- Short attributed quotes (authority names, URLs, fees, steps) with the
  HCCH acknowledgement and per-row source URIs. US-federal quotes are
  public-domain; HCCH quotes satisfy the disclaimer (non-commercial +
  acknowledged); all other parties: minimal factual quotes only.

Behind a JUDGES-ONLY token (expiring, access-logged, per `crates/web`
access rules; no public link):

- `warc/` payloads and per-country + world `.wacz` files (verbatim copies
  of third-party pages, including sites with unknown/restrictive terms
  and third-party images).
- `text/*.txt` full-text extractions (same reason — full reproduction).
- Any response record from a host whose robots verdict was marginal or
  whose terms forbid redistribution; token-gating does not cure a
  violation, so FAIL-CLOSED skip remains the default and gated inclusion
  needs an explicit operator note per host.

Two pre-run actions for the operator/IT lead:

1. Approve the open-vs-token split above (approval-only; no changes asked).
2. Note the two `Disallow: /` findings (businessexpress.ny.gov seed,
   egypt-nyc.com host): crawler skips are expected and are not errors.
