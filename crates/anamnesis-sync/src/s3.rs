//! An s3-compatible bucket the user configured themselves (aws s3, cloudflare r2,
//! backblaze b2, minio, …), addressed path-style and signed with aws signature v4.
//! Only ciphertext and the plain inbox ever go there.

use std::time::Duration;

use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Result, SyncError};
use crate::storage::Storage;

/// Where the bucket is. The keys are not here: the server keeps them in the os keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3Config {
    /// e.g. `https://<account>.r2.cloudflarestorage.com` or `https://s3.eu-west-1.amazonaws.com`.
    pub endpoint: String,
    pub bucket: String,
    /// `auto` for r2; the bucket's region for aws.
    #[serde(default = "default_region")]
    pub region: String,
    /// A folder inside the bucket (may be empty).
    #[serde(default)]
    pub prefix: String,
}

fn default_region() -> String {
    "auto".into()
}

pub struct S3Storage {
    cfg: S3Config,
    access_key: String,
    secret_key: String,
    agent: ureq::Agent,
}

/// aws's uri encoding: everything but `A-Z a-z 0-9 - _ . ~` (and `/` in paths).
const AWS: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn enc(s: &str) -> String {
    utf8_percent_encode(s, AWS).to_string()
}

fn enc_path(p: &str) -> String {
    p.split('/').map(enc).collect::<Vec<_>>().join("/")
}

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn hmac(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

/// The pieces of a signature v4 request.
pub(crate) struct Signed {
    pub authorization: String,
}

/// Sign a request (aws signature v4, service `s3`). `headers` must hold every header to
/// sign, lowercase names, including `host`, `x-amz-date` and `x-amz-content-sha256`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sign(
    method: &str,
    canonical_uri: &str,
    query: &[(String, String)],
    headers: &[(String, String)],
    payload_hash: &str,
    region: &str,
    access_key: &str,
    secret_key: &str,
    amz_date: &str,
) -> Signed {
    let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (enc(k), enc(v))).collect();
    q.sort();
    let canonical_query = q
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let mut hs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.trim().to_string()))
        .collect();
    hs.sort();
    let canonical_headers: String = hs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = hs
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let date = &amz_date[..8];
    let scope = format!("{date}/{region}/s3/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical.as_bytes())
    );
    let k_date = hmac(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, region.as_bytes());
    let k_service = hmac(&k_region, b"s3");
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex::encode(hmac(&k_signing, to_sign.as_bytes()));
    Signed {
        authorization: format!(
            "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
        ),
    }
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn xml_values<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(&after[..j]);
        rest = &after[j + close.len()..];
    }
    out
}

impl S3Storage {
    pub fn new(cfg: S3Config, access_key: String, secret_key: String) -> Result<S3Storage> {
        let ep = cfg.endpoint.trim_end_matches('/');
        if !ep.starts_with("https://")
            && !ep.starts_with("http://127.0.0.1")
            && !ep.starts_with("http://localhost")
        {
            return Err(SyncError::Invalid(
                "the endpoint must be an https url".into(),
            ));
        }
        if cfg.bucket.is_empty() || cfg.bucket.contains('/') {
            return Err(SyncError::Invalid("the bucket name is not valid".into()));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(120)))
            .build()
            .into();
        Ok(S3Storage {
            cfg: S3Config {
                endpoint: ep.to_string(),
                ..cfg
            },
            access_key,
            secret_key,
            agent,
        })
    }

    fn object_key(&self, path: &str) -> String {
        let p = self.cfg.prefix.trim_matches('/');
        if p.is_empty() {
            path.to_string()
        } else {
            format!("{p}/{path}")
        }
    }

    fn host(&self) -> String {
        self.cfg
            .endpoint
            .split_once("://")
            .map(|x| x.1)
            .unwrap_or(&self.cfg.endpoint)
            .to_string()
    }

    /// Send one signed request. Returns (status, body).
    fn request(
        &self,
        method: &str,
        key: Option<&str>,
        query: &[(String, String)],
        body: &[u8],
    ) -> Result<(u16, Vec<u8>)> {
        let uri = match key {
            Some(k) => format!("/{}/{}", enc(&self.cfg.bucket), enc_path(k)),
            None => format!("/{}", enc(&self.cfg.bucket)),
        };
        let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
        let payload_hash = sha256_hex(body);
        let headers = vec![
            ("host".to_string(), self.host()),
            ("x-amz-content-sha256".to_string(), payload_hash.clone()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        let s = sign(
            method,
            &uri,
            query,
            &headers,
            &payload_hash,
            &self.cfg.region,
            &self.access_key,
            &self.secret_key,
            &amz_date,
        );
        let mut url = format!("{}{uri}", self.cfg.endpoint);
        if !query.is_empty() {
            let q: Vec<String> = query
                .iter()
                .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
                .collect();
            url.push('?');
            url.push_str(&q.join("&"));
        }
        let io = |e: ureq::Error| SyncError::Io(format!("s3 {method}: {e}"));
        let mut resp = match method {
            "GET" => self
                .agent
                .get(&url)
                .header("x-amz-date", &amz_date)
                .header("x-amz-content-sha256", &payload_hash)
                .header("authorization", &s.authorization)
                .call()
                .map_err(io)?,
            "DELETE" => self
                .agent
                .delete(&url)
                .header("x-amz-date", &amz_date)
                .header("x-amz-content-sha256", &payload_hash)
                .header("authorization", &s.authorization)
                .call()
                .map_err(io)?,
            "PUT" => self
                .agent
                .put(&url)
                .header("x-amz-date", &amz_date)
                .header("x-amz-content-sha256", &payload_hash)
                .header("authorization", &s.authorization)
                .send(body)
                .map_err(io)?,
            other => return Err(SyncError::Invalid(format!("method {other}"))),
        };
        let status = resp.status().as_u16();
        let bytes = resp
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_vec()
            .map_err(io)?;
        Ok((status, bytes))
    }

    fn fail(status: u16, body: &[u8], what: &str) -> SyncError {
        let text = String::from_utf8_lossy(body);
        let code = xml_values(&text, "Code")
            .first()
            .map(|s| s.to_string())
            .unwrap_or_default();
        SyncError::Io(
            format!("s3 {what}: http {status} {code}")
                .trim()
                .to_string(),
        )
    }
}

impl Storage for S3Storage {
    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let full = self.object_key(prefix);
        let strip = {
            let p = self.cfg.prefix.trim_matches('/');
            if p.is_empty() {
                String::new()
            } else {
                format!("{p}/")
            }
        };
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut q = vec![
                ("list-type".to_string(), "2".to_string()),
                ("prefix".to_string(), full.clone()),
            ];
            if let Some(t) = &token {
                q.push(("continuation-token".to_string(), t.clone()));
            }
            let (status, body) = self.request("GET", None, &q, b"")?;
            if status != 200 {
                return Err(Self::fail(status, &body, "list"));
            }
            let xml = String::from_utf8_lossy(&body);
            for k in xml_values(&xml, "Key") {
                let k = xml_unescape(k);
                if let Some(rel) = k.strip_prefix(&strip) {
                    out.push(rel.to_string());
                }
            }
            let truncated = xml_values(&xml, "IsTruncated").first() == Some(&"true");
            token = xml_values(&xml, "NextContinuationToken")
                .first()
                .map(|t| xml_unescape(t));
            if !truncated || token.is_none() {
                break;
            }
        }
        out.sort();
        Ok(out)
    }

    fn get(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let (status, body) = self.request("GET", Some(&self.object_key(path)), &[], b"")?;
        match status {
            200 => Ok(Some(body)),
            404 => Ok(None),
            s => Err(Self::fail(s, &body, "get")),
        }
    }

    fn put(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let (status, body) = self.request("PUT", Some(&self.object_key(path)), &[], bytes)?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(Self::fail(status, &body, "put"))
        }
    }

    fn delete(&self, path: &str) -> Result<()> {
        let (status, body) = self.request("DELETE", Some(&self.object_key(path)), &[], b"")?;
        if (200..300).contains(&status) || status == 404 {
            Ok(())
        } else {
            Err(Self::fail(status, &body, "delete"))
        }
    }

    fn describe(&self) -> String {
        let p = self.cfg.prefix.trim_matches('/');
        format!(
            "s3: {}/{}{}",
            self.host(),
            self.cfg.bucket,
            if p.is_empty() {
                String::new()
            } else {
                format!("/{p}")
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "GET object" example of aws's signature v4 documentation.
    #[test]
    fn signature_v4_matches_the_aws_example() {
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let s = sign(
            "GET",
            "/test.txt",
            &[],
            &[
                ("host".into(), "examplebucket.s3.amazonaws.com".into()),
                ("range".into(), "bytes=0-9".into()),
                ("x-amz-content-sha256".into(), empty.into()),
                ("x-amz-date".into(), "20130524T000000Z".into()),
            ],
            empty,
            "us-east-1",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "20130524T000000Z",
        );
        assert_eq!(
            s.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn list_xml_and_config_checks() {
        let xml = "<ListBucketResult><IsTruncated>true</IsTruncated><Contents><Key>p/devices/a&amp;b/x.anb</Key></Contents><Contents><Key>p/inbox/n.md</Key></Contents><NextContinuationToken>t1</NextContinuationToken></ListBucketResult>";
        assert_eq!(
            xml_values(xml, "Key"),
            vec!["p/devices/a&amp;b/x.anb", "p/inbox/n.md"]
        );
        assert_eq!(xml_unescape("a&amp;b"), "a&b");
        assert_eq!(xml_values(xml, "IsTruncated"), vec!["true"]);
        let cfg = |ep: &str| S3Config {
            endpoint: ep.into(),
            bucket: "notes".into(),
            region: "auto".into(),
            prefix: "anamnesis".into(),
        };
        assert!(S3Storage::new(cfg("http://example.org"), "a".into(), "b".into()).is_err());
        let s = S3Storage::new(
            cfg("https://acct.r2.cloudflarestorage.com/"),
            "a".into(),
            "b".into(),
        )
        .unwrap();
        assert_eq!(s.object_key("vault.json"), "anamnesis/vault.json");
        assert_eq!(
            s.describe(),
            "s3: acct.r2.cloudflarestorage.com/notes/anamnesis"
        );
        assert_eq!(enc_path("inbox/a b+c.md"), "inbox/a%20b%2Bc.md");
    }
}
