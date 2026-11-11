//! "Continue with ChatGPT" (Sign in with ChatGPT, plan usage for open-source / local apps):
//! OAuth 2.0 authorization code + PKCE with a loopback redirect and runtime client
//! registration (no client secret).
//!
//! Primary sources (retrieved 2026-11-11):
//! - flow, parameters, callback, token exchange, credential storage:
//!   https://developers.openai.com/siwc/token-sharing-open-source/sign-in
//! - reference implementation (discovery, scopes, refresh and revocation bodies, id-token checks):
//!   https://github.com/openai/sign-in-with-chatgpt-devkit/blob/0a36fefeb913055c8c7a1a29b63d82b96b2e841a/packages/local/src/oauth.ts
//! - overview and eligibility: https://developers.openai.com/siwc/token-sharing-open-source
//! - errors / refresh failures: https://developers.openai.com/siwc/token-sharing-open-source/errors-and-recovery
//! - cookbook: https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::store::StoredCredentials;
use crate::error::{HarnessError, Result};

pub const DEFAULT_ISSUER: &str = "https://auth.openai.com";
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub const CALLBACK_PATH: &str = "/auth/callback";
/// First-time registration uses this client id; the callback returns the issued `oaiapp_…` id.
pub const DYNAMIC_CLIENT_ID: &str = "dynamic_agent_client";
pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
pub const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
/// Documented endpoints, used only if discovery fails.
const FALLBACK_AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const FALLBACK_TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";

/// Refresh errors that mean the refresh token is unusable (sign in again).
const TERMINAL_REFRESH_ERRORS: &[&str] = &[
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
    #[serde(default)]
    pub jwks_uri: Option<String>,
}

fn origin(u: &str) -> Option<String> {
    let u = url::Url::parse(u).ok()?;
    Some(u.origin().ascii_serialization())
}

/// Fetch `{issuer}/.well-known/openid-configuration`; every endpoint must share the issuer's origin.
pub async fn discover(client: &reqwest::Client, issuer: &str) -> Result<Discovery> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let d: Discovery = client
        .get(url)
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()
        .map_err(|e| HarnessError::OAuth(e.to_string()))?
        .json()
        .await?;
    let want = origin(issuer);
    let ok = |u: &str| origin(u) == want;
    if !ok(&d.issuer)
        || !ok(&d.authorization_endpoint)
        || !ok(&d.token_endpoint)
        || d.revocation_endpoint.as_deref().is_some_and(|u| !ok(u))
        || d.jwks_uri.as_deref().is_some_and(|u| !ok(u))
    {
        return Err(HarnessError::OAuth(
            "discovery endpoints do not match the issuer origin".into(),
        ));
    }
    Ok(d)
}

pub async fn discover_or_documented(client: &reqwest::Client, issuer: &str) -> Discovery {
    match discover(client, issuer).await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("chatgpt oauth discovery failed ({e}); using documented endpoints");
            Discovery {
                issuer: issuer.to_string(),
                authorization_endpoint: FALLBACK_AUTHORIZE.into(),
                token_endpoint: FALLBACK_TOKEN.into(),
                revocation_endpoint: None,
                jwks_uri: None,
            }
        }
    }
}

pub fn random_value() -> String {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("os randomness");
    B64.encode(b)
}

/// PKCE S256 challenge: base64url(sha256(verifier)) without padding.
pub fn pkce_challenge(verifier: &str) -> String {
    B64.encode(Sha256::digest(verifier.as_bytes()))
}

/// Opaque per-installation host id (`urn:uuid:…`), never derived from user data.
pub fn new_host_id() -> String {
    format!("urn:uuid:{}", uuid::Uuid::new_v4())
}

pub struct AuthorizeParams<'a> {
    pub authorization_endpoint: &'a str,
    pub client_id: Option<&'a str>,
    pub app_name: &'a str,
    pub host_id: &'a str,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub nonce: &'a str,
    pub code_challenge: &'a str,
    pub login_hint: Option<&'a str>,
    pub reconsent: bool,
}

pub fn authorize_url(p: &AuthorizeParams) -> String {
    let mut u = url::Url::parse(p.authorization_endpoint).expect("authorization endpoint url");
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("client_id", p.client_id.unwrap_or(DYNAMIC_CLIENT_ID));
        q.append_pair("response_type", "code");
        q.append_pair("redirect_uri", p.redirect_uri);
        q.append_pair("scope", SCOPES);
        q.append_pair("resource", RESOURCE);
        q.append_pair("state", p.state);
        q.append_pair("nonce", p.nonce);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("code_challenge", p.code_challenge);
        q.append_pair("ext_agent_host_id", p.host_id);
        if p.client_id.is_none() {
            // only on initial registration
            q.append_pair("agent_name_hint", p.app_name);
        }
        if let Some(h) = p.login_hint {
            q.append_pair("login_hint", h);
        }
        if p.reconsent {
            q.append_pair("prompt", "consent");
        }
    }
    u.to_string()
}

/// What the browser sends back to the loopback listener.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Callback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub client_id: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

pub fn parse_callback(path_and_query: &str) -> Option<Callback> {
    let u = url::Url::parse(&format!("http://127.0.0.1{path_and_query}")).ok()?;
    if u.path() != CALLBACK_PATH {
        return None;
    }
    let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
    Some(Callback {
        code: q.get("code").cloned(),
        state: q.get("state").cloned(),
        client_id: q.get("client_id").cloned(),
        scope: q.get("scope").cloned(),
        error: q.get("error").cloned(),
    })
}

/// A loopback listener on 127.0.0.1 (never `localhost`, per the docs).
pub struct LoopbackListener {
    listener: TcpListener,
    pub redirect_uri: String,
}

impl LoopbackListener {
    /// Bind `port`, or any free port if it is taken or 0. Later sign-ins may use a different
    /// port; scheme, host and path stay the same.
    pub async fn bind(port: u16) -> Result<Self> {
        let listener = match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(_) if port != 0 => TcpListener::bind(("127.0.0.1", 0)).await?,
            Err(e) => return Err(e.into()),
        };
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            redirect_uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}"),
        })
    }

    /// Wait for the browser's redirect; answers with a short page. Other requests
    /// (favicon…) get 404 and the wait continues.
    pub async fn wait(self, timeout: Duration) -> Result<Callback> {
        let fut = async {
            loop {
                let (mut sock, _) = self.listener.accept().await?;
                let mut buf = vec![0u8; 8192];
                let n = tokio::time::timeout(Duration::from_secs(10), sock.read(&mut buf))
                    .await
                    .unwrap_or(Ok(0))?;
                let req = String::from_utf8_lossy(&buf[..n]);
                let target = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("");
                match parse_callback(target) {
                    Some(cb) => {
                        let body = if cb.error.is_some() {
                            "<!doctype html><meta charset=utf-8><title>anamnesis</title><body style=\"font-family:sans-serif;background:#111;color:#ddd\"><p>sign-in was cancelled. you can close this tab.</p>"
                        } else {
                            "<!doctype html><meta charset=utf-8><title>anamnesis</title><body style=\"font-family:sans-serif;background:#111;color:#ddd\"><p>signed in. you can close this tab and return to anamnesis.</p>"
                        };
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\ncache-control: no-store\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = sock.write_all(resp.as_bytes()).await;
                        let _ = sock.shutdown().await;
                        return Ok::<_, HarnessError>(cb);
                    }
                    None => {
                        let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                    }
                }
            }
        };
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| HarnessError::OAuth("sign-in timed out".into()))?
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub scope: Option<String>,
}

async fn token_request(
    client: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
) -> Result<TokenResponse> {
    let resp = client
        .post(endpoint)
        .header("accept", "application/json")
        .form(form)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;
    if resp.status().is_success() {
        return Ok(resp.json().await?);
    }
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let code = body
        .get("error")
        .and_then(|e| {
            e.as_str()
                .map(str::to_string)
                .or_else(|| e.get("code").and_then(|c| c.as_str()).map(str::to_string))
        })
        .unwrap_or_else(|| format!("http {status}"));
    Err(HarnessError::OAuth(code))
}

pub async fn exchange_code(
    client: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse> {
    token_request(
        client,
        token_endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
            ("resource", RESOURCE),
        ],
    )
    .await
}

pub async fn refresh(
    client: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<TokenResponse> {
    token_request(
        client,
        token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh_token),
            ("resource", RESOURCE),
        ],
    )
    .await
}

pub fn is_terminal_refresh_error(e: &HarnessError) -> bool {
    matches!(e, HarnessError::OAuth(c) if TERMINAL_REFRESH_ERRORS.contains(&c.as_str()))
}

pub async fn revoke(
    client: &reqwest::Client,
    endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<()> {
    let mut last = None;
    for _ in 0..2 {
        match client
            .post(endpoint)
            .form(&[
                ("token", refresh_token),
                ("token_type_hint", "refresh_token"),
                ("client_id", client_id),
            ])
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => {
                last = Some(HarnessError::OAuth(format!(
                    "revoke http {}",
                    r.status().as_u16()
                )))
            }
            Err(e) => last = Some(e.into()),
        }
    }
    Err(last.unwrap_or_else(|| HarnessError::OAuth("revoke failed".into())))
}

/// Apply a token response to the stored record (access, expiry, scopes and rotating
/// refresh token are replaced together).
pub fn apply_tokens(creds: &mut StoredCredentials, t: &TokenResponse, now: i64) {
    creds.access_token = Some(t.access_token.clone());
    if let Some(r) = &t.refresh_token {
        creds.refresh_token = Some(r.clone());
    }
    if let Some(i) = &t.id_token {
        creds.id_token = Some(i.clone());
    }
    creds.token_type = t.token_type.clone().or(creds.token_type.take());
    creds.expires_at = t.expires_in.map(|s| now + s);
    if let Some(s) = &t.scope {
        creds.scopes = s.split_whitespace().map(str::to_string).collect();
    }
    creds.saved_at = Some(now);
}

// ---- id token verification (RS256 against the issuer's jwks) -------------------------

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct IdClaims {
    pub iss: String,
    pub sub: String,
    pub aud: Value,
    pub exp: i64,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

fn decode_part<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    let bytes = B64
        .decode(s.trim_end_matches('='))
        .map_err(|_| HarnessError::OAuth("invalid_id_token".into()))?;
    serde_json::from_slice(&bytes).map_err(|_| HarnessError::OAuth("invalid_id_token".into()))
}

/// Check the claims: issuer, audience (= issued client id), expiry (5 s tolerance), nonce.
pub fn check_claims(
    claims: &IdClaims,
    issuer: &str,
    client_id: &str,
    nonce: Option<&str>,
    now: i64,
) -> Result<()> {
    let bad = |why: &str| Err(HarnessError::OAuth(format!("invalid_id_token: {why}")));
    if claims.iss.trim_end_matches('/') != issuer.trim_end_matches('/') {
        return bad("issuer");
    }
    let aud_ok = match &claims.aud {
        Value::String(a) => a == client_id,
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(client_id)),
        _ => false,
    };
    if !aud_ok {
        return bad("audience");
    }
    if claims.exp + 5 < now {
        return bad("expired");
    }
    if let Some(n) = nonce
        && claims.nonce.as_deref() != Some(n)
    {
        return bad("nonce");
    }
    Ok(())
}

/// Verify an RS256 signature against a jwks document.
pub fn verify_signature(id_token: &str, jwks: &Value) -> Result<()> {
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() != 3 {
        return Err(HarnessError::OAuth("invalid_id_token".into()));
    }
    let header: Value = decode_part(parts[0])?;
    if header.get("alg").and_then(|a| a.as_str()) != Some("RS256") {
        return Err(HarnessError::OAuth("invalid_id_token: alg".into()));
    }
    let kid = header.get("kid").and_then(|k| k.as_str());
    let sig = B64
        .decode(parts[2])
        .map_err(|_| HarnessError::OAuth("invalid_id_token".into()))?;
    let msg = format!("{}.{}", parts[0], parts[1]);
    let keys = jwks
        .get("keys")
        .and_then(|k| k.as_array())
        .cloned()
        .unwrap_or_default();
    for k in keys
        .iter()
        .filter(|k| k.get("kty").and_then(|t| t.as_str()) == Some("RSA"))
    {
        if let (Some(want), Some(have)) = (kid, k.get("kid").and_then(|x| x.as_str()))
            && want != have
        {
            continue;
        }
        let (Some(n), Some(e)) = (
            k.get("n").and_then(|x| x.as_str()),
            k.get("e").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        let (Ok(n), Ok(e)) = (B64.decode(n), B64.decode(e)) else {
            continue;
        };
        let pk = ring::signature::RsaPublicKeyComponents { n: &n, e: &e };
        if pk
            .verify(
                &ring::signature::RSA_PKCS1_2048_8192_SHA256,
                msg.as_bytes(),
                &sig,
            )
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(HarnessError::OAuth("invalid_id_token: signature".into()))
}

pub fn decode_claims(id_token: &str) -> Result<IdClaims> {
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or_else(|| HarnessError::OAuth("invalid_id_token".into()))?;
    decode_part(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_rfc7636_vector() {
        // RFC 7636 appendix B
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_has_documented_params() {
        let u = authorize_url(&AuthorizeParams {
            authorization_endpoint: "https://auth.example.com/authorize",
            client_id: None,
            app_name: "anamnesis",
            host_id: "urn:uuid:1",
            redirect_uri: "http://127.0.0.1:1455/auth/callback",
            state: "s",
            nonce: "n",
            code_challenge: "c",
            login_hint: None,
            reconsent: false,
        });
        let q: HashMap<String, String> = url::Url::parse(&u)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        assert_eq!(q["client_id"], DYNAMIC_CLIENT_ID);
        assert_eq!(q["agent_name_hint"], "anamnesis");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["resource"], RESOURCE);
        assert!(q["scope"].split(' ').any(|s| s == PLAN_SCOPE));
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:1455/auth/callback");

        let u = authorize_url(&AuthorizeParams {
            client_id: Some("oaiapp_1"),
            login_hint: Some("a@b.c"),
            ..AuthorizeParams {
                authorization_endpoint: "https://auth.example.com/authorize",
                client_id: None,
                app_name: "anamnesis",
                host_id: "urn:uuid:1",
                redirect_uri: "r",
                state: "s",
                nonce: "n",
                code_challenge: "c",
                login_hint: None,
                reconsent: false,
            }
        });
        let q: HashMap<String, String> = url::Url::parse(&u)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        assert_eq!(q["client_id"], "oaiapp_1");
        assert!(!q.contains_key("agent_name_hint"));
    }

    #[test]
    fn callback_parsing() {
        let cb = parse_callback("/auth/callback?code=c1&state=s1&client_id=oaiapp_9&scope=openid+chatgpt.tokens.use.direct")
            .unwrap();
        assert_eq!(cb.code.as_deref(), Some("c1"));
        assert_eq!(cb.client_id.as_deref(), Some("oaiapp_9"));
        assert!(cb.scope.unwrap().contains(PLAN_SCOPE));
        assert!(parse_callback("/favicon.ico").is_none());
    }

    #[test]
    fn claims_checks() {
        let c = IdClaims {
            iss: "https://auth.openai.com".into(),
            sub: "u".into(),
            aud: Value::String("oaiapp_1".into()),
            exp: 1000,
            nonce: Some("n".into()),
            email: None,
        };
        assert!(check_claims(&c, "https://auth.openai.com", "oaiapp_1", Some("n"), 900).is_ok());
        assert!(check_claims(&c, "https://evil", "oaiapp_1", Some("n"), 900).is_err());
        assert!(check_claims(&c, "https://auth.openai.com", "other", Some("n"), 900).is_err());
        assert!(check_claims(&c, "https://auth.openai.com", "oaiapp_1", Some("x"), 900).is_err());
        assert!(check_claims(&c, "https://auth.openai.com", "oaiapp_1", Some("n"), 2000).is_err());
    }

    #[test]
    fn bad_signature_rejected() {
        let h = B64.encode(br#"{"alg":"RS256","kid":"k"}"#);
        let p = B64.encode(br#"{"iss":"x"}"#);
        let tok = format!("{h}.{p}.{}", B64.encode([1u8; 256]));
        let jwks = serde_json::json!({"keys":[{"kty":"RSA","kid":"k","n":B64.encode([0xC5u8; 256]),"e":"AQAB"}]});
        assert!(verify_signature(&tok, &jwks).is_err());
        let none = format!("{}.{p}.", B64.encode(br#"{"alg":"none"}"#));
        assert!(verify_signature(&none, &jwks).is_err());
    }

    #[test]
    fn terminal_refresh_errors() {
        assert!(is_terminal_refresh_error(&HarnessError::OAuth(
            "invalid_grant".into()
        )));
        assert!(!is_terminal_refresh_error(&HarnessError::OAuth(
            "http 503".into()
        )));
    }
}
