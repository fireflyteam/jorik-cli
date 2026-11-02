//! One-shot, loopback-only browser login shared by CLI and TUI.
//! The nonce is in the callback path, which the existing backend preserves.
//! Only a one-time authorization code reaches loopback; bearer credentials are
//! obtained from the server over a separate S256 PKCE exchange.

use crate::{
    api,
    credentials::{self, Auth},
};
use anyhow::{Result, anyhow, bail, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, Url};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_util::sync::CancellationToken;
use zeroize::{Zeroize, Zeroizing};

const MAX_REQUEST: usize = 16 * 1024;

#[derive(Clone, Copy)]
struct Limits {
    total: Duration,
    read: Duration,
    write: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            total: Duration::from_secs(120),
            read: Duration::from_secs(3),
            write: Duration::from_secs(2),
        }
    }
}

pub struct LoginSession {
    listener: TcpListener,
    authority: String,
    path: String,
    nonce: String,
    origin: String,
    authorize: Url,
    exchange: Url,
    verifier: Zeroizing<String>,
    limits: Limits,
}

pub struct PendingLogin {
    pub auth: Auth,
    stream: TcpStream,
    nonce: String,
    write_timeout: Duration,
}

#[derive(Clone, Copy, Debug)]
struct Rejection(&'static str, &'static str);

impl LoginSession {
    pub async fn bind(base: &str) -> Result<Self> {
        Self::with_limits(base, Limits::default()).await
    }

    async fn with_limits(base: &str, limits: Limits) -> Result<Self> {
        let server = credentials::server_identity(base)?;
        let origin = Url::parse(&server)?.origin().ascii_serialization();
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| anyhow!("cannot bind the local authentication listener"))?;
        let authority = listener.local_addr()?.to_string();
        let mut random = [0; 32];
        getrandom::fill(&mut random)
            .map_err(|_| anyhow!("cannot generate a secure login nonce"))?;
        let nonce = URL_SAFE_NO_PAD.encode(random);
        getrandom::fill(&mut random).map_err(|_| anyhow!("cannot generate PKCE verifier"))?;
        let verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(random));
        random.zeroize();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let exchange = Url::parse(&api::build_url(&server, "/oauth/cli/exchange"))?;
        let path = format!("/oauth-callback/{nonce}");
        let callback = format!("http://{authority}{path}");
        let mut authorize = Url::parse(&api::build_url(&server, "/authorize"))?;
        authorize
            .query_pairs_mut()
            .append_pair("callback", &callback)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(Self {
            listener,
            authority,
            path,
            nonce,
            origin,
            authorize,
            exchange,
            verifier,
            limits,
        })
    }

    pub fn authorization_url(&self) -> &str {
        self.authorize.as_str()
    }

    pub async fn wait(self, cancel: &CancellationToken) -> Result<PendingLogin> {
        let total = self.limits.total;
        tokio::select! {
            biased;
            _ = cancel.cancelled() => bail!("login cancelled"),
            result = timeout(total, self.receive()) => {
                result.map_err(|_| anyhow!("login timed out after 120 seconds; start a new login"))?
            }
        }
    }

    async fn receive(self) -> Result<PendingLogin> {
        loop {
            let (mut stream, peer) = self
                .listener
                .accept()
                .await
                .map_err(|_| anyhow!("local login listener failed"))?;
            if !peer.ip().is_loopback() {
                continue;
            }
            let result = timeout(self.limits.read, self.read_request(&mut stream))
                .await
                .unwrap_or(Err(Rejection("408 Request Timeout", "Request timed out.")));
            match result {
                Ok(code) => {
                    let auth = match self.exchange_code(&code).await {
                        Ok(auth) => auth,
                        Err(error) => {
                            let _ = send_page(
                                &mut stream,
                                "502 Bad Gateway",
                                "Login exchange failed. Return to Jorik for details.",
                                Some(&self.nonce),
                                self.limits.write,
                            )
                            .await;
                            return Err(error);
                        }
                    };
                    return Ok(PendingLogin {
                        auth,
                        stream,
                        nonce: self.nonce,
                        write_timeout: self.limits.write,
                    });
                }
                Err(Rejection(status, message)) => {
                    // Never echo a request target, token, header, or native error.
                    let terminal = matches!(
                        message,
                        "Legacy bearer callbacks are not supported; update the OAuth server."
                            | "Discord authorization was denied."
                    );
                    let _ = send_page(
                        &mut stream,
                        status,
                        message,
                        terminal.then_some(self.nonce.as_str()),
                        self.limits.write,
                    )
                    .await;
                    if terminal {
                        bail!(message);
                    }
                }
            }
        }
    }

    async fn read_request(
        &self,
        stream: &mut TcpStream,
    ) -> std::result::Result<Zeroizing<String>, Rejection> {
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        let mut chunk = zeroize::Zeroizing::new([0; 2048]);
        loop {
            let count = stream
                .read(&mut chunk[..])
                .await
                .map_err(|_| Rejection("400 Bad Request", "Invalid request."))?;
            if count == 0 {
                return Err(Rejection("400 Bad Request", "Incomplete request."));
            }
            if count > MAX_REQUEST.saturating_sub(bytes.len()) {
                return Err(Rejection(
                    "431 Request Header Fields Too Large",
                    "Request too large.",
                ));
            }
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(auth) = self.parse_request(&bytes)? {
                return Ok(auth);
            }
            if bytes.len() == MAX_REQUEST {
                return Err(Rejection(
                    "431 Request Header Fields Too Large",
                    "Request too large.",
                ));
            }
        }
    }

    fn parse_request(
        &self,
        bytes: &[u8],
    ) -> std::result::Result<Option<Zeroizing<String>>, Rejection> {
        let invalid = Rejection("400 Bad Request", "Invalid login callback.");
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut request = httparse::Request::new(&mut headers);
        let header_end = match request.parse(bytes).map_err(|_| invalid)? {
            httparse::Status::Partial => return Ok(None),
            httparse::Status::Complete(end) => end,
        };
        if request.method != Some("GET") {
            return Err(Rejection("405 Method Not Allowed", "Use GET."));
        }
        if bytes.len() != header_end {
            return Err(invalid);
        }
        let target = request.path.ok_or(invalid)?;
        if !target.starts_with('/') || target.starts_with("//") || target.contains('#') {
            return Err(invalid);
        }
        let (path, _) = target.split_once('?').ok_or(invalid)?;
        if path != self.path {
            return Err(Rejection("404 Not Found", "Not found."));
        }
        let hosts = request
            .headers
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case("host"))
            .collect::<Vec<_>>();
        if hosts.len() != 1 || hosts[0].value != self.authority.as_bytes() {
            return Err(Rejection("403 Forbidden", "Invalid callback host."));
        }
        let origins = request
            .headers
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case("origin"))
            .collect::<Vec<_>>();
        if origins.len() > 1
            || origins
                .first()
                .is_some_and(|origin| origin.value != self.origin.as_bytes())
        {
            return Err(Rejection("403 Forbidden", "Invalid callback origin."));
        }
        let mut content_length = false;
        for header in request.headers {
            if header.name.eq_ignore_ascii_case("transfer-encoding") {
                return Err(invalid);
            }
            if header.name.eq_ignore_ascii_case("content-length") {
                if content_length || header.value != b"0" {
                    return Err(invalid);
                }
                content_length = true;
            }
            if header.name.eq_ignore_ascii_case("sec-fetch-mode") && header.value != b"navigate" {
                return Err(Rejection("403 Forbidden", "Navigation required."));
            }
        }
        let url =
            Url::parse(&format!("http://{}{}", self.authority, target)).map_err(|_| invalid)?;
        let mut keys = HashSet::new();
        let mut code = Zeroizing::new(String::new());
        for (key, value) in url.query_pairs() {
            if !keys.insert(key.to_string()) {
                return Err(invalid);
            }
            match key.as_ref() {
                "code" => *code = value.into_owned(),
                "error" if value == "access_denied" => {
                    return Err(Rejection(
                        "400 Bad Request",
                        "Discord authorization was denied.",
                    ));
                }
                "token" => {
                    return Err(Rejection(
                        "400 Bad Request",
                        "Legacy bearer callbacks are not supported; update the OAuth server.",
                    ));
                }
                _ => return Err(invalid),
            }
        }
        if code.len() != 43
            || !code
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err(invalid);
        }
        Ok(Some(code))
    }

    async fn exchange_code(&self, code: &str) -> Result<Auth> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()?;
        let mut response = client
            .post(self.exchange.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .json(&serde_json::json!({"code": code, "code_verifier": self.verifier.as_str()}))
            .send()
            .await
            .map_err(|_| {
                anyhow!("PKCE exchange failed; verify the server supports CLI 0.5 authentication")
            })?;
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "PKCE exchange was rejected by the server"
        );
        ensure!(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("application/json")),
            "invalid PKCE exchange response"
        );
        if let Some(length) = response.content_length() {
            ensure!(length <= 8192, "PKCE response is too large");
        }
        let mut body = Zeroizing::new(Vec::new());
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("PKCE response could not be read"))?
        {
            ensure!(
                chunk.len() <= 8192usize.saturating_sub(body.len()),
                "PKCE response is too large"
            );
            body.extend_from_slice(&chunk);
        }
        let auth: Auth =
            serde_json::from_slice(&body).map_err(|_| anyhow!("invalid PKCE credentials"))?;
        credentials::validate_auth(&auth)?;
        Ok(auth)
    }

    #[cfg(test)]
    fn address(&self) -> std::net::SocketAddr {
        self.listener.local_addr().unwrap()
    }
}

impl PendingLogin {
    pub async fn respond(mut self, success: bool) -> Result<()> {
        if success {
            send_page(
                &mut self.stream,
                "200 OK",
                "Login complete. Your credentials were saved securely. You may close this window.",
                Some(&self.nonce),
                self.write_timeout,
            )
            .await
        } else {
            send_page(
                &mut self.stream,
                "500 Internal Server Error",
                "Login was not completed. Return to Jorik for details.",
                Some(&self.nonce),
                self.write_timeout,
            )
            .await
        }
    }
}

async fn send_page(
    stream: &mut TcpStream,
    status: &str,
    message: &str,
    nonce: Option<&str>,
    limit: Duration,
) -> Result<()> {
    let script = nonce
        .map(|nonce| {
            format!("<script nonce=\"{nonce}\">history.replaceState(null,'','/complete');</script>")
        })
        .unwrap_or_default();
    let policy = nonce
        .map(|nonce| format!("script-src 'nonce-{nonce}'; "))
        .unwrap_or_default();
    // Only static application messages are passed here. No callback values,
    // avatars, third-party scripts, or secrets are embedded in the document.
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Jorik login</title></head><body><h1>Jorik</h1><p>{message}</p>{script}</body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nPragma: no-cache\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'none'; {policy}base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\n\r\n{body}",
        body.len()
    );
    timeout(limit, async {
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await
    })
    .await
    .map_err(|_| anyhow!("browser callback response timed out"))?
    .map_err(|_| anyhow!("could not send browser callback response"))
}

pub async fn revoke(client: &Client, base: &str, token: Option<&str>) -> Result<bool> {
    let Some(token) = token else {
        return Ok(false);
    };
    let base = credentials::server_identity(base)?;
    let response = client
        .post(api::build_url(&base, "/webhook/auth/revoke"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| anyhow!("remote token revocation failed; local signout is independent"))?;
    ensure!(
        response.status().is_success(),
        "remote revocation returned {}",
        response.status()
    );
    let body = api::response_text(response).await?;
    let data: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| anyhow!("invalid revocation response"))?;
    Ok(data
        .get("revoked")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false))
}

#[cfg(test)]
mod tests;
