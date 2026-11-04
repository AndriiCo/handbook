//! Key checks: one no-op call per provider, 10 s each, run against a
//! target's effective config. A check never spends anything (no label,
//! no message, no charge, no ACME order) and its detail never carries a
//! secret: provider answers are reduced to status codes and error codes,
//! and any secret value that still appears is masked.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use super::file::{SetupFile, TargetKind, is_secret};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
            Status::Skip => "skip",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub id: &'static str,
    pub label: &'static str,
    pub status: Status,
    pub detail: String,
}

/// Ids and labels, in display order (the wizard lists these).
pub const CHECKS: &[(&str, &str)] = &[
    ("ssh", "SSH"),
    ("dns", "DNS"),
    ("meta", "Meta model API"),
    ("mail", "Gmail inbox"),
    ("fedex", "FedEx OAuth"),
    ("usps", "USPS OAuth + payment"),
    ("meta_login", "Meta login app"),
    ("apple", "Sign in with Apple"),
    ("google", "Google sign-in"),
    ("stripe", "Stripe"),
    ("telnyx", "Telnyx"),
    ("smtp", "SMTP relay"),
    ("acme_eab", "ACME EAB"),
];

struct Values(BTreeMap<String, Value>);

impl Values {
    fn get(&self, key: &str) -> Option<String> {
        match self.0.get(key)? {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }

    fn all(&self, keys: &[&str]) -> Option<Vec<String>> {
        keys.iter().map(|k| self.get(k)).collect()
    }

    fn secrets(&self) -> Vec<String> {
        self.0
            .iter()
            .filter(|(k, _)| is_secret(k))
            .filter_map(|(_, v)| v.as_str().map(str::to_string))
            .filter(|s| s.len() >= 6)
            .collect()
    }
}

fn agent() -> ureq::Agent {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let _ = rustls::crypto::CryptoProvider::install_default(provider);
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into()
}

/// (status, body) of an answer; transport errors become `Err`.
type Answer = Result<(u16, String), String>;

fn answer(result: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Answer {
    let mut response = result.map_err(|e| format!("unreachable: {e}"))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .unwrap_or_default();
    Ok((status, body))
}

/// The provider's error code, if its answer is JSON (`error`,
/// `error.code`, `error.message`, `errors[0].code`).
fn error_code(body: &str) -> String {
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return String::new();
    };
    let pick = |v: &Value| v.as_str().map(str::to_string);
    let code = pick(&json["error"])
        .or_else(|| {
            pick(&json["error"]["code"])
                .or_else(|| json["error"]["code"].as_i64().map(|c| c.to_string()))
        })
        .or_else(|| pick(&json["error"]["type"]))
        .or_else(|| pick(&json["errors"][0]["code"]))
        .or_else(|| pick(&json["errors"][0]["title"]))
        .unwrap_or_default();
    code.chars().take(80).collect()
}

fn verdict(id: &'static str, status: Status, detail: impl Into<String>) -> CheckResult {
    let label = CHECKS
        .iter()
        .find(|(i, _)| *i == id)
        .map_or(id, |(_, l)| *l);
    CheckResult {
        id,
        label,
        status,
        detail: detail.into(),
    }
}

fn skip(id: &'static str, why: &str) -> CheckResult {
    verdict(id, Status::Skip, why.to_string())
}

fn http_verdict(id: &'static str, answer: Answer, ok_detail: &str) -> CheckResult {
    match answer {
        Ok((code, _)) if (200..300).contains(&code) => verdict(id, Status::Ok, ok_detail),
        Ok((code, body)) => verdict(
            id,
            Status::Fail,
            format!("HTTP {code} {}", error_code(&body)),
        ),
        Err(e) => verdict(id, Status::Fail, e),
    }
}

/// Every check for one target. Unset providers are skipped.
pub fn run_target(file: &SetupFile, name: &str) -> Vec<CheckResult> {
    let Ok(values) = file.target_config(name) else {
        return vec![verdict("ssh", Status::Fail, format!("no target {name}"))];
    };
    let values = Values(values);
    let target = &file.targets[name];
    let production = name == "production";
    let checks: Vec<Box<dyn Fn() -> CheckResult + Sync + Send + '_>> = vec![
        Box::new(|| check_ssh(target)),
        Box::new(|| check_dns(target)),
        Box::new(|| check_meta(&values)),
        Box::new(|| check_mail(&values)),
        Box::new(|| check_fedex(&values)),
        Box::new(|| check_usps(&values)),
        Box::new(|| check_meta_login(&values)),
        Box::new(|| check_apple(&values)),
        Box::new(|| check_google(&values, target.public_host.as_deref())),
        Box::new(|| check_stripe(&values, production)),
        Box::new(|| check_telnyx(&values)),
        Box::new(|| check_smtp(&values)),
        Box::new(|| check_eab(&values)),
    ];
    let secrets = values.secrets();
    std::thread::scope(|scope| {
        let handles: Vec<_> = checks.iter().map(|c| scope.spawn(c)).collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| verdict("ssh", Status::Fail, "check panicked"))
            })
            .map(|mut r| {
                for s in &secrets {
                    r.detail = r.detail.replace(s.as_str(), "[secret]");
                }
                r
            })
            .collect()
    })
}

fn check_ssh(target: &super::file::Target) -> CheckResult {
    let Some(alias) = &target.ssh else {
        return skip("ssh", "no ssh alias (local target)");
    };
    let run = |alias: &str, cmd: &str| {
        std::process::Command::new("ssh")
            .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", alias, cmd])
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !run(alias, "true") {
        return verdict(
            "ssh",
            Status::Fail,
            format!("{alias}: login refused or unreachable"),
        );
    }
    match &target.ssh_admin {
        Some(admin) if !run(admin, "sudo -n true") => verdict(
            "ssh",
            Status::Warn,
            format!("{alias} ok; {admin} has no passwordless sudo (provision prompts)"),
        ),
        Some(admin) => verdict("ssh", Status::Ok, format!("{alias} ok; {admin} can sudo")),
        None => verdict("ssh", Status::Ok, format!("{alias} ok")),
    }
}

fn check_dns(target: &super::file::Target) -> CheckResult {
    use std::net::ToSocketAddrs as _;
    if target.kind != TargetKind::Server {
        return skip("dns", "no public DNS for this target");
    }
    let mut problems = Vec::new();
    for name in &target.dns {
        match (name.as_str(), 443).to_socket_addrs() {
            Ok(addrs) => {
                let ips: Vec<String> = addrs.map(|a| a.ip().to_string()).collect();
                for want in [&target.bind, &target.bind6].into_iter().flatten() {
                    if !ips.contains(want) {
                        problems.push(format!("{name} does not resolve to {want}"));
                    }
                }
            }
            Err(_) => problems.push(format!("{name} does not resolve")),
        }
    }
    if problems.is_empty() {
        verdict(
            "dns",
            Status::Ok,
            format!("{} names resolve", target.dns.len()),
        )
    } else {
        verdict("dns", Status::Fail, problems.join("; "))
    }
}

fn check_meta(v: &Values) -> CheckResult {
    let Some(key) = v.get("meta.api_key") else {
        return skip("meta", "meta.api_key unset");
    };
    let base = v
        .get("meta.base_url")
        .unwrap_or_else(|| handbook_agent::model::META_BASE_URL.to_string());
    match handbook_agent::model::MetaRestClient::new(base, key).check() {
        Ok(models) => verdict(
            "meta",
            Status::Ok,
            format!("{} models visible", models.len()),
        ),
        Err(e) => verdict("meta", Status::Fail, e.to_string()),
    }
}

/// The model ids a target's Meta key sees (`GET /v1/models`), for the
/// map's Test button. The key stays here; errors are masked like checks.
pub fn meta_models(file: &SetupFile, name: &str) -> Result<Vec<String>, String> {
    let values = Values(file.target_config(name)?);
    let key = values
        .get("meta.api_key")
        .ok_or(format!("meta.api_key is not set for {name}"))?;
    let base = values
        .get("meta.base_url")
        .unwrap_or_else(|| handbook_agent::model::META_BASE_URL.to_string());
    handbook_agent::model::MetaRestClient::new(base, key)
        .check()
        .map_err(|e| {
            values
                .secrets()
                .iter()
                .fold(e.to_string(), |d, s| d.replace(s.as_str(), "[secret]"))
        })
}

fn check_mail(v: &Values) -> CheckResult {
    let Some([id, secret, refresh]) = v
        .all(&["mail.client_id", "mail.client_secret", "mail.refresh_token"])
        .and_then(|x| <[String; 3]>::try_from(x).ok())
    else {
        return skip("mail", "mail.client_id/client_secret/refresh_token unset");
    };
    let token_url = v
        .get("mail.token_url")
        .unwrap_or_else(|| mail::client::TOKEN_URL.to_string());
    let base = v
        .get("mail.base_url")
        .unwrap_or_else(|| mail::client::GMAIL_BASE_URL.to_string());
    let agent = agent();
    let token = match answer(agent.post(&token_url).send_form([
        ("grant_type", "refresh_token"),
        ("client_id", id.as_str()),
        ("client_secret", secret.as_str()),
        ("refresh_token", refresh.as_str()),
    ])) {
        Ok((200, body)) => serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|j| j["access_token"].as_str().map(str::to_string)),
        Ok((code, body)) => {
            return verdict(
                "mail",
                Status::Fail,
                format!("token HTTP {code} {}", error_code(&body)),
            );
        }
        Err(e) => return verdict("mail", Status::Fail, e),
    };
    let Some(token) = token else {
        return verdict("mail", Status::Fail, "token answer has no access_token");
    };
    let url = format!("{}/gmail/v1/users/me/profile", base.trim_end_matches('/'));
    http_verdict(
        "mail",
        answer(
            agent
                .get(&url)
                .header("Authorization", &format!("Bearer {token}"))
                .call(),
        ),
        "refresh token works; mailbox profile readable",
    )
}

fn check_fedex(v: &Values) -> CheckResult {
    let Some([id, secret]) = v
        .all(&["labels.fedex_client_id", "labels.fedex_client_secret"])
        .and_then(|x| <[String; 2]>::try_from(x).ok())
    else {
        return skip("fedex", "FedEx keys unset");
    };
    // An unset base URL means production (config guide).
    let base = v
        .get("labels.fedex_base_url")
        .unwrap_or_else(|| "https://apis.fedex.com".into());
    let url = labels::live::oauth_token_url(&base);
    let sandbox = base.contains("sandbox");
    http_verdict(
        "fedex",
        answer(
            agent()
                .post(&url)
                .send_form(labels::live::oauth_token_pairs(&id, &secret)),
        ),
        if sandbox {
            "sandbox token minted"
        } else {
            "production token minted"
        },
    )
}

fn check_usps(v: &Values) -> CheckResult {
    let Some([id, secret]) = v
        .all(&["usps.client_id", "usps.client_secret"])
        .and_then(|x| <[String; 2]>::try_from(x).ok())
    else {
        return skip("usps", "USPS keys unset");
    };
    let base = v
        .get("usps.base_url")
        .unwrap_or_else(|| format!("https://{}", labels::usps::TEM_HOST));
    let base = base.trim_end_matches('/');
    let agent = agent();
    let token = match answer(
        agent
            .post(&format!("{base}/oauth2/v3/token"))
            .send_json(labels::usps::token_body(&id, &secret)),
    ) {
        Ok((200, body)) => match labels::usps::parse_token(body.as_bytes()) {
            Ok(t) => t,
            Err(e) => return verdict("usps", Status::Fail, e),
        },
        Ok((code, body)) => {
            return verdict(
                "usps",
                Status::Fail,
                format!("token HTTP {code} {}", error_code(&body)),
            );
        }
        Err(e) => return verdict("usps", Status::Fail, e),
    };
    let Some([crid, mid, manifest_mid, epa]) = v
        .all(&[
            "usps.crid",
            "usps.mid",
            "usps.manifest_mid",
            "usps.epa_account",
        ])
        .and_then(|x| <[String; 4]>::try_from(x).ok())
    else {
        return verdict(
            "usps",
            Status::Warn,
            "token minted; payment accounts unset (CRID, MIDs, EPA)",
        );
    };
    let body = labels::usps::payment_body(&labels::usps::PaymentAccount {
        crid: &crid,
        mid: &mid,
        manifest_mid: &manifest_mid,
        epa_account: &epa,
    });
    match answer(
        agent
            .post(&format!("{base}/payments/v3/payment-authorization"))
            .header("Authorization", &format!("Bearer {token}"))
            .send_json(body),
    ) {
        Ok((code, body)) if (200..300).contains(&code) => {
            match labels::usps::parse_payment_token(body.as_bytes()) {
                Ok(_) => verdict(
                    "usps",
                    Status::Ok,
                    "token and payment authorization minted (nothing bought)",
                ),
                Err(e) => verdict("usps", Status::Fail, e),
            }
        }
        Ok((code, body)) => verdict(
            "usps",
            Status::Fail,
            format!("payment HTTP {code} {}", error_code(&body)),
        ),
        Err(e) => verdict("usps", Status::Fail, e),
    }
}

fn check_meta_login(v: &Values) -> CheckResult {
    let Some([id, secret]) = v
        .all(&["meta.client_id", "meta.client_secret"])
        .and_then(|x| <[String; 2]>::try_from(x).ok())
    else {
        return skip("meta_login", "meta.client_id/client_secret unset");
    };
    // An app access token proves the id/secret pair; POST keeps the
    // secret out of any URL.
    http_verdict(
        "meta_login",
        answer(
            agent()
                .post("https://graph.facebook.com/oauth/access_token")
                .send_form([
                    ("client_id", id.as_str()),
                    ("client_secret", secret.as_str()),
                    ("grant_type", "client_credentials"),
                ]),
        ),
        "app id and secret accepted",
    )
}

fn check_apple(v: &Values) -> CheckResult {
    let Some([team, client, key_id, pem]) = v
        .all(&[
            "apple.team_id",
            "apple.client_id",
            "apple.key_id",
            "apple.private_key",
        ])
        .and_then(|x| <[String; 4]>::try_from(x).ok())
    else {
        return skip("apple", "Apple keys unset");
    };
    match auth::apple::mint_client_secret(&team, &client, &key_id, &pem, crate::unix_now()) {
        Ok(_) => verdict(
            "apple",
            Status::Ok,
            "ES256 client secret signs (offline; Apple has no no-op call)",
        ),
        Err(e) => verdict(
            "apple",
            Status::Fail,
            format!("{e:?}").chars().take(120).collect::<String>(),
        ),
    }
}

fn check_google(v: &Values, public_host: Option<&str>) -> CheckResult {
    let Some([id, secret]) = v
        .all(&["google.client_id", "google.client_secret"])
        .and_then(|x| <[String; 2]>::try_from(x).ok())
    else {
        return skip("google", "google.client_id/client_secret unset");
    };
    let redirect = format!(
        "https://{}/auth/google/callback",
        public_host.unwrap_or("localhost")
    );
    // A made-up code: Google checks the client first, then the code.
    match answer(
        agent()
            .post("https://oauth2.googleapis.com/token")
            .send_form([
                ("grant_type", "authorization_code"),
                ("code", "setup-check-not-a-real-code"),
                ("client_id", id.as_str()),
                ("client_secret", secret.as_str()),
                ("redirect_uri", redirect.as_str()),
            ]),
    ) {
        Ok((_, body)) => match error_code(&body).as_str() {
            "invalid_grant" => verdict("google", Status::Ok, "client id and secret accepted"),
            "redirect_uri_mismatch" => verdict(
                "google",
                Status::Warn,
                format!("client ok; add {redirect} as a redirect URI"),
            ),
            "invalid_client" | "unauthorized_client" => {
                verdict("google", Status::Fail, "client id or secret refused")
            }
            other => verdict("google", Status::Warn, format!("unexpected answer {other}")),
        },
        Err(e) => verdict("google", Status::Fail, e),
    }
}

fn check_stripe(v: &Values, production: bool) -> CheckResult {
    let Some(key) = v.get("stripe.secret_key") else {
        return skip("stripe", "stripe.secret_key unset");
    };
    let live = key.starts_with("sk_live_") || key.starts_with("rk_live_");
    if live && !production {
        return verdict(
            "stripe",
            Status::Fail,
            "a live key on a test target; use an sk_test_ key here",
        );
    }
    let mode = if live { "live" } else { "test" };
    match answer(
        agent()
            .get("https://api.stripe.com/v1/balance")
            .header("Authorization", &format!("Bearer {key}"))
            .call(),
    ) {
        Ok((200, _)) if production && !live => verdict(
            "stripe",
            Status::Warn,
            "test key on production (no real charges)",
        ),
        Ok((200, _)) => verdict(
            "stripe",
            Status::Ok,
            format!("{mode} key accepted (balance read)"),
        ),
        Ok((code, body)) => verdict(
            "stripe",
            Status::Fail,
            format!("HTTP {code} {}", error_code(&body)),
        ),
        Err(e) => verdict("stripe", Status::Fail, e),
    }
}

fn check_telnyx(v: &Values) -> CheckResult {
    let Some(key) = v.get("telnyx.api_key") else {
        return skip("telnyx", "telnyx.api_key unset");
    };
    // With an API key the Calls page dials for real. Without the
    // webhook key every call event is refused (no recording, no
    // transcript); without the WebRTC connection there is no audio.
    let Some(public) = v.get("telnyx.webhook_public_key") else {
        return verdict(
            "telnyx",
            Status::Fail,
            "telnyx.webhook_public_key unset: every call webhook is refused",
        );
    };
    {
        use base64::Engine as _;
        let parsed = base64::engine::general_purpose::STANDARD
            .decode(public.trim())
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
            .and_then(|b| ed25519_dalek::VerifyingKey::from_bytes(&b).ok());
        if parsed.is_none() {
            return verdict(
                "telnyx",
                Status::Fail,
                "telnyx.webhook_public_key is not an Ed25519 key (base64, 32 bytes)",
            );
        }
    }
    if v.get("telnyx.webrtc_connection_id").is_none() {
        return verdict(
            "telnyx",
            Status::Fail,
            "telnyx.webrtc_connection_id unset: calls have no browser audio",
        );
    }
    let base = v
        .get("telnyx.base_url")
        .unwrap_or_else(|| "https://api.telnyx.com".into());
    http_verdict(
        "telnyx",
        answer(
            agent()
                .get(&format!("{}/v2/balance", base.trim_end_matches('/')))
                .header("Authorization", &format!("Bearer {key}"))
                .call(),
        ),
        "API key accepted (balance read); webhook key parses",
    )
}

fn check_smtp(v: &Values) -> CheckResult {
    use std::net::ToSocketAddrs as _;
    let Some(host) = v.get("mailer.host") else {
        return skip("smtp", "mailer.host unset");
    };
    let port: u16 = v
        .get("mailer.port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(587);
    let addr = match (host.as_str(), port)
        .to_socket_addrs()
        .map(|mut a| a.next())
    {
        Ok(Some(a)) => a,
        _ => return verdict("smtp", Status::Fail, format!("{host} does not resolve")),
    };
    match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
        Ok(_) => verdict(
            "smtp",
            Status::Warn,
            format!("{host}:{port} reachable; login is tested by the first send"),
        ),
        Err(e) => verdict("smtp", Status::Fail, format!("{host}:{port}: {e}")),
    }
}

fn check_eab(v: &Values) -> CheckResult {
    use base64::Engine as _;
    let (kid, hmac) = (v.get("acme.eab_kid"), v.get("acme.eab_hmac"));
    match (kid, hmac) {
        (None, None) => skip(
            "acme_eab",
            "no EAB (minted just in time with gcloud, or already registered)",
        ),
        (Some(_), Some(hmac)) => {
            let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(hmac.trim_end_matches('='))
                .map(|b| b.len());
            match decoded {
                Ok(n) if n >= 32 => verdict(
                    "acme_eab",
                    Status::Ok,
                    "format ok (single-use: not spent by a check)",
                ),
                Ok(n) => verdict(
                    "acme_eab",
                    Status::Fail,
                    format!("HMAC is {n} bytes; expected at least 32"),
                ),
                Err(_) => verdict("acme_eab", Status::Fail, "HMAC is not base64url"),
            }
        }
        _ => verdict(
            "acme_eab",
            Status::Fail,
            "set both acme.eab_kid and acme.eab_hmac, or neither",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_reduced_and_unset_providers_skip() {
        assert_eq!(
            error_code(r#"{"error":"invalid_grant","error_description":"Bad Request"}"#),
            "invalid_grant"
        );
        assert_eq!(error_code(r#"{"error":{"code":401,"message":"x"}}"#), "401");
        assert_eq!(
            error_code(r#"{"errors":[{"code":"NOT.AUTHORIZED.ERROR"}]}"#),
            "NOT.AUTHORIZED.ERROR"
        );
        assert_eq!(error_code("<html>"), "");
        let file = SetupFile::template();
        let results = run_target(&file, "local");
        assert_eq!(results.len(), CHECKS.len());
        assert!(
            results.iter().all(|r| r.status == Status::Skip),
            "{results:?}"
        );
    }

    #[test]
    fn offline_checks_classify_formats() {
        let mut values = BTreeMap::new();
        values.insert(
            "acme.eab_kid".to_string(),
            Value::String("kid-synthetic".into()),
        );
        values.insert("acme.eab_hmac".to_string(), Value::String("c2hvcnQ".into()));
        assert_eq!(check_eab(&Values(values.clone())).status, Status::Fail);
        values.insert(
            "acme.eab_hmac".to_string(),
            Value::String("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into()),
        );
        assert_eq!(check_eab(&Values(values)).status, Status::Ok);
        let mut live = BTreeMap::new();
        live.insert(
            "stripe.secret_key".to_string(),
            Value::String(format!("sk_{}_synthetic", "live")),
        );
        let r = check_stripe(&Values(live), false);
        assert_eq!(
            r.status,
            Status::Fail,
            "live key off production never even dials"
        );
        let mut bad = BTreeMap::new();
        bad.insert(
            "telnyx.api_key".to_string(),
            Value::String("KEYsynthetic".into()),
        );
        bad.insert(
            "telnyx.webhook_public_key".to_string(),
            Value::String("bm90LWEta2V5".into()),
        );
        assert_eq!(check_telnyx(&Values(bad.clone())).status, Status::Fail);
        // A valid webhook key still fails without one of the two keys
        // a working call needs (no network: both refuse first).
        bad.remove("telnyx.webhook_public_key");
        let r = check_telnyx(&Values(bad.clone()));
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.detail.contains("webhook_public_key unset"),
            "{}",
            r.detail
        );
        let synthetic_key = {
            use base64::Engine as _;
            let signing = ed25519_dalek::SigningKey::from_bytes(&[11u8; 32]);
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().as_bytes())
        };
        bad.insert(
            "telnyx.webhook_public_key".to_string(),
            Value::String(synthetic_key),
        );
        let r = check_telnyx(&Values(bad));
        assert_eq!(r.status, Status::Fail);
        assert!(
            r.detail.contains("webrtc_connection_id unset"),
            "{}",
            r.detail
        );
    }
}
