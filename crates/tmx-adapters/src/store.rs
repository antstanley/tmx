//! The `reqwest`-backed [`S3ObjectStore`] adapter — the `store` executor.
//!
//! [`S3ObjectStore`] implements the `ObjectStore` port
//! ([`.specs/06-ports-and-adapters.md` §Executor ports](../../../../.specs/06-ports-and-adapters.md)):
//! the five `store` operations — `get` / `put` / `delete` / `list` / `head` — against an
//! S3-compatible endpoint (AWS S3, MinIO, LocalStack, …). Requests are signed with **AWS Signature
//! Version 4** (path-style addressing) and issued over the same `reqwest` client the `fetch` adapter
//! uses, so no heavy S3 SDK is pulled into the tree; the signing primitives (SHA-256, HMAC-SHA-256)
//! come from `ring`, already present transitively via `rustls`.
//!
//! The captured bytes of a `get` are bounded by
//! [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES) — an object larger
//! than the cap is a typed `output_too_large` error, read in bounded chunks and rejected before it is
//! ever fully buffered (the adversarial-response contract the `fetch` adapter also follows). Every
//! remote/host failure is translated into a typed [`RunError`] at the boundary via
//! [`From<S3Error>`](RunError) — a `get` of a missing key is a typed `object_not_found`, never a
//! panic (06 §Adapters return typed errors, never panic on host failure).
//!
//! **Credentials stay maskable.** The access-key id, secret access key, and any session token are
//! held on the adapter and surfaced by [`S3ObjectStore::credential_values`] so the run's `Masker` can
//! register them before any output is emitted; they are used only to *sign* a request (in the
//! `Authorization` header) and never appear in a [`StoreResult`], so no emitted payload carries a raw
//! credential value. This module lives behind the `store` Cargo feature so a minimal build can drop
//! it (and its `reqwest`/`tokio`/`ring` edge).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use tmx_core::error::RunError;
use tmx_core::model::Milliseconds;
use tmx_core::ports::driven::{ObjectStore, StoreOp, StoreResult};
use tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES;

/// The SigV4 service name for S3.
const S3_SERVICE: &str = "s3";
/// The SigV4 algorithm token.
const SIGV4_ALGORITHM: &str = "AWS4-HMAC-SHA256";
/// The SigV4 credential-scope terminator.
const SIGV4_TERMINATOR: &str = "aws4_request";
/// The default AWS region when none is configured.
const DEFAULT_REGION: &str = "us-east-1";
/// The hex SHA-256 of an empty payload — the body hash for `get`/`head`/`delete`/`list`.
const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// Seconds in a day — the divisor splitting a Unix timestamp into a day count and a time-of-day.
const SECONDS_PER_DAY: u64 = 86_400;
/// Seconds in an hour.
const SECONDS_PER_HOUR: u64 = 3_600;
/// Seconds in a minute.
const SECONDS_PER_MINUTE: u64 = 60;

/// The credentials an [`S3ObjectStore`] signs with — kept private and exposed only as maskable
/// values via [`S3ObjectStore::credential_values`], never embedded in a [`StoreResult`].
#[derive(Debug, Clone)]
pub struct S3Credentials {
    /// The access-key id.
    access_key_id: String,
    /// The secret access key — a sensitive value the `Masker` must register.
    secret_access_key: String,
    /// An optional session token (STS temporary credentials).
    session_token: Option<String>,
}

impl S3Credentials {
    /// Static credentials from an access-key id and secret access key.
    #[must_use]
    pub fn new(access_key_id: impl Into<String>, secret_access_key: impl Into<String>) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token: None,
        }
    }

    /// Attach a session token (temporary STS credentials).
    #[must_use]
    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into();
        self.session_token = if token.is_empty() { None } else { Some(token) };
        self
    }
}

/// The configuration an [`S3ObjectStore`] is constructed with — the `endpoint` / `region` /
/// `credentials` the port takes, plus the target `bucket` (path-style addressing).
#[derive(Debug, Clone)]
pub struct S3Config {
    /// The S3-compatible endpoint base URL (e.g. `http://localhost:9000` for MinIO). When empty, the
    /// AWS default `https://s3.{region}.amazonaws.com` is used.
    pub endpoint: String,
    /// The region the request is signed for.
    pub region: String,
    /// The target bucket.
    pub bucket: String,
    /// The signing credentials.
    pub credentials: S3Credentials,
}

/// The `store` executor: a `reqwest`-backed, SigV4-signing [`ObjectStore`] adapter.
///
/// Holds one `reqwest` client, the [`S3Config`] it signs and addresses with, and the captured-output
/// cap a `get` enforces (default [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES));
/// tests construct one with a tiny cap to exercise the `output_too_large` path with a small object.
#[derive(Debug, Clone)]
pub struct S3ObjectStore {
    /// The HTTP client every request rides.
    client: reqwest::Client,
    /// The endpoint / region / bucket / credentials configuration.
    config: S3Config,
    /// The captured-object ceiling, in bytes, a `get` enforces.
    output_cap_bytes: u64,
}

impl S3ObjectStore {
    /// An object store over `config`, bounding a `get` by
    /// [`CAPTURED_OUTPUT_MAX_BYTES`](tmx_schema::limits::CAPTURED_OUTPUT_MAX_BYTES).
    ///
    /// # Errors
    ///
    /// Returns a typed [`RunError`] (`store_client_init_failed`) if the underlying `reqwest` client
    /// cannot be built (e.g. the TLS backend fails to initialise).
    pub fn new(config: S3Config) -> Result<Self, RunError> {
        Self::with_output_cap_bytes(config, CAPTURED_OUTPUT_MAX_BYTES)
    }

    /// An object store with an explicit captured-object cap, in bytes — for tests exercising the
    /// `output_too_large` path with a small object.
    ///
    /// # Errors
    ///
    /// Returns a typed [`RunError`] (`store_client_init_failed`) if the `reqwest` client cannot be
    /// built.
    pub fn with_output_cap_bytes(
        config: S3Config,
        output_cap_bytes: u64,
    ) -> Result<Self, RunError> {
        let client = reqwest::Client::builder().build().map_err(|error| {
            RunError::run_failure(
                "store_client_init_failed",
                format!("failed to build the object-store HTTP client: {error}"),
            )
        })?;
        Ok(Self {
            client,
            config,
            output_cap_bytes,
        })
    }

    /// An object store configured from the standard AWS environment variables
    /// (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AWS_REGION`,
    /// `AWS_ENDPOINT_URL`) plus `TMX_STORE_BUCKET` for the target bucket — the composition-root wiring.
    /// Absent variables default to empty (the region defaults to `us-east-1`); a missing credential is
    /// surfaced only when a request is actually signed, never at construction.
    ///
    /// # Errors
    ///
    /// Returns a typed [`RunError`] if the `reqwest` client cannot be built.
    pub fn from_env() -> Result<Self, RunError> {
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| DEFAULT_REGION.to_string());
        let credentials = S3Credentials {
            access_key_id: std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_default(),
            secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default(),
            session_token: std::env::var("AWS_SESSION_TOKEN")
                .ok()
                .filter(|t| !t.is_empty()),
        };
        Self::new(S3Config {
            endpoint: std::env::var("AWS_ENDPOINT_URL").unwrap_or_default(),
            region,
            bucket: std::env::var("TMX_STORE_BUCKET").unwrap_or_default(),
            credentials,
        })
    }

    /// The sensitive credential values the run's `Masker` must register before any output, so no
    /// emitted payload can leak a raw credential. Empty values are omitted (they would match
    /// everywhere and are never a real secret).
    #[must_use]
    pub fn credential_values(&self) -> Vec<String> {
        let mut values = Vec::new();
        for value in [
            &self.config.credentials.access_key_id,
            &self.config.credentials.secret_access_key,
        ] {
            if !value.is_empty() {
                values.push(value.clone());
            }
        }
        if let Some(token) = &self.config.credentials.session_token
            && !token.is_empty()
        {
            values.push(token.clone());
        }
        values
    }

    /// The endpoint base URL, defaulting to the AWS regional endpoint when none is configured.
    fn endpoint_base(&self) -> String {
        let endpoint = self.config.endpoint.trim_end_matches('/');
        if endpoint.is_empty() {
            format!("https://s3.{}.amazonaws.com", self.config.region)
        } else {
            endpoint.to_string()
        }
    }

    /// Perform one object-store operation, returning a [`StoreResult`] or a typed [`S3Error`].
    ///
    /// `timeout`, when set, bounds every request the op issues (the per-task `timeout` from the
    /// `store` task) — a breach surfaces as a [`S3Error::Transport`] whose `is_timeout()` maps to a
    /// typed `task_timeout` at the port boundary, the same contract `fetch`/`exec`/`run` honour.
    async fn perform(
        &self,
        op: StoreOp,
        timeout: Option<Milliseconds>,
    ) -> Result<StoreResult, S3Error> {
        match op {
            StoreOp::Get { key } => {
                let response = self
                    .signed_request(reqwest::Method::GET, &key, "", &[], timeout)
                    .await?;
                let status = response.status().as_u16();
                if status == 404 {
                    return Err(S3Error::ObjectNotFound { key });
                }
                let response = ok_or_http(response, &key)?;
                let body = self.read_capped(response).await?;
                Ok(StoreResult::Get { body })
            }
            StoreOp::Put { key, body } => {
                let response = self
                    .signed_request(reqwest::Method::PUT, &key, "", &body, timeout)
                    .await?;
                let _ = ok_or_http(response, &key)?;
                Ok(StoreResult::Done)
            }
            StoreOp::Delete { key } => {
                let response = self
                    .signed_request(reqwest::Method::DELETE, &key, "", &[], timeout)
                    .await?;
                // S3 delete is idempotent: a 204 (deleted) and a 404 (already absent) are both success.
                let status = response.status().as_u16();
                if status != 404 {
                    let _ = ok_or_http(response, &key)?;
                }
                Ok(StoreResult::Done)
            }
            StoreOp::Head { key } => {
                let response = self
                    .signed_request(reqwest::Method::HEAD, &key, "", &[], timeout)
                    .await?;
                let status = response.status().as_u16();
                if status == 404 {
                    return Ok(StoreResult::Head {
                        exists: false,
                        size_bytes: None,
                    });
                }
                let response = ok_or_http(response, &key)?;
                let size_bytes = response
                    .headers()
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|text| text.parse::<u64>().ok());
                Ok(StoreResult::Head {
                    exists: true,
                    size_bytes,
                })
            }
            StoreOp::List { prefix } => {
                // ListObjectsV2: GET on the bucket with a `list-type=2` (+ optional `prefix`) query.
                let query = if prefix.is_empty() {
                    "list-type=2".to_string()
                } else {
                    format!("list-type=2&prefix={}", uri_encode(&prefix, true))
                };
                let response = self
                    .signed_request(reqwest::Method::GET, "", &query, &[], timeout)
                    .await?;
                let response = ok_or_http(response, "")?;
                let body = self.read_capped(response).await?;
                let text = String::from_utf8_lossy(&body);
                Ok(StoreResult::List {
                    keys: parse_list_keys(&text),
                })
            }
        }
    }

    /// Build, sign (SigV4), and send one request for `key` with `query` and `body`, bounded by
    /// `timeout` when set (the per-task `store` timeout).
    async fn signed_request(
        &self,
        method: reqwest::Method,
        key: &str,
        query: &str,
        body: &[u8],
        timeout: Option<Milliseconds>,
    ) -> Result<reqwest::Response, S3Error> {
        let base = self.endpoint_base();
        // The canonical path is `/{bucket}` for a bucket-level op (list) and `/{bucket}/{key}` for an
        // object op, each segment URI-encoded but with slashes in the key preserved.
        let canonical_uri = if key.is_empty() {
            format!("/{}", uri_encode(&self.config.bucket, false))
        } else {
            format!(
                "/{}/{}",
                uri_encode(&self.config.bucket, false),
                uri_encode(key, false)
            )
        };
        let url_string = if query.is_empty() {
            format!("{base}{canonical_uri}")
        } else {
            format!("{base}{canonical_uri}?{query}")
        };
        let url = reqwest::Url::parse(&url_string).map_err(|_| S3Error::InvalidUrl {
            url: url_string.clone(),
        })?;
        let host = host_header(&url).ok_or_else(|| S3Error::InvalidUrl {
            url: url_string.clone(),
        })?;

        // Every empty-body op (`get`/`head`/`delete`/`list`) shares the same well-known hash, so it
        // is taken from the named constant rather than re-hashed.
        let payload_hash = if body.is_empty() {
            EMPTY_PAYLOAD_SHA256.to_string()
        } else {
            sha256_hex(body)
        };
        let now = SystemTime::now();
        let amz_date = format_amz_datetime(now);
        let date_stamp = amz_date.get(..8).unwrap_or("").to_string();

        // The headers that participate in the signature, lowercase-named and sorted.
        let mut signed_headers: Vec<(String, String)> = vec![
            ("host".to_string(), host.clone()),
            ("x-amz-content-sha256".to_string(), payload_hash.clone()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];
        if let Some(token) = &self.config.credentials.session_token {
            signed_headers.push(("x-amz-security-token".to_string(), token.clone()));
        }

        let authorization = sigv4_authorization(
            method.as_str(),
            &canonical_uri,
            query,
            &signed_headers,
            &payload_hash,
            &self.config.credentials.access_key_id,
            &self.config.credentials.secret_access_key,
            &self.config.region,
            S3_SERVICE,
            &amz_date,
            &date_stamp,
        );

        let mut builder = self
            .client
            .request(method, url)
            .header("x-amz-date", &amz_date)
            .header("x-amz-content-sha256", &payload_hash)
            .header(reqwest::header::AUTHORIZATION, authorization);
        if let Some(token) = &self.config.credentials.session_token {
            builder = builder.header("x-amz-security-token", token);
        }
        if !body.is_empty() {
            builder = builder.body(body.to_vec());
        }
        // Apply the per-task timeout as a wall-clock request bound; a breach is a `reqwest` timeout
        // error whose `is_timeout()` maps to a typed `task_timeout` at the port boundary.
        if let Some(Milliseconds(ms)) = timeout {
            builder = builder.timeout(Duration::from_millis(ms));
        }
        builder.send().await.map_err(S3Error::Transport)
    }

    /// Read a response body, bounded by the adapter's cap. A body larger than the cap is
    /// [`S3Error::OutputTooLarge`] before it is ever fully buffered.
    async fn read_capped(&self, response: reqwest::Response) -> Result<Vec<u8>, S3Error> {
        let mut body = Vec::new();
        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(S3Error::Transport)? {
            body.extend_from_slice(&chunk);
            if body.len() as u64 > self.output_cap_bytes {
                return Err(S3Error::OutputTooLarge {
                    cap_bytes: self.output_cap_bytes,
                });
            }
        }
        Ok(body)
    }
}

#[async_trait]
impl ObjectStore for S3ObjectStore {
    async fn op(
        &self,
        op: StoreOp,
        timeout: Option<Milliseconds>,
    ) -> Result<StoreResult, RunError> {
        self.perform(op, timeout).await.map_err(RunError::from)
    }
}

/// Reject a non-2xx response as a typed [`S3Error::Http`], passing a 2xx response through.
fn ok_or_http(response: reqwest::Response, key: &str) -> Result<reqwest::Response, S3Error> {
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        Ok(response)
    } else {
        Err(S3Error::Http {
            status,
            key: key.to_string(),
        })
    }
}

/// The `Host` header value a request carries: `host` plus an explicit non-default `:port`.
fn host_header(url: &reqwest::Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

/// Extract the object keys from an S3 `ListObjectsV2` XML body — the `<Key>…</Key>` leaves.
///
/// A deliberately minimal scan (no XML dependency): S3/MinIO emit each object's key in a `<Key>`
/// element with no attributes. Basic XML entities are un-escaped so a key containing `&`/`<`/`>` is
/// recovered verbatim.
fn parse_list_keys(xml: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Key>") {
        let after = &rest[start + "<Key>".len()..];
        let Some(end) = after.find("</Key>") else {
            break;
        };
        keys.push(unescape_xml(&after[..end]));
        rest = &after[end + "</Key>".len()..];
    }
    keys
}

/// Un-escape the five predefined XML entities in a listed key.
fn unescape_xml(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

// =============================================================================================
// AWS Signature Version 4 — pure signing helpers (unit-tested against the AWS test vectors).
// =============================================================================================

/// Build the SigV4 `Authorization` header value for a request.
///
/// Pure and deterministic given its inputs: it composes the canonical request, the string-to-sign,
/// derives the signing key, and returns the full `Authorization` header. Unit-tested against the
/// published AWS `get-vanilla` test vector.
#[allow(clippy::too_many_arguments)]
fn sigv4_authorization(
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    signed_headers: &[(String, String)],
    payload_hash: &str,
    access_key_id: &str,
    secret_access_key: &str,
    region: &str,
    service: &str,
    amz_date: &str,
    date_stamp: &str,
) -> String {
    // Headers are signed in sorted, lowercase-name order; the value is trimmed of surrounding space.
    let mut headers: Vec<(String, String)> = signed_headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    headers.sort_by(|a, b| a.0.cmp(&b.0));

    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed_header_names: Vec<&str> = headers.iter().map(|(name, _)| name.as_str()).collect();
    let signed_header_list = signed_header_names.join(";");

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_header_list}\n{payload_hash}"
    );

    let credential_scope = format!("{date_stamp}/{region}/{service}/{SIGV4_TERMINATOR}");
    let string_to_sign = format!(
        "{SIGV4_ALGORITHM}\n{amz_date}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    let signing_key = signing_key(secret_access_key, date_stamp, region, service);
    let signature = hex_lower(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    format!(
        "{SIGV4_ALGORITHM} Credential={access_key_id}/{credential_scope}, SignedHeaders={signed_header_list}, Signature={signature}"
    )
}

/// Derive the SigV4 signing key: `HMAC(HMAC(HMAC(HMAC("AWS4"+secret, date), region), service), "aws4_request")`.
fn signing_key(secret_access_key: &str, date_stamp: &str, region: &str, service: &str) -> Vec<u8> {
    let k_secret = format!("AWS4{secret_access_key}");
    let k_date = hmac_sha256(k_secret.as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, SIGV4_TERMINATOR.as_bytes())
}

/// The hex-lowercase SHA-256 of `data`.
fn sha256_hex(data: &[u8]) -> String {
    hex_lower(ring::digest::digest(&ring::digest::SHA256, data).as_ref())
}

/// HMAC-SHA-256 of `data` under `key`.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    ring::hmac::sign(&key, data).as_ref().to_vec()
}

/// Lowercase-hex encode `bytes`.
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(HEX[(byte >> 4) as usize]));
        out.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    out
}

/// AWS-style URI encoding (RFC 3986 unreserved kept literal; everything else percent-encoded,
/// uppercase hex). When `encode_slash` is false, `/` is preserved (used for object-key paths).
fn uri_encode(input: &str, encode_slash: bool) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        let unreserved = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'.' | b'_' | b'~')
            || (!encode_slash && byte == b'/');
        if unreserved {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[(byte >> 4) as usize]));
            out.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    out
}

/// Format a [`SystemTime`] as the SigV4 `x-amz-date` basic-ISO instant `YYYYMMDDTHHMMSSZ` (UTC).
///
/// A time before the Unix epoch cannot occur for `SystemTime::now`; it is clamped to the epoch rather
/// than panicking.
fn format_amz_datetime(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let days = secs / SECONDS_PER_DAY;
    let seconds_of_day = secs % SECONDS_PER_DAY;
    let hour = seconds_of_day / SECONDS_PER_HOUR;
    let minute = (seconds_of_day % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    let second = seconds_of_day % SECONDS_PER_MINUTE;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// Convert a count of days since the Unix epoch into a `(year, month, day)` civil date (proleptic
/// Gregorian, UTC). Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // Shift the epoch to 0000-03-01 so leap days fall at the end of the 400-year era.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // day of era, [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year of era, [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year (Mar-based), [0, 365]
    let mp = (5 * doy + 2) / 153; // month, Mar=0..Feb=11
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    // Non-negative for every real timestamp; the casts are lossless in that range.
    (year.max(0) as u64, month as u64, day as u64)
}

/// The internal, adapter-local failure modes of an object-store request.
///
/// Kept private and translated to a typed [`RunError`] at the port boundary via
/// [`From<S3Error>`](RunError) — the same discipline the process/HTTP/filesystem adapters follow, so
/// every remote/host failure funnels through one typed translation, never a panic.
#[derive(Debug)]
enum S3Error {
    /// The composed request URL was not a valid URL.
    InvalidUrl {
        /// The offending URL string.
        url: String,
    },
    /// A transport failure sending the request or reading the response (connect, timeout, reset).
    Transport(reqwest::Error),
    /// A `get`/`head` for an object key that does not exist.
    ObjectNotFound {
        /// The missing object key.
        key: String,
    },
    /// A non-2xx HTTP status from the endpoint (auth, bucket-missing, server error, …).
    Http {
        /// The HTTP status code.
        status: u16,
        /// The object key the request concerned (empty for a bucket-level op).
        key: String,
    },
    /// A captured body exceeded the adapter's captured-output cap.
    OutputTooLarge {
        /// The cap that was exceeded, in bytes.
        cap_bytes: u64,
    },
}

impl From<S3Error> for RunError {
    fn from(error: S3Error) -> Self {
        match error {
            S3Error::InvalidUrl { url } => RunError::run_failure(
                "store_request_failed",
                format!("could not build a valid object-store URL: {url}"),
            ),
            S3Error::Transport(source) => {
                if source.is_timeout() {
                    RunError::run_failure(
                        "task_timeout",
                        format!("the object-store request exceeded its timeout: {source}"),
                    )
                } else {
                    RunError::run_failure(
                        "store_request_failed",
                        format!("the object-store request failed: {source}"),
                    )
                }
            }
            S3Error::ObjectNotFound { key } => {
                RunError::run_failure("object_not_found", format!("no such object: {key}"))
            }
            S3Error::Http { status, key } => {
                let target = if key.is_empty() {
                    "the bucket".to_string()
                } else {
                    format!("object {key}")
                };
                RunError::run_failure(
                    "store_request_failed",
                    format!("the object-store request for {target} returned HTTP {status}"),
                )
            }
            S3Error::OutputTooLarge { cap_bytes } => RunError::run_failure(
                "output_too_large",
                format!("the object exceeds the captured-output cap of {cap_bytes} bytes"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmx_core::Masker;
    use tmx_core::error::ErrorCategory;

    fn creds() -> S3Credentials {
        S3Credentials::new("AKIDEXAMPLE", "topsecretkeyvalue")
    }

    fn store_with_cap(cap: u64) -> S3ObjectStore {
        S3ObjectStore::with_output_cap_bytes(
            S3Config {
                endpoint: "http://localhost:9000".to_string(),
                region: "us-east-1".to_string(),
                bucket: "bucket".to_string(),
                credentials: creds(),
            },
            cap,
        )
        .expect("the client builds")
    }

    // -----------------------------------------------------------------------------------------
    // SigV4 — pinned to the published AWS `get-vanilla` test vector.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn sigv4_matches_the_aws_get_vanilla_vector() {
        // The canonical AWS SigV4 test-suite `get-vanilla` case: GET / on example.amazonaws.com,
        // service "service", region us-east-1, at 20150830T123600Z, empty payload. Its Authorization
        // header (hence signature) is published, so reproducing it proves the signer end to end.
        let headers = vec![
            ("host".to_string(), "example.amazonaws.com".to_string()),
            ("x-amz-date".to_string(), "20150830T123600Z".to_string()),
        ];
        let auth = sigv4_authorization(
            "GET",
            "/",
            "",
            &headers,
            EMPTY_PAYLOAD_SHA256,
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "service",
            "20150830T123600Z",
            "20150830",
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, \
             SignedHeaders=host;x-amz-date, \
             Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31",
            "the signer reproduces the published get-vanilla Authorization header"
        );
        // A second, independent assertion: the empty-payload constant is the SHA-256 of "".
        assert_eq!(
            sha256_hex(b""),
            EMPTY_PAYLOAD_SHA256,
            "the empty-payload hash constant matches SHA-256 of the empty string"
        );
    }

    #[test]
    fn amz_datetime_formats_a_known_instant() {
        // 1440938160 seconds after the epoch is 2015-08-30T12:36:00Z — the get-vanilla instant.
        let instant = UNIX_EPOCH + Duration::from_secs(1_440_938_160);
        assert_eq!(
            format_amz_datetime(instant),
            "20150830T123600Z",
            "the epoch→civil conversion and formatting match the known instant"
        );
        // The epoch itself is 1970-01-01T00:00:00Z.
        assert_eq!(
            format_amz_datetime(UNIX_EPOCH),
            "19700101T000000Z",
            "the Unix epoch formats to the expected instant"
        );
    }

    #[test]
    fn uri_encode_follows_the_aws_rules() {
        // Unreserved characters are literal; the space and other characters are percent-encoded with
        // uppercase hex; a slash is kept only when encode_slash is false.
        assert_eq!(
            uri_encode("aA1-._~", true),
            "aA1-._~",
            "unreserved stay literal"
        );
        assert_eq!(
            uri_encode("a b", true),
            "a%20b",
            "space is %20, uppercase hex"
        );
        assert_eq!(uri_encode("a/b", false), "a/b", "slash kept for a key path");
        assert_eq!(
            uri_encode("a/b", true),
            "a%2Fb",
            "slash encoded in a query value"
        );
    }

    // -----------------------------------------------------------------------------------------
    // Typed-error translation (negative space) — every failure mode is a typed RunError.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn missing_key_get_is_a_typed_object_not_found() {
        // The DoD's missing-key case, at the translation boundary: a `get` of an absent key is a
        // typed run failure with a stable code, never a panic.
        let error: RunError = S3Error::ObjectNotFound {
            key: "missing/object".to_string(),
        }
        .into();
        assert_eq!(
            error.category,
            ErrorCategory::RunFailure,
            "a missing object is a typed run failure"
        );
        assert_eq!(
            error.code, "object_not_found",
            "the missing-object code is stable and specific"
        );
    }

    #[test]
    fn over_cap_and_http_and_transport_errors_are_typed() {
        let over_cap: RunError = S3Error::OutputTooLarge { cap_bytes: 8 }.into();
        assert_eq!(over_cap.code, "output_too_large", "the over-cap limit code");
        assert_eq!(over_cap.category, ErrorCategory::RunFailure);

        let http: RunError = S3Error::Http {
            status: 403,
            key: "k".to_string(),
        }
        .into();
        assert_eq!(
            http.code, "store_request_failed",
            "a non-2xx is a request failure"
        );
        assert_eq!(http.category, ErrorCategory::RunFailure);

        let bad_url: RunError = S3Error::InvalidUrl {
            url: "not a url".to_string(),
        }
        .into();
        assert_eq!(
            bad_url.code, "store_request_failed",
            "a bad URL is a request failure"
        );
    }

    // -----------------------------------------------------------------------------------------
    // Credentials stay maskable and never appear in an emitted payload.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn credentials_are_surfaced_for_masking_and_redacted() {
        let store = store_with_cap(CAPTURED_OUTPUT_MAX_BYTES);
        let values = store.credential_values();
        assert!(
            values.contains(&"topsecretkeyvalue".to_string()),
            "the secret access key is surfaced for the Masker to register"
        );
        assert!(
            values.contains(&"AKIDEXAMPLE".to_string()),
            "the access-key id is surfaced too"
        );

        // Genuine masking (not a no-op): register the surfaced credentials, then a payload that echoes
        // the secret is scrubbed to the placeholder — no raw credential value survives.
        let mut masker = Masker::new();
        for value in &values {
            masker.register(value.clone());
        }
        masker.assert_ready(&values.iter().map(String::as_str).collect::<Vec<_>>());
        let payload = serde_json::json!({
            "note": "signed with topsecretkeyvalue in the header",
        });
        let masked = masker.redact_value(&payload);
        let text = serde_json::to_string(masked.get()).expect("serialises");
        assert!(
            !text.contains("topsecretkeyvalue"),
            "the raw secret must not survive redaction: {text}"
        );
    }

    #[test]
    fn store_results_never_carry_a_credential_value() {
        // Structural negative space: the StoreResult variants an op can emit carry object data only —
        // bytes, keys, existence, size — and no credential field, so an emitted payload cannot leak a
        // credential regardless of adapter behaviour.
        let results = [
            StoreResult::Get {
                body: b"object-bytes".to_vec(),
            },
            StoreResult::List {
                keys: vec!["a".to_string(), "b".to_string()],
            },
            StoreResult::Head {
                exists: true,
                size_bytes: Some(3),
            },
            StoreResult::Done,
        ];
        for result in &results {
            let rendered = format!("{result:?}");
            assert!(
                !rendered.contains("topsecretkeyvalue") && !rendered.contains("AKIDEXAMPLE"),
                "no StoreResult carries a credential value: {rendered}"
            );
        }
    }

    #[test]
    fn empty_credentials_are_not_registered_as_secrets() {
        // An unconfigured credential is empty; it must not be surfaced (an empty secret would match
        // everywhere). Negative-space companion to the masking guarantee.
        let store = S3ObjectStore::new(S3Config {
            endpoint: String::new(),
            region: "us-east-1".to_string(),
            bucket: "b".to_string(),
            credentials: S3Credentials::new("", ""),
        })
        .expect("builds");
        assert!(
            store.credential_values().is_empty(),
            "empty credential values are never surfaced as secrets"
        );
    }

    // -----------------------------------------------------------------------------------------
    // List-response parsing.
    // -----------------------------------------------------------------------------------------

    #[test]
    fn parse_list_keys_extracts_object_keys() {
        let xml = "<ListBucketResult><Contents><Key>a/one.txt</Key></Contents>\
                   <Contents><Key>a/two &amp; more.txt</Key></Contents></ListBucketResult>";
        let keys = parse_list_keys(xml);
        assert_eq!(keys.len(), 2, "both keys are extracted");
        assert_eq!(keys[0], "a/one.txt", "the first key is verbatim");
        assert_eq!(
            keys[1], "a/two & more.txt",
            "XML entities in a key are un-escaped"
        );
    }

    #[test]
    fn parse_list_keys_of_an_empty_listing_is_empty() {
        // Negative space: a listing with no Contents yields no keys, not a panic.
        let xml = "<ListBucketResult><Name>bucket</Name></ListBucketResult>";
        assert!(
            parse_list_keys(xml).is_empty(),
            "an empty listing yields no keys"
        );
    }

    // -----------------------------------------------------------------------------------------
    // Per-task timeout — a `store` op honours its `timeout` under the cancellation contract, the
    // same as `exec`/`run`/`fetch`: a breach is a typed `task_timeout`, at ~the timeout.
    // -----------------------------------------------------------------------------------------

    #[tokio::test]
    async fn a_store_op_against_a_silent_endpoint_times_out_typed_at_its_timeout() {
        use std::net::TcpListener;
        use std::time::Instant;

        // A server that accepts the connection but never replies, so the request hangs until the
        // per-op timeout fires (rather than the server's long hold).
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local listener");
        let addr = listener.local_addr().expect("the listener has an address");
        std::thread::spawn(move || {
            if let Some(Ok(stream)) = listener.incoming().next() {
                std::thread::sleep(Duration::from_secs(5));
                drop(stream);
            }
        });

        let store = S3ObjectStore::new(S3Config {
            endpoint: format!("http://{addr}"),
            region: "us-east-1".to_string(),
            bucket: "bucket".to_string(),
            credentials: creds(),
        })
        .expect("the client builds");

        let timeout_ms = 200;
        let started = Instant::now();
        let error = store
            .op(
                StoreOp::Get {
                    key: "slow/object".to_string(),
                },
                Some(Milliseconds(timeout_ms)),
            )
            .await
            .expect_err("a silent endpoint must time out, not hang");
        let elapsed = started.elapsed();

        assert_eq!(
            error.code, "task_timeout",
            "a per-op timeout breach is the same typed code as exec/run/fetch"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the op returns at ~its {timeout_ms}ms timeout, not after the server's 5s hold: {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn a_store_op_with_no_timeout_is_not_bounded_by_a_request_timeout() {
        // Negative-space companion: with `timeout: None` no request timeout is applied — the op
        // fails only on the transport error it actually hits (connection refused here), never a
        // spurious `task_timeout`. Proves the timeout is opt-in, threaded from the task.
        let store = S3ObjectStore::new(S3Config {
            // Port 1 is unbound, so the connect is refused immediately (a fast, deterministic error).
            endpoint: "http://127.0.0.1:1".to_string(),
            region: "us-east-1".to_string(),
            bucket: "bucket".to_string(),
            credentials: creds(),
        })
        .expect("the client builds");

        let error = store
            .op(
                StoreOp::Get {
                    key: "k".to_string(),
                },
                None,
            )
            .await
            .expect_err("a refused connection is an error, not a hang");
        assert_ne!(
            error.code, "task_timeout",
            "with no timeout set, a transport error is not reported as a timeout"
        );
        assert_eq!(
            error.code, "store_request_failed",
            "a connection failure with no timeout is a plain request failure"
        );
    }

    // -----------------------------------------------------------------------------------------
    // Reviewable integration flow — requires a local S3-compatible endpoint (MinIO/LocalStack).
    // Run with: TMX_STORE_BUCKET=<bucket> AWS_ENDPOINT_URL=http://localhost:9000 \
    //   AWS_ACCESS_KEY_ID=... AWS_SECRET_ACCESS_KEY=... AWS_REGION=us-east-1 \
    //   cargo nextest run -p tmx-adapters --features store -- --ignored
    // -----------------------------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires a local S3-compatible endpoint (set AWS_*/TMX_STORE_BUCKET env)"]
    async fn put_head_get_list_delete_round_trip() {
        let store = S3ObjectStore::from_env().expect("the client builds");
        let key = format!("tmx-it/{}.txt", std::process::id());
        let body = b"tmx object payload".to_vec();

        // put
        let put = store
            .op(
                StoreOp::Put {
                    key: key.clone(),
                    body: body.clone(),
                },
                None,
            )
            .await
            .expect("put succeeds");
        assert_eq!(put, StoreResult::Done, "put reports Done");

        // head — exists with the right size
        let head = store
            .op(StoreOp::Head { key: key.clone() }, None)
            .await
            .expect("head succeeds");
        assert_eq!(
            head,
            StoreResult::Head {
                exists: true,
                size_bytes: Some(body.len() as u64),
            },
            "head reflects the put object's size"
        );

        // get — the bytes round-trip
        let got = store
            .op(StoreOp::Get { key: key.clone() }, None)
            .await
            .expect("get succeeds");
        assert_eq!(
            got,
            StoreResult::Get { body: body.clone() },
            "get returns the put bytes"
        );

        // list — the key is present under its prefix
        let list = store
            .op(
                StoreOp::List {
                    prefix: "tmx-it/".to_string(),
                },
                None,
            )
            .await
            .expect("list succeeds");
        match list {
            StoreResult::List { keys } => {
                assert!(
                    keys.contains(&key),
                    "the listing includes the put key: {keys:?}"
                );
            }
            other => panic!("expected a List result, got {other:?}"),
        }

        // delete — then head shows it gone
        store
            .op(StoreOp::Delete { key: key.clone() }, None)
            .await
            .expect("delete succeeds");
        let gone = store
            .op(StoreOp::Head { key: key.clone() }, None)
            .await
            .expect("head after delete succeeds");
        assert_eq!(
            gone,
            StoreResult::Head {
                exists: false,
                size_bytes: None,
            },
            "the object is absent after delete"
        );
    }

    #[tokio::test]
    #[ignore = "requires a local S3-compatible endpoint (set AWS_*/TMX_STORE_BUCKET env)"]
    async fn get_of_a_missing_key_is_object_not_found() {
        let store = S3ObjectStore::from_env().expect("the client builds");
        let error = store
            .op(
                StoreOp::Get {
                    key: format!("tmx-it/definitely-absent-{}", std::process::id()),
                },
                None,
            )
            .await
            .expect_err("a missing key is an error, not a panic");
        assert_eq!(
            error.code, "object_not_found",
            "a real missing get is typed"
        );
    }
}
