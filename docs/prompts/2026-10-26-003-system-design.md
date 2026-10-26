# 003 — System design proposal

- Prompt timestamp (UTC): 2026-10-26 ~18:20 (approximate, same session)
- Response timestamp (UTC): 2026-10-26 18:25:43
- Author: Andrii Tereshchenko
- Agent/model: Muse Code, powered by Meta Muse Spark

## Prompt

Propose a system design for a our project. We need to keep a track of various official sources of infromation. It will be websites (both official websites of bodies and some third-parties without restrictive robots.txt), PDF and others files (maybe even scans), photos from customers (from locations, street signs, parking lot directions), maybe videos for visit instructions, emails received (offical and from customer)  and comments with corrections proposed from our sites. Do not overengineer the database, use something simple as SQLLite for metadata, store files and emails locally (maybe with compression or batching, like a mbox or Brotolli compreses tar archives or similar one). Make it still easy to access if we need to find conflicting facts or statements. Index the files by the office / jurisdiction it applies. Have an audit history for changes made to apostille handbook or algorithms used to regenerate it (git commit ids, release / build  numbers and date/time). One of WoW type of features will be call transcripts recognition. We must be able to make a call to first source of infromation, ask for permission to record the call and transcribe, then human will ask the questions based on script prepared by our app and we will have answers trascribed and used to improve the content of the Handbook. Have a task tracker designed for corrections / validaitons to be applied to the handbook pages. Make sure there is no duplicate requests for the same information or authority (batch multiple requests - like a "What's your correct shipping address?" and "Are you closed early on 2nd  Wednesday every month or it's one-time poster on your front door?" to be asked during one phone call (to not overwhelm the help desk).  Have details on support channels that authorities are using - both public to steer the customers to (team / volume based) and some private channels for us to validate information for posting on website (specific emails / phones / extension numbers, etc.).  For websites store the entire page in .warc format with all the data on SSL certificates used in such a way that it will be possible to confirm authenticity of page, for emails make sure to have indication if it was signed by GMail/Outlook DMARC and store the data about selectors used to validate it. Have infromation if this is one of official email/website domains or fake one. We do not want bad actors to make incorrect edits to our handbook.  This will be first step - build an instracture to collect the data to reason about. Text / Photos / phone call transcripts / emails  and all the details on how to validate them. Use efficient disk and memory. Keep it as simple as possible (per AGENTS.md). Technology stack is to be based on Rust language (including any build/maintenance tools). It will be deplyed on one server, do not worry about scalling for now.

## Response

Design-only turn: no code written. Proposed the "Evidence Store"
(Step 1: collection infrastructure) in `docs/design.md`:

- One-file SQLite metadata DB (11 tables: authority, source, domain,
  channel, capture, fact, capture_fts, task, batch, call_session, build,
  audit), every row indexed by office/jurisdiction.
- Local file store: one `.warc.gz` per web capture, per-source monthly
  mbox (Brotli once sealed) for email, content-addressed blobs for
  files/scans/photos/videos/transcripts, extracted-text sidecars indexed
  by FTS5 for conflict hunting.
- Authenticity: TLS chain + validation result in WARC warcinfo;
  SPF/DKIM (with selectors) + DMARC verdicts per email; official vs fake
  domain table; trust tiers (official/staff_verified/community/rejected)
  with corroboration-or-staff-approval promotion, all promotions audited.
- Tasks with `dedup_key` unique index (no duplicate live requests),
  grouped into per-authority-per-channel batches (one call, many
  questions); public vs private support channels separated.
- Call workflow: app-prepared script from batched tasks, consent gating
  (no consent → no recording, tasks reopened), self-hosted whisper-rs
  transcription, transcript → proposed facts.
- Conflicts as a SQL query + `xtask review-conflicts`; append-only audit
  log and build manifests (git commit, build number, facts hash,
  timestamp) tracing any page to exact inputs and code.
- Rust-only, phased crates/xtask plan, single server, streaming ingest
  and on-demand derivatives for disk/memory efficiency.
- Requirements traceability table mapping every prompt clause to a
  design section; assumptions and 4 open questions for the author
  (content repo layout, whisper vs API, retention limits, pilot
  authority).

Files changed: `docs/design.md` (new), `README.md` (design pointer),
`docs/prompts/README.md` (index), this entry. No code changes; no
build/test verification applicable (verified: `git status` clean after
commit). No push (not asked this turn, per AGENTS.md §5).

## Commits

- This entry's own commit: the commit adding this file (see
  `git log -- docs/prompts/2026-10-26-003-system-design.md`).

## Follow-ups

- Author to answer the 4 open questions in `docs/design.md` §11.
- Implementation (schema + store + first xtask commands) awaits approval
  of the proposal.
