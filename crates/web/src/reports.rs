//! Walk-in visitor reports (SA-21 lane).
//!
//! Signed-in visitors file a "walk-in report" about an issuing
//! office: free text plus up to [`MAX_PHOTOS`] photos. Photos are
//! checked by magic bytes ([`classify_photo`]) and JPEG EXIF/App
//! segments are stripped ([`strip_exif`]) before storage. Nothing is
//! shown publicly unmasked: [`display_text`] renders rows whose
//! masked text is not ready as `"awaiting masking"` and no hosted
//! model or media call happens on this path.
//!
//! Staff can turn a masked report into a cited "visitor report" fact
//! proposal ([`promote_to_visitor_fact`]); the proposal row carries
//! the citation, and the accept path (builder-owned) decides what an
//! accepted fact looks like.
//!
//! New tables arrive ONLY via `CREATE TABLE IF NOT EXISTS` (hard
//! rule: never `ALTER`/`DROP`/`RENAME`, never touch
//! `SCHEMA_VERSION`). [`migrate_report_tables`] is idempotent; the
//! builder calls it beside the existing migrations at boot (exact
//! snippet returned to the builder, not pasted here — `main.rs` is
//! builder-owned).
//!
//! Rate limits: the submit handler allows [`SUBMIT_PER_MINUTE`]
//! submits per IP per minute plus a tighter per-account bucket
//! ([`SUBMIT_PER_HOUR_PER_ACCOUNT`]/hour).
//!
//! Photos can also upload ahead of the submit, the moment the
//! visitor picks them ([`stage_upload`]): the bytes are checked and
//! EXIF-stripped exactly as on submit, held in `report_upload` under
//! the visitor's account, and the submit names them by id. Staged
//! uploads nobody submits are pruned after [`UPLOAD_TTL_SECS`].

use std::net::SocketAddr;
use std::sync::LazyLock;

use axum::{
    Json,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::json;

use crate::{AppState, admin::AdminAuth, passkey, store_unavailable, unix_now};

/// Most photos accepted on one report.
pub const MAX_PHOTOS: usize = 3;

/// Largest accepted photo: 8 MiB covers phone photos; anything
/// bigger is a staff-assisted ingest.
pub const MAX_PHOTO_BYTES: usize = 8 * 1024 * 1024;

/// Longest accepted report body (characters).
pub const MAX_BODY_CHARS: usize = 2_000;

/// Submits per IP per minute on the report submit path.
pub const SUBMIT_PER_MINUTE: u32 = 5;

/// Submits per signed-in account per hour (abuse ceiling).
pub const SUBMIT_PER_HOUR_PER_ACCOUNT: u32 = 20;

/// Staged uploads per IP per minute.
pub const UPLOAD_PER_MINUTE: u32 = 20;

/// Most staged (not yet submitted) uploads one account may hold.
pub const MAX_STAGED_PER_ACCOUNT: i64 = 12;

/// A staged upload no submit claimed is pruned after a day.
pub const UPLOAD_TTL_SECS: i64 = 24 * 3600;

/// Placeholder shown wherever a report has no masked text yet. No
/// unmasked body or photo is ever served to signed-out readers, and
/// even signed-in readers see only this marker until masking lands.
pub const AWAITING_MASKING: &str = "awaiting masking";

/// Citation carried by every promoted visitor-report proposal.
pub const VISITOR_REPORT_SOURCE: &str = "visitor report";

/// Photo kinds accepted on reports, by magic bytes (the file
/// extension and any client `Content-Type` are ignored).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoKind {
    Jpeg,
    Png,
    Webp,
    Gif,
}

/// Rate buckets for the submit path (separate map: a flood of
/// report IPs must never fill auth's map and fail sign-in closed).
static SUBMIT_LIMITS: LazyLock<passkey::RateBuckets> = LazyLock::new(passkey::RateBuckets::default);

/// Create `report`, `report_photo` and `report_promotion` when
/// missing. Idempotent: safe to call on every boot beside
/// [`handbook_store::db::migrate`].
pub fn migrate_report_tables(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS report(
           id INTEGER PRIMARY KEY,
           user_id TEXT NOT NULL DEFAULT '',
           authority_slug TEXT NOT NULL DEFAULT '',
           body TEXT NOT NULL DEFAULT '',
           masked_body TEXT,
           created_at INTEGER NOT NULL,
           masked_at INTEGER,
           status TEXT NOT NULL DEFAULT 'open'
         );
         CREATE INDEX IF NOT EXISTS idx_report_authority
           ON report(authority_slug);
         CREATE TABLE IF NOT EXISTS report_photo(
           id INTEGER PRIMARY KEY,
           report_id INTEGER NOT NULL REFERENCES report(id),
           kind TEXT NOT NULL DEFAULT '',
           filename TEXT NOT NULL DEFAULT '',
           bytes BLOB NOT NULL,
           sha256_hex TEXT NOT NULL DEFAULT '',
           exif_stripped INTEGER NOT NULL DEFAULT 0,
           masked INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_report_photo_report
           ON report_photo(report_id);
         CREATE TABLE IF NOT EXISTS report_promotion(
           id INTEGER PRIMARY KEY,
           report_id INTEGER NOT NULL REFERENCES report(id),
           authority_slug TEXT NOT NULL DEFAULT '',
           statement TEXT NOT NULL DEFAULT '',
           source TEXT NOT NULL DEFAULT 'visitor report',
           created_by TEXT NOT NULL DEFAULT '',
           created_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS report_upload(
           id TEXT PRIMARY KEY,
           user_id TEXT NOT NULL DEFAULT '',
           filename TEXT NOT NULL DEFAULT '',
           bytes BLOB NOT NULL,
           created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_report_upload_user
           ON report_upload(user_id);",
    )
}

/// Classify photo bytes by magic number. `Some(kind)` for JPEG,
/// PNG, WebP (`RIFF....WEBP`) and GIF (`GIF87a`/`GIF89a`); `None`
/// for anything else (extension and declared MIME are ignored).
pub fn classify_photo(bytes: &[u8]) -> Option<PhotoKind> {
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some(PhotoKind::Jpeg);
    }
    if bytes.len() >= 8 && bytes[0..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some(PhotoKind::Png);
    }
    if bytes.len() >= 12 && bytes[0..4] == *b"RIFF" && bytes[8..12] == *b"WEBP" {
        return Some(PhotoKind::Webp);
    }
    if bytes.len() >= 6 && (bytes[0..6] == *b"GIF87a" || bytes[0..6] == *b"GIF89a") {
        return Some(PhotoKind::Gif);
    }
    None
}

/// Strip JPEG metadata segments (`APPn`, which carry EXIF/XMP, and
/// the `COM` comment segment) so stored bytes carry no camera,
/// location or timestamp metadata. Non-JPEG input (or input that
/// does not parse as JPEG) is returned byte-identical; the return
/// flag reports whether any segment was removed.
pub fn strip_exif(bytes: &[u8]) -> (Vec<u8>, bool) {
    if classify_photo(bytes) != Some(PhotoKind::Jpeg) {
        return (bytes.to_vec(), false);
    }
    let Some(out) = strip_jpeg_segments(bytes) else {
        return (bytes.to_vec(), false);
    };
    let stripped = out.len() != bytes.len();
    (out, stripped)
}

/// Remove `APPn`/`COM` segments from a JPEG byte string. `None`
/// when the input does not parse (caller keeps the original).
fn strip_jpeg_segments(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..2]);
    let mut i = 2;
    let mut removed = false;
    let mut clean_end = false;
    while i < bytes.len() {
        if bytes[i] != 0xFF {
            // Segment expected, entropy found: malformed.
            return None;
        }
        // Skip fill bytes; the marker is the first non-0xFF byte.
        let mut j = i + 1;
        while j < bytes.len() && bytes[j] == 0xFF {
            j += 1;
        }
        if j >= bytes.len() {
            return None;
        }
        let marker = bytes[j];
        if marker == 0x00 {
            // Stuffed byte outside entropy: malformed.
            return None;
        }
        // Standalone markers: no length field, copy and continue.
        if marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
            out.extend_from_slice(&bytes[i..j + 1]);
            i = j + 1;
            continue;
        }
        // End-of-image: copy it and whatever trails, then done.
        if marker == 0xD9 {
            out.extend_from_slice(&bytes[i..]);
            clean_end = true;
            break;
        }
        // Start-of-scan: the rest is entropy-coded image data.
        if marker == 0xDA {
            out.extend_from_slice(&bytes[i..]);
            clean_end = true;
            break;
        }
        if j + 3 > bytes.len() {
            return None;
        }
        let len = u16::from_be_bytes([bytes[j + 1], bytes[j + 2]]) as usize;
        if len < 2 || j + 1 + len > bytes.len() {
            return None;
        }
        let drop = (0xE0..=0xEF).contains(&marker) || marker == 0xFE;
        removed |= drop;
        if !drop {
            out.extend_from_slice(&bytes[i..j + 1 + len]);
        }
        i = j + 1 + len;
    }
    if !clean_end && i < bytes.len() {
        return None;
    }
    if !removed {
        return Some(bytes.to_vec());
    }
    Some(out)
}

/// Mask one line of visitor text for display: email addresses and
/// long digit runs (phone/order numbers) become placeholders.
/// Pure and dependency-free; the masked copy is what staff and
/// readers see.
pub fn mask_pii(text: &str) -> String {
    let masked_email = mask_emails(text);
    mask_digit_runs(&masked_email)
}

fn mask_emails(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        if bytes[i] == b'@' {
            let start = text[..i]
                .rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+')))
                .map(|p| p + 1)
                .unwrap_or(0);
            let rest = &text[i + 1..];
            let end_off = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-')))
                .unwrap_or(rest.len());
            let domain = &rest[..end_off];
            if start < i && domain.contains('.') {
                out.truncate(start);
                out.push_str("[email]");
                i += 1 + end_off;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn mask_digit_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        // Boundary spaces belong to the surrounding text, not the
        // number: "call 555-1234 now" keeps its spaces.
        let lead = run.len() - run.trim_start_matches(' ').len();
        let rest = &run[lead..];
        let trail = rest.len() - rest.trim_end_matches(' ').len();
        let core = &rest[..rest.len() - trail];
        let digits: usize = core.chars().filter(|c| c.is_ascii_digit()).count();
        out.push_str(&run[..lead]);
        if digits >= 7 {
            out.push_str("[number]");
        } else {
            out.push_str(core);
        }
        out.push_str(&rest[rest.len() - trail..]);
        run.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() || matches!(c, ' ' | '-' | '.' | '(' | ')' | '+') {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Display text for a report row: the masked body when masking has
/// landed, otherwise [`AWAITING_MASKING`]. Unmasked bodies are
/// never rendered.
pub fn display_text(masked_body: Option<&str>) -> &str {
    match masked_body {
        Some(body) if !body.trim().is_empty() => body,
        _ => AWAITING_MASKING,
    }
}

/// Validate a submit before any write: non-empty body within
/// [`MAX_BODY_CHARS`], known authority slug, at most
/// [`MAX_PHOTOS`] photos.
pub fn validate_submit(
    body: &str,
    authority_slug: &str,
    photo_count: usize,
) -> Result<(), &'static str> {
    if authority_slug.trim().is_empty() {
        return Err("authority required");
    }
    if body.trim().is_empty() {
        return Err("report text required");
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err("report text too long");
    }
    if photo_count > MAX_PHOTOS {
        return Err("at most 3 photos");
    }
    Ok(())
}

/// One photo accepted by [`submit_report`]: raw bytes plus the
/// client-supplied filename (kept for staff reference only; the
/// kind comes from magic bytes).
pub struct IncomingPhoto {
    pub filename: String,
    pub bytes: Vec<u8>,
}

/// Insert a report and its photos. Photos are magic-byte checked
/// (unknown kinds rejected), size-capped, EXIF-stripped, and stored
/// with `masked = 0` — display shows [`AWAITING_MASKING`] until a
/// masking pass fills `masked_body`. Returns the report id.
pub fn submit_report(
    conn: &rusqlite::Connection,
    user_id: &str,
    authority_slug: &str,
    body: &str,
    photos: &[IncomingPhoto],
    now: i64,
) -> Result<i64, &'static str> {
    validate_submit(body, authority_slug, photos.len())?;
    let slug = authority_slug.trim().to_lowercase();
    let mut kinds = Vec::with_capacity(photos.len());
    for photo in photos {
        if photo.bytes.len() > MAX_PHOTO_BYTES {
            return Err("photo too large");
        }
        let Some(kind) = classify_photo(&photo.bytes) else {
            return Err("photo type not allowed");
        };
        kinds.push(kind);
    }
    conn.execute(
        "INSERT INTO report(user_id, authority_slug, body, created_at, status)
         VALUES (?1, ?2, ?3, ?4, 'open')",
        rusqlite::params![user_id, slug, body.trim(), now],
    )
    .map_err(|_| "store unavailable")?;
    let report_id = conn.last_insert_rowid();
    for (photo, kind) in photos.iter().zip(kinds) {
        let (clean, stripped) = strip_exif(&photo.bytes);
        let digest = sha256_hex(&clean);
        let kind_str = match kind {
            PhotoKind::Jpeg => "jpeg",
            PhotoKind::Png => "png",
            PhotoKind::Webp => "webp",
            PhotoKind::Gif => "gif",
        };
        // Staff reference only: bounded client-supplied name, never
        // a path and never rendered to readers.
        let name: String = photo.filename.chars().take(128).collect();
        conn.execute(
            "INSERT INTO report_photo(report_id, kind, filename, bytes, sha256_hex, exif_stripped, masked)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            rusqlite::params![report_id, kind_str, name, clean, digest, i32::from(stripped)],
        )
        .map_err(|_| "store unavailable")?;
    }
    Ok(report_id)
}

fn new_upload_id() -> String {
    let bytes = rand::random::<[u8; 16]>();
    let mut id = String::with_capacity(32);
    for b in bytes {
        id.push_str(&format!("{b:02x}"));
    }
    id
}

/// Stage one photo for a later submit: size-capped, magic-byte
/// checked and EXIF-stripped now, so a bad file fails at pick time.
/// Prunes the account's expired uploads first; refuses past
/// [`MAX_STAGED_PER_ACCOUNT`]. Returns the upload id.
pub fn stage_upload(
    conn: &rusqlite::Connection,
    user_id: &str,
    filename: &str,
    bytes: &[u8],
    now: i64,
) -> Result<String, &'static str> {
    if bytes.is_empty() {
        return Err("empty file");
    }
    if bytes.len() > MAX_PHOTO_BYTES {
        return Err("photo too large");
    }
    if classify_photo(bytes).is_none() {
        return Err("photo type not allowed");
    }
    conn.execute(
        "DELETE FROM report_upload WHERE created_at < ?1",
        [now - UPLOAD_TTL_SECS],
    )
    .map_err(|_| "store unavailable")?;
    let held: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM report_upload WHERE user_id = ?1",
            [user_id],
            |row| row.get(0),
        )
        .map_err(|_| "store unavailable")?;
    if held >= MAX_STAGED_PER_ACCOUNT {
        return Err("too many photos waiting; send or remove some first");
    }
    let (clean, _) = strip_exif(bytes);
    let id = new_upload_id();
    let name: String = filename.chars().take(128).collect();
    conn.execute(
        "INSERT INTO report_upload(id, user_id, filename, bytes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, user_id, name, clean, now],
    )
    .map_err(|_| "store unavailable")?;
    Ok(id)
}

/// Drop one of the account's staged uploads (the visitor removed
/// it). `true` when a row went away.
pub fn drop_upload(conn: &rusqlite::Connection, user_id: &str, id: &str) -> bool {
    conn.execute(
        "DELETE FROM report_upload WHERE id = ?1 AND user_id = ?2",
        rusqlite::params![id, user_id],
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// Load the account's staged uploads by id, in the order given.
/// An id that is unknown, expired or another account's fails the
/// whole set (the visitor re-picks that photo).
pub fn staged_photos(
    conn: &rusqlite::Connection,
    user_id: &str,
    ids: &[String],
    now: i64,
) -> Result<Vec<IncomingPhoto>, &'static str> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row: Option<(String, Vec<u8>)> = conn
            .query_row(
                "SELECT filename, bytes FROM report_upload
                 WHERE id = ?1 AND user_id = ?2 AND created_at >= ?3",
                rusqlite::params![id, user_id, now - UPLOAD_TTL_SECS],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let Some((filename, bytes)) = row else {
            return Err("a photo upload expired; pick it again");
        };
        out.push(IncomingPhoto { filename, bytes });
    }
    Ok(out)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Public view of one report: `display` is the masked body or
/// [`AWAITING_MASKING`]; `photo_count` counts stored photos, of
/// which only masked ones are viewable (`viewable_photos`).
pub struct ReportView {
    pub id: i64,
    pub authority_slug: String,
    pub display: String,
    pub photo_count: i64,
    pub viewable_photos: i64,
    pub created_at: i64,
}

/// List reports for an authority, masked-first. Unmasked bodies
/// never leave this function: rows without `masked_body` render as
/// [`AWAITING_MASKING`].
pub fn list_reports(
    conn: &rusqlite::Connection,
    authority_slug: &str,
    limit: i64,
) -> Vec<ReportView> {
    let slug = authority_slug.trim().to_lowercase();
    let limit = limit.clamp(1, 50);
    let mut stmt = match conn.prepare(
        "SELECT id, authority_slug, masked_body, created_at,
           (SELECT COUNT(*) FROM report_photo WHERE report_id = report.id) AS photos,
           (SELECT COUNT(*) FROM report_photo WHERE report_id = report.id AND masked = 1) AS viewable
         FROM report WHERE authority_slug = ?1
         ORDER BY id DESC LIMIT ?2",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map(rusqlite::params![slug, limit], |row| {
        let masked: Option<String> = row.get(2)?;
        Ok(ReportView {
            id: row.get(0)?,
            authority_slug: row.get(1)?,
            display: display_text(masked.as_deref()).to_string(),
            photo_count: row.get(4)?,
            viewable_photos: row.get(5)?,
            created_at: row.get(3)?,
        })
    })
    .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
    .unwrap_or_default()
}

/// Staff promotion: turn a masked report into a cited
/// "visitor report" fact proposal. Fails when the report is
/// unknown or its masking has not landed yet (there is nothing
/// safe to cite). Returns the promotion id.
pub fn promote_to_visitor_fact(
    conn: &rusqlite::Connection,
    report_id: i64,
    statement_override: Option<&str>,
    staff: &str,
    now: i64,
) -> Result<i64, &'static str> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT authority_slug, masked_body FROM report WHERE id = ?1",
            [report_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let Some((slug, masked)) = row else {
        return Err("unknown report");
    };
    let cited = match statement_override {
        Some(text) if !text.trim().is_empty() => mask_pii(text.trim()),
        _ => match masked {
            Some(body) if !body.trim().is_empty() => body,
            _ => return Err("report awaiting masking"),
        },
    };
    conn.execute(
        "INSERT INTO report_promotion(report_id, authority_slug, statement, source, created_by, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            report_id,
            slug,
            cited,
            VISITOR_REPORT_SOURCE,
            staff,
            now
        ],
    )
    .map_err(|_| "store unavailable")?;
    Ok(conn.last_insert_rowid())
}

fn rate_limited() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({"error": "rate limited"})),
    )
        .into_response()
}

fn signed_in(conn: &rusqlite::Connection, headers: &HeaderMap, now: i64) -> Option<String> {
    passkey::session_of(conn, headers, now).map(|session| session.user_id)
}

#[derive(serde::Deserialize)]
pub struct SubmitBody {
    pub authority: String,
    pub body: String,
    /// Photos as base64 data (the Svelte form posts JSON; a
    /// multipart upgrade is builder-owned if ever needed).
    #[serde(default)]
    pub photos: Vec<IncomingPhotoBody>,
    /// Photos already staged by [`upload_photo`], by id.
    #[serde(default)]
    pub upload_ids: Vec<String>,
}

#[derive(serde::Deserialize)]
pub struct IncomingPhotoBody {
    #[serde(default)]
    pub filename: String,
    /// Base64-encoded file bytes.
    #[serde(default)]
    pub data_base64: String,
}

/// `POST /api/v1/reports`: signed-in report submit. Rate-limited
/// per IP ([`SUBMIT_PER_MINUTE`]/min) and per account
/// ([`SUBMIT_PER_HOUR_PER_ACCOUNT`]/hour). Anonymous callers get
/// 401 (the form route itself redirects to login).
pub async fn submit(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<SubmitBody>,
) -> Response {
    use base64::Engine as _;
    let now = unix_now();
    let ip = passkey::client_ip(addr);
    if !SUBMIT_LIMITS.check(&format!("rep:{ip}"), now, SUBMIT_PER_MINUTE, 60) {
        return rate_limited();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let Some(user_id) = signed_in(&conn, &headers, now) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "login required"})),
        )
            .into_response();
    };
    if !SUBMIT_LIMITS.check(
        &format!("rep:user:{user_id}"),
        now,
        SUBMIT_PER_HOUR_PER_ACCOUNT,
        3600,
    ) {
        return rate_limited();
    }
    let mut photos = Vec::with_capacity(req.photos.len().min(MAX_PHOTOS + 1));
    for photo in &req.photos {
        let bytes = match base64::engine::general_purpose::STANDARD.decode(&photo.data_base64) {
            Ok(bytes) => bytes,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "photo is not valid base64"})),
                )
                    .into_response();
            }
        };
        photos.push(IncomingPhoto {
            filename: photo.filename.clone(),
            bytes,
        });
    }
    if req.upload_ids.len() > MAX_PHOTOS {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "at most 3 photos"})),
        )
            .into_response();
    }
    match staged_photos(&conn, &user_id, &req.upload_ids, now) {
        Ok(staged) => photos.extend(staged),
        Err(message) => {
            return (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response();
        }
    }
    match submit_report(&conn, &user_id, &req.authority, &req.body, &photos, now) {
        Ok(id) => {
            for upload in &req.upload_ids {
                drop_upload(&conn, &user_id, upload);
            }
            (StatusCode::CREATED, Json(json!({"id": id}))).into_response()
        }
        Err(message) => (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response(),
    }
}

/// Body cap for [`upload_photo`]: one photo plus slack.
pub const UPLOAD_BODY_BYTES: usize = MAX_PHOTO_BYTES + 64 * 1024;

#[derive(serde::Deserialize)]
pub struct UploadQuery {
    #[serde(default)]
    pub filename: String,
}

/// `POST /api/v1/reports/uploads?filename=…`: stage one photo the
/// moment it is picked. The body is the raw file with an `image/*`
/// or `application/octet-stream` type (never a type a cross-site
/// form can send unpreflighted), from an allowed Origin, signed in.
pub async fn upload_photo(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<UploadQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let now = unix_now();
    let ip = passkey::client_ip(addr);
    if !SUBMIT_LIMITS.check(&format!("rep-up:{ip}"), now, UPLOAD_PER_MINUTE, 60) {
        return rate_limited();
    }
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !(content_type.starts_with("image/") || content_type == "application/octet-stream")
        || !passkey::origin_ok(&headers, state.admin_mtls_off)
    {
        return passkey::csrf_fail();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let Some(user_id) = signed_in(&conn, &headers, now) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "login required"})),
        )
            .into_response();
    };
    match stage_upload(&conn, &user_id, &q.filename, &body, now) {
        Ok(id) => (StatusCode::CREATED, Json(json!({"id": id}))).into_response(),
        Err(message) => (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response(),
    }
}

/// `DELETE /api/v1/reports/uploads/{id}`: the visitor removed a
/// staged photo. 204 whether or not it was still there.
pub async fn delete_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let now = unix_now();
    if !passkey::origin_ok(&headers, state.admin_mtls_off) {
        return passkey::csrf_fail();
    }
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let Some(user_id) = signed_in(&conn, &headers, now) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "login required"})),
        )
            .into_response();
    };
    drop_upload(&conn, &user_id, &id);
    StatusCode::NO_CONTENT.into_response()
}

/// `GET /api/v1/authorities/{slug}/reports`: masked report list.
/// Unmasked reports render as [`AWAITING_MASKING`]; no hosted
/// calls, no PII.
pub async fn list_for_authority(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Response {
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    let reports = list_reports(&conn, &slug, 20);
    let items: Vec<serde_json::Value> = reports
        .iter()
        .map(|report| {
            json!({
                "id": report.id,
                "authority": report.authority_slug,
                "text": report.display,
                "photos": report.photo_count,
                "viewable_photos": report.viewable_photos,
                "created_at": report.created_at,
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({"reports": items}))).into_response()
}

#[derive(serde::Deserialize)]
pub struct PromoteBody {
    /// Optional staff-edited statement; masked again server-side.
    #[serde(default)]
    pub statement: Option<String>,
}

/// `POST /api/v1/reports/{id}/promote`: staff turns a masked
/// report into a cited "visitor report" fact proposal. Admin
/// branch only (builder mounts it there — `AdminAuth` resolves
/// only on the admin listener); 409 while the report still awaits
/// masking.
pub async fn promote(
    State(state): State<AppState>,
    admin: AdminAuth,
    Path(report_id): Path<i64>,
    Json(req): Json<PromoteBody>,
) -> Response {
    let now = unix_now();
    let Some(conn) = passkey::open_conn(&state) else {
        return store_unavailable();
    };
    match promote_to_visitor_fact(
        &conn,
        report_id,
        req.statement.as_deref(),
        &admin.user.user_id,
        now,
    ) {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({"promotion_id": id, "source": VISITOR_REPORT_SOURCE})),
        )
            .into_response(),
        Err("report awaiting masking") => (
            StatusCode::CONFLICT,
            Json(json!({"error": "report awaiting masking"})),
        )
            .into_response(),
        Err("unknown report") => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "unknown report"})),
        )
            .into_response(),
        Err(message) => (StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memdb() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("memdb");
        migrate_report_tables(&conn).expect("migrate");
        conn
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = memdb();
        migrate_report_tables(&conn).expect("second migrate");
        migrate_report_tables(&conn).expect("third migrate");
    }

    #[test]
    fn magic_bytes_accept_jpeg_png_webp_gif() {
        assert_eq!(
            classify_photo(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
            Some(PhotoKind::Jpeg)
        );
        assert_eq!(
            classify_photo(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(PhotoKind::Png)
        );
        let webp = b"RIFF\x00\x00\x00\x00WEBPVP8 ".to_vec();
        assert_eq!(classify_photo(&webp), Some(PhotoKind::Webp));
        assert_eq!(classify_photo(b"GIF89a\x01\x00"), Some(PhotoKind::Gif));
        assert_eq!(classify_photo(b"GIF87a\x01\x00"), Some(PhotoKind::Gif));
        assert_eq!(classify_photo(b"%PDF-1.4 fake"), None);
        assert_eq!(classify_photo(b""), None);
    }

    #[test]
    fn exif_app_segment_is_stripped() {
        // Minimal JPEG: SOI + APP1(EXIF) + DQT + SOS.
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xE1, 0x00, 0x08, b'E', b'x', b'i', b'f', 0x00, 0x00]);
        jpeg.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x04, 0x01, 0x02]);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x11, 0x22]);
        let (clean, stripped) = strip_exif(&jpeg);
        assert!(stripped);
        assert!(!clean.windows(4).any(|w| w == b"Exif"));
        assert_eq!(classify_photo(&clean), Some(PhotoKind::Jpeg));
        // Non-JPEG passes through untouched.
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let (same, stripped) = strip_exif(&png);
        assert!(!stripped);
        assert_eq!(same, png);
    }

    #[test]
    fn submit_caps_photos_and_rejects_non_photos() {
        let conn = memdb();
        let png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        let photo = |n: &str| IncomingPhoto {
            filename: n.to_string(),
            bytes: png.clone(),
        };
        let four = vec![photo("a"), photo("b"), photo("c"), photo("d")];
        assert_eq!(
            submit_report(&conn, "u1", "usa-wa-sos", "text", &four, 1),
            Err("at most 3 photos")
        );
        let bad = vec![IncomingPhoto {
            filename: "x.pdf".to_string(),
            bytes: b"%PDF-1.4".to_vec(),
        }];
        assert_eq!(
            submit_report(&conn, "u1", "usa-wa-sos", "text", &bad, 1),
            Err("photo type not allowed")
        );
        assert!(submit_report(&conn, "u1", "usa-wa-sos", "  ", &[], 1).is_err());
    }

    #[test]
    fn staged_uploads_belong_to_their_account_and_expire() {
        let conn = memdb();
        let png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        assert_eq!(
            stage_upload(&conn, "u1", "x.pdf", b"%PDF-1.4", 1),
            Err("photo type not allowed")
        );
        let id = stage_upload(&conn, "u1", "a.png", &png, 100).expect("stage");
        assert!(staged_photos(&conn, "u2", std::slice::from_ref(&id), 100).is_err());
        let got = staged_photos(&conn, "u1", std::slice::from_ref(&id), 100).expect("own upload");
        assert_eq!(got[0].filename, "a.png");
        assert!(
            staged_photos(
                &conn,
                "u1",
                std::slice::from_ref(&id),
                100 + UPLOAD_TTL_SECS + 1
            )
            .is_err()
        );
        assert!(!drop_upload(&conn, "u2", &id));
        assert!(drop_upload(&conn, "u1", &id));
        for _ in 0..MAX_STAGED_PER_ACCOUNT {
            stage_upload(&conn, "u3", "p.png", &png, 200).expect("under cap");
        }
        assert!(stage_upload(&conn, "u3", "p.png", &png, 200).is_err());
        // Expired rows are pruned on the next stage, freeing the cap.
        assert!(stage_upload(&conn, "u3", "p.png", &png, 200 + UPLOAD_TTL_SECS + 1).is_ok());
    }

    #[test]
    fn unmasked_reports_show_awaiting_masking() {
        let conn = memdb();
        let id = submit_report(&conn, "u1", "usa-wa-sos", "myraw text", &[], 7).expect("submit");
        let rows = list_reports(&conn, "usa-wa-sos", 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].display, AWAITING_MASKING);
        // Masking lands: the masked copy is what renders.
        conn.execute(
            "UPDATE report SET masked_body = ?1, masked_at = ?2 WHERE id = ?3",
            rusqlite::params!["masked copy", 8, id],
        )
        .expect("mask");
        let rows = list_reports(&conn, "usa-wa-sos", 10);
        assert_eq!(rows[0].display, "masked copy");
    }

    #[test]
    fn promote_needs_masking_and_cites_visitor_report() {
        let conn = memdb();
        let id = submit_report(&conn, "u1", "usa-wa-sos", "fee is 20", &[], 7).expect("submit");
        assert_eq!(
            promote_to_visitor_fact(&conn, id, None, "staff", 9),
            Err("report awaiting masking")
        );
        assert_eq!(
            promote_to_visitor_fact(&conn, 9999, None, "staff", 9),
            Err("unknown report")
        );
        conn.execute(
            "UPDATE report SET masked_body = ?1, masked_at = ?2 WHERE id = ?3",
            rusqlite::params!["fee is [number]", 8, id],
        )
        .expect("mask");
        let pid = promote_to_visitor_fact(&conn, id, None, "staff", 9).expect("promote");
        let (source, statement): (String, String) = conn
            .query_row(
                "SELECT source, statement FROM report_promotion WHERE id = ?1",
                [pid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read promotion");
        assert_eq!(source, VISITOR_REPORT_SOURCE);
        assert_eq!(statement, "fee is [number]");
        // Staff override is masked again server-side.
        let pid2 = promote_to_visitor_fact(
            &conn,
            id,
            Some("mail jane@example.test about it"),
            "staff",
            10,
        )
        .expect("promote override");
        let statement: String = conn
            .query_row(
                "SELECT statement FROM report_promotion WHERE id = ?1",
                [pid2],
                |row| row.get(0),
            )
            .expect("read override");
        assert_eq!(statement, "mail [email] about it");
    }

    #[test]
    fn pii_masking_covers_email_and_numbers() {
        assert_eq!(
            mask_pii("write jane.doe@example.test ok"),
            "write [email] ok"
        );
        assert_eq!(mask_pii("call +1 (555) 123-4567 now"), "call [number] now");
        assert_eq!(mask_pii("fee is 20 dollars"), "fee is 20 dollars");
    }
}
