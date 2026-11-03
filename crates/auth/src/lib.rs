//! Sessions, roles, CSRF/Origin guards, WS topic authz, login-method rules.
//!
//! Pure policy plus small SQLite helpers for the axum API, per
//! `docs/specs/spa-separation.md` §1.4 and `docs/auth-playbook.md`.
//! The web crate owns HTTP; this crate never sees a request.

pub mod apple;
pub mod csrf;
pub mod login;
pub mod magic;
pub mod meta;
pub mod oauth;
pub mod passkey;
pub mod risc;
pub mod ws;

use rusqlite::{Connection, OptionalExtension};

/// Public session cookie (separate origin ⇒ separate cookie).
pub const PUB_COOKIE: &str = "__Host-hb_pub";
/// Admin session cookie, served only on the admin origin.
pub const ADM_COOKIE: &str = "__Host-hb_adm";
/// Double-submit CSRF token: readable cookie echoed in [`CSRF_HEADER`].
pub const CSRF_COOKIE: &str = "hb_csrf";
/// Header carrying the CSRF token echo on mutations.
pub const CSRF_HEADER: &str = "x-csrf";

/// Production admin host: the default until [`init_hosts`] runs.
pub const ADMIN_HOST: &str = "admin.andrii.ai";
/// Production public host: the default until [`init_hosts`] runs.
pub const PUBLIC_HOST: &str = "andrii.ai";

static HOSTS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();

/// Fix the served host names once at boot from `net.public_host`: the
/// admin host is `admin.<public>`. The first call wins; a blank or
/// malformed name keeps the production defaults. Returns whether this
/// call set them.
pub fn init_hosts(public_host: &str) -> bool {
    let host = public_host
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let valid = !host.is_empty()
        && host.len() <= 200
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !valid {
        return false;
    }
    HOSTS.set((host.clone(), format!("admin.{host}"))).is_ok()
}

/// The public host this server answers as (default [`PUBLIC_HOST`]).
pub fn public_host() -> &'static str {
    HOSTS
        .get()
        .map_or(PUBLIC_HOST, |(public, _)| public.as_str())
}

/// The admin host this server answers as (default [`ADMIN_HOST`]).
pub fn admin_host() -> &'static str {
    HOSTS.get().map_or(ADMIN_HOST, |(_, admin)| admin.as_str())
}

/// Customer idle timeout, seconds (24h; SECURITY.md wins).
pub const CUSTOMER_IDLE_SECS: i64 = 86_400;
/// Customer absolute timeout, seconds (12h; SECURITY.md wins).
pub const CUSTOMER_ABSOLUTE_SECS: i64 = 43_200;
/// Staff/admin idle timeout, seconds (30 min).
pub const ADMIN_IDLE_SECS: i64 = 1_800;
/// Staff/admin absolute timeout, seconds (8h).
pub const ADMIN_ABSOLUTE_SECS: i64 = 28_800;
/// Magic-link fallback window, seconds (7d; spec §12.6, OPEN-2).
pub const FALLBACK_WINDOW_SECS: i64 = 604_800;
/// Customer idle timeout while the fallback window is active,
/// seconds (1h; the 12h absolute still binds above it).
pub const FALLBACK_IDLE_SECS: i64 = 3_600;

/// Caller role, ordered so `>=` is the privilege check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Anonymous,
    Customer,
    Staff,
    Admin,
}

impl Role {
    /// Wire form stored in `session.role`.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Anonymous => "anonymous",
            Role::Customer => "customer",
            Role::Staff => "staff",
            Role::Admin => "admin",
        }
    }

    /// Parse the wire form; unknown input is `None`, never a guess.
    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "anonymous" => Some(Role::Anonymous),
            "customer" => Some(Role::Customer),
            "staff" => Some(Role::Staff),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }

    /// True when this role satisfies a `min` requirement.
    pub fn meets(self, min: Role) -> bool {
        self >= min
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One login session (row in `session`, never in git).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub role: Role,
    pub customer_ref: Option<String>,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub expires_at: i64,
    pub ip_hash: String,
    pub ua_hash: String,
}

impl Session {
    /// Absolute expiry reached (`expires_at` is exclusive).
    pub fn expired(&self, now: i64) -> bool {
        now >= self.expires_at
    }

    /// Idle expiry reached (role-dependent window).
    pub fn idle_expired(&self, now: i64) -> bool {
        let idle = match self.role {
            Role::Staff | Role::Admin => ADMIN_IDLE_SECS,
            Role::Customer | Role::Anonymous => CUSTOMER_IDLE_SECS,
        };
        now.saturating_sub(self.last_seen_at) >= idle
    }

    /// Usable right now: neither expiry hit.
    pub fn valid(&self, now: i64) -> bool {
        !self.expired(now) && !self.idle_expired(now)
    }

    /// Usable right now honoring an active magic-link fallback
    /// window (spec §12.6): customer idle tightens to 1h while
    /// `fallback_until` is in the future; a stale past value
    /// counts as inactive. Other roles are unchanged.
    pub fn valid_with_fallback(&self, now: i64, fallback_until: Option<i64>) -> bool {
        if self.expired(now) {
            return false;
        }
        let idle = match self.role {
            Role::Staff | Role::Admin => ADMIN_IDLE_SECS,
            Role::Customer | Role::Anonymous if fallback_until.is_some_and(|u| u > now) => {
                FALLBACK_IDLE_SECS
            }
            Role::Customer | Role::Anonymous => CUSTOMER_IDLE_SECS,
        };
        now.saturating_sub(self.last_seen_at) < idle
    }
}

/// Session usable right now, honoring the account fallback window
/// (customer role only: one `account` lookup; other roles skip
/// it). Replaces bare `session.valid` at every call site.
pub fn session_usable(conn: &Connection, session: &Session, now: i64) -> bool {
    if !matches!(session.role, Role::Customer) {
        return session.valid(now);
    }
    let fallback = passkey::get_account(conn, &session.user_id)
        .ok()
        .flatten()
        .and_then(|a| a.fallback_until);
    session.valid_with_fallback(now, fallback)
}

/// Authenticated caller resolved from a session cookie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub role: Role,
    pub customer_ref: Option<String>,
    pub session_id: String,
    pub cert_fingerprint: Option<String>,
}

/// Absolute session lifetime for a role, in seconds.
pub fn absolute_timeout(role: Role) -> i64 {
    match role {
        Role::Staff | Role::Admin => ADMIN_ABSOLUTE_SECS,
        Role::Customer | Role::Anonymous => CUSTOMER_ABSOLUTE_SECS,
    }
}

/// New-session fields; `ip`/`user_agent` are hashed, never stored raw.
pub struct NewSession<'a> {
    pub user_id: &'a str,
    pub role: Role,
    pub customer_ref: Option<&'a str>,
    pub now: i64,
    pub ip: &'a str,
    pub user_agent: &'a str,
}

/// Create the `session`, `identity_link`, `admin_cert`,
/// `account`, `webauthn_credential`, and `webauthn_challenge`
/// tables (spec `customer-auth.md` §1).
///
/// Separate from `handbook-store` migrations (single-writer rule):
/// the web binary runs both at startup.
pub fn migrate_auth_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS account(
           user_id TEXT PRIMARY KEY,
           role TEXT NOT NULL DEFAULT 'customer',
           customer_ref TEXT UNIQUE,
           created_at INTEGER NOT NULL,
           locked_at INTEGER,
           step_up_at INTEGER,
           fallback_until INTEGER
         ) STRICT;
         CREATE TABLE IF NOT EXISTS webauthn_credential(
           credential_id TEXT PRIMARY KEY,
           user_id TEXT NOT NULL REFERENCES account(user_id),
           passkey_json TEXT NOT NULL,
           label TEXT NOT NULL DEFAULT '',
           created_at INTEGER NOT NULL,
           last_used_at INTEGER,
           revoked_at INTEGER
         ) STRICT;
         CREATE INDEX IF NOT EXISTS idx_cred_user ON webauthn_credential(user_id);
         CREATE TABLE IF NOT EXISTS webauthn_challenge(
           id TEXT PRIMARY KEY,
           user_id TEXT,
           kind TEXT NOT NULL,
           state_json TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           expires_at INTEGER NOT NULL,
           consumed_at INTEGER
         ) STRICT;
         CREATE INDEX IF NOT EXISTS idx_challenge_exp ON webauthn_challenge(expires_at);
         CREATE TABLE IF NOT EXISTS session(
           id TEXT PRIMARY KEY,
           user_id TEXT NOT NULL,
           role TEXT NOT NULL,
           customer_ref TEXT,
           created_at INTEGER NOT NULL,
           last_seen_at INTEGER NOT NULL,
           expires_at INTEGER NOT NULL,
           ip_hash TEXT NOT NULL,
           ua_hash TEXT NOT NULL
         ) STRICT;
         CREATE INDEX IF NOT EXISTS idx_session_user ON session(user_id);
         CREATE TABLE IF NOT EXISTS identity_link(
           user_id TEXT NOT NULL,
           method TEXT NOT NULL,
           provider_sub TEXT NOT NULL,
           email_hash TEXT,
           verified_email INTEGER NOT NULL DEFAULT 0,
           is_relay INTEGER NOT NULL DEFAULT 0,
           disabled_at INTEGER,
           PRIMARY KEY (user_id, method, provider_sub)
         ) STRICT;
         CREATE TABLE IF NOT EXISTS admin_cert(
           fingerprint TEXT PRIMARY KEY,
           cn TEXT NOT NULL,
           issued_at INTEGER NOT NULL,
           expires_at INTEGER NOT NULL,
           revoked_at INTEGER,
           cert_path TEXT NOT NULL DEFAULT ''
         ) STRICT;
         CREATE TABLE IF NOT EXISTS staff_invite(
           token_hash TEXT PRIMARY KEY,
           role TEXT NOT NULL,
           cn TEXT NOT NULL,
           created_by TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           expires_at INTEGER NOT NULL,
           consumed_at INTEGER,
           consumed_by TEXT
         ) STRICT;
         CREATE TABLE IF NOT EXISTS magic_token(
           token_hash TEXT PRIMARY KEY,
           email_hash TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           expires_at INTEGER NOT NULL,
           consumed_at INTEGER
         ) STRICT;
         CREATE INDEX IF NOT EXISTS idx_magic_email ON magic_token(email_hash);
         CREATE INDEX IF NOT EXISTS idx_magic_exp ON magic_token(expires_at);
         -- Meta data-deletion requests (customer-auth.md §11.9):
         -- confirmation code → human-readable status. No user id
         -- or Meta sub is kept: post-purge there is nothing to link.
         CREATE TABLE IF NOT EXISTS meta_deletion(
           code TEXT PRIMARY KEY,
           status TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           completed_at INTEGER
         ) STRICT;",
    )?;
    Ok(())
}

/// Fresh 256-bit session id, hex.
pub fn mint_session_id() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex_of(&bytes)
}

/// SHA-256 hex of a value hashed for storage (IPs, user agents, emails).
pub fn hash_str(value: &str) -> String {
    use sha2::Digest as _;
    hex_of(&sha2::Sha256::digest(value.as_bytes()))
}

pub(crate) fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Insert a session; absolute expiry derives from the role.
pub fn create_session(conn: &Connection, id: &str, new: &NewSession<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO session(id, user_id, role, customer_ref, created_at,
                              last_seen_at, expires_at, ip_hash, ua_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            new.user_id,
            new.role.as_str(),
            new.customer_ref,
            new.now,
            new.now.saturating_add(absolute_timeout(new.role)),
            hash_str(new.ip),
            hash_str(new.user_agent),
        ],
    )?;
    Ok(())
}

/// One `session` row as selected (column order of [`get_session`]).
type SessionRow = (
    String,
    String,
    String,
    Option<String>,
    i64,
    i64,
    i64,
    String,
    String,
);

/// Look up a session by id; callers must check [`Session::valid`].
pub fn get_session(conn: &Connection, id: &str) -> rusqlite::Result<Option<Session>> {
    let row: Option<SessionRow> = conn
        .query_row(
            "SELECT id, user_id, role, customer_ref, created_at,
                    last_seen_at, expires_at, ip_hash, ua_hash
             FROM session WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(
        |(
            id,
            user_id,
            role,
            customer_ref,
            created_at,
            last_seen_at,
            expires_at,
            ip_hash,
            ua_hash,
        )| {
            Role::parse(&role).map(|role| Session {
                id,
                user_id,
                role,
                customer_ref,
                created_at,
                last_seen_at,
                expires_at,
                ip_hash,
                ua_hash,
            })
        },
    ))
}

/// Refresh idle tracking after an authenticated request.
pub fn touch_session(conn: &Connection, id: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE session SET last_seen_at = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    )?;
    Ok(())
}

/// Delete one session (logout).
pub fn delete_session(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM session WHERE id = ?1", [id])?;
    Ok(())
}

/// Delete sessions past absolute expiry. Returns rows deleted.
pub fn delete_expired_sessions(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM session WHERE expires_at <= ?1", [now])
}

/// Delete every session of one user (revocation signals, lockout).
pub fn revoke_user_sessions(conn: &Connection, user_id: &str) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM session WHERE user_id = ?1", [user_id])
}

/// Record a newly issued admin client cert (fingerprint = SHA-256 hex).
/// Column shape matches `xtask admin-cert-*` (the CA/file owner); either
/// side may create the table first, so the DDL must stay identical.
pub fn issue_admin_cert(
    conn: &Connection,
    fingerprint: &str,
    cn: &str,
    issued_at: i64,
    expires_at: i64,
    cert_path: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO admin_cert(fingerprint, cn, issued_at, expires_at, revoked_at, cert_path)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
        rusqlite::params![fingerprint, cn, issued_at, expires_at, cert_path],
    )?;
    Ok(())
}

/// Revoke an admin client cert; revoked certs fail closed.
pub fn revoke_admin_cert(conn: &Connection, fingerprint: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE admin_cert SET revoked_at = ?1 WHERE fingerprint = ?2",
        rusqlite::params![now, fingerprint],
    )?;
    Ok(())
}

/// True when the fingerprint is known, not revoked, and not
/// expired (`expires_at` is exclusive: `now >= expires_at` fails
/// closed, the same reading as [`Session::valid`]).
pub fn admin_cert_valid(conn: &Connection, fingerprint: &str, now: i64) -> rusqlite::Result<bool> {
    let row: Option<(Option<i64>, i64)> = conn
        .query_row(
            "SELECT revoked_at, expires_at FROM admin_cert WHERE fingerprint = ?1",
            [fingerprint],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(matches!(row, Some((None, expires_at)) if now < expires_at))
}

/// `Set-Cookie` value for a session id. `__Host-` names require
/// `Secure` + `Path=/` + no `Domain`; over plain-HTTP dev the
/// browser will not store it, which is correct fail-closed behavior.
pub fn session_cookie(name: &str, id: &str, secure: bool) -> String {
    let mut cookie = format!("{name}={id}; HttpOnly; SameSite=Lax; Path=/");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Double-submit CSRF cookie: readable by page JS (NOT HttpOnly),
/// echoed back in [`CSRF_HEADER`].
pub fn csrf_cookie(token: &str, secure: bool) -> String {
    let mut cookie = format!("{CSRF_COOKIE}={token}; SameSite=Lax; Path=/");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Expire a cookie immediately (logout).
pub fn clear_cookie(name: &str, secure: bool) -> String {
    let mut cookie = format!("{name}=; Max-Age=0; HttpOnly; SameSite=Lax; Path=/");
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// Extract one cookie value from a `Cookie` header value.
pub fn cookie_value<'a>(cookies: &'a str, name: &str) -> Option<&'a str> {
    cookies
        .split(';')
        .map(str::trim)
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> Connection {
        let conn = Connection::open_in_memory().expect("open memory db");
        migrate_auth_tables(&conn).expect("migrate auth tables");
        conn
    }

    fn new_session<'a>(now: i64) -> NewSession<'a> {
        NewSession {
            user_id: "u1",
            role: Role::Customer,
            customer_ref: Some("cust-1"),
            now,
            ip: "127.0.0.1",
            user_agent: "test",
        }
    }

    #[test]
    fn roles_order_and_parse() {
        assert!(Role::Admin.meets(Role::Staff));
        assert!(Role::Staff.meets(Role::Staff));
        assert!(!Role::Customer.meets(Role::Staff));
        assert!(!Role::Anonymous.meets(Role::Customer));
        for role in [Role::Anonymous, Role::Customer, Role::Staff, Role::Admin] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("root"), None);
        assert_eq!(Role::parse(""), None);
    }

    #[test]
    fn session_roundtrip_and_expiry() {
        let conn = memory();
        let now = 1_800_000_000;
        create_session(&conn, "sess-1", &new_session(now)).expect("create");
        let s = get_session(&conn, "sess-1").expect("get").expect("present");
        assert_eq!(s.user_id, "u1");
        assert_eq!(s.role, Role::Customer);
        assert_eq!(s.customer_ref.as_deref(), Some("cust-1"));
        assert_eq!(s.expires_at, now + CUSTOMER_ABSOLUTE_SECS);
        assert_ne!(s.ip_hash, "127.0.0.1");
        assert!(s.valid(now));
        assert!(!s.valid(now + CUSTOMER_ABSOLUTE_SECS));
        assert!(!s.valid(now + CUSTOMER_IDLE_SECS));
        assert_eq!(get_session(&conn, "missing").expect("get"), None);
    }

    #[test]
    fn admin_idle_window_is_shorter() {
        let conn = memory();
        let now = 1_800_000_000;
        create_session(
            &conn,
            "sess-adm",
            &NewSession {
                role: Role::Admin,
                customer_ref: None,
                ..new_session(now)
            },
        )
        .expect("create");
        let s = get_session(&conn, "sess-adm")
            .expect("get")
            .expect("present");
        assert!(s.valid(now + ADMIN_IDLE_SECS - 1));
        assert!(!s.valid(now + ADMIN_IDLE_SECS));
    }

    #[test]
    fn fallback_window_tightens_customer_idle() {
        let conn = memory();
        let now = 1_800_000_000;
        conn.execute(
            "INSERT INTO account(user_id, role, customer_ref, created_at, locked_at, step_up_at, fallback_until)
             VALUES ('u1', 'customer', 'cust-1', 1, NULL, NULL, NULL)",
            [],
        )
        .unwrap();
        create_session(&conn, "sess-1", &new_session(now)).expect("create");
        let s = get_session(&conn, "sess-1").expect("get").expect("present");
        // No window: role default (24h idle; absolute binds first).
        assert!(session_usable(&conn, &s, now + FALLBACK_IDLE_SECS));
        // Active window: 1h idle binds.
        magic::set_fallback(&conn, "u1", now + FALLBACK_WINDOW_SECS).unwrap();
        assert!(session_usable(&conn, &s, now + FALLBACK_IDLE_SECS - 1));
        assert!(!session_usable(&conn, &s, now + FALLBACK_IDLE_SECS));
        // Stale window: inactive again (compare > now, not non-NULL).
        magic::set_fallback(&conn, "u1", now - 1).unwrap();
        assert!(session_usable(&conn, &s, now + FALLBACK_IDLE_SECS));
        // Graduation: passkey login clears the window.
        magic::set_fallback(&conn, "u1", now + FALLBACK_WINDOW_SECS).unwrap();
        passkey::clear_fallback(&conn, "u1").unwrap();
        assert!(session_usable(&conn, &s, now + FALLBACK_IDLE_SECS));
    }

    #[test]
    fn session_usable_skips_account_for_staff() {
        let conn = memory();
        let now = 1_800_000_000;
        create_session(
            &conn,
            "sess-adm",
            &NewSession {
                role: Role::Admin,
                customer_ref: None,
                ..new_session(now)
            },
        )
        .expect("create");
        let s = get_session(&conn, "sess-adm")
            .expect("get")
            .expect("present");
        // No account row at all: staff path never looks.
        assert!(session_usable(&conn, &s, now + ADMIN_IDLE_SECS - 1));
        assert!(!session_usable(&conn, &s, now + ADMIN_IDLE_SECS));
    }

    #[test]
    fn touch_extends_idle_but_not_absolute() {
        let conn = memory();
        let now = 1_800_000_000;
        create_session(&conn, "sess-1", &new_session(now)).expect("create");
        touch_session(&conn, "sess-1", now + 1000).expect("touch");
        let s = get_session(&conn, "sess-1").expect("get").expect("present");
        assert_eq!(s.last_seen_at, now + 1000);
        assert_eq!(s.expires_at, now + CUSTOMER_ABSOLUTE_SECS);
        delete_expired_sessions(&conn, now + CUSTOMER_ABSOLUTE_SECS).expect("sweep");
        assert_eq!(get_session(&conn, "sess-1").expect("get"), None);
    }

    #[test]
    fn revoke_kills_all_user_sessions() {
        let conn = memory();
        let now = 1_800_000_000;
        create_session(&conn, "a", &new_session(now)).expect("create a");
        create_session(&conn, "b", &new_session(now)).expect("create b");
        assert_eq!(revoke_user_sessions(&conn, "u1").expect("revoke"), 2);
        assert_eq!(get_session(&conn, "a").expect("get"), None);
        delete_session(&conn, "b").expect("delete is idempotent");
    }

    #[test]
    fn host_names_default_to_production_and_reject_garbage() {
        // The process-wide names are not set in unit tests: defaults hold.
        assert_eq!(public_host(), PUBLIC_HOST);
        assert_eq!(admin_host(), ADMIN_HOST);
        // Garbage never sets them (and leaves the defaults in place).
        for bad in [
            "",
            "  ",
            "exa mple.test",
            "a..b",
            "x/y",
            "https://andrii.ai",
        ] {
            assert!(!init_hosts(bad), "{bad:?}");
        }
        assert_eq!(admin_host(), ADMIN_HOST);
    }

    #[test]
    fn admin_cert_lifecycle() {
        let conn = memory();
        assert!(!admin_cert_valid(&conn, "fp-unknown", 150).expect("unknown is invalid"));
        issue_admin_cert(&conn, "fp-1", "alice", 100, 200, "").expect("issue");
        assert!(admin_cert_valid(&conn, "fp-1", 150).expect("valid"));
        revoke_admin_cert(&conn, "fp-1", 200).expect("revoke");
        assert!(!admin_cert_valid(&conn, "fp-1", 150).expect("revoked fails closed"));
    }

    #[test]
    fn admin_cert_expired_fails_closed() {
        // An unrevoked cert past `expires_at` is invalid — expiry is
        // exclusive, so `now == expires_at` already fails.
        let conn = memory();
        issue_admin_cert(&conn, "fp-old", "alice", 100, 200, "").expect("issue");
        assert!(admin_cert_valid(&conn, "fp-old", 199).expect("live until expiry"));
        assert!(!admin_cert_valid(&conn, "fp-old", 200).expect("expired at the boundary"));
        assert!(!admin_cert_valid(&conn, "fp-old", 201).expect("expired after"));
    }

    #[test]
    fn cookie_helpers() {
        assert_eq!(mint_session_id().len(), 64);
        let set = session_cookie(ADM_COOKIE, "id", true);
        assert!(set.contains("__Host-hb_adm=id"));
        assert!(set.contains("Secure"));
        assert!(!set.contains("Domain="));
        assert_eq!(
            cookie_value("a=1; __Host-hb_adm=tok; b=2", ADM_COOKIE),
            Some("tok")
        );
        assert_eq!(cookie_value("a=1", ADM_COOKIE), None);
        assert_eq!(cookie_value("__Host-hb_adm=", ADM_COOKIE), None);
    }
}
