use super::*;

#[tokio::test]
async fn denied_and_legacy_callbacks_end_login_without_leaking_credentials() {
    for query in ["error=access_denied", "token=old-bearer-secret"] {
        let session = session().await;
        let address = session.address();
        let request = request(&session, query);
        let cancel = CancellationToken::new();
        let worker = tokio::spawn(async move { session.wait(&cancel).await.is_err() });
        let page = exchange(address, &request).await;
        assert!(page.starts_with("HTTP/1.1 400"));
        assert!(page.contains("history.replaceState"));
        assert!(!page.contains("old-bearer-secret"));
        assert!(worker.await.unwrap());
    }
}

const CODE: &str = "GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG";

async fn exchange_server(body: &str, status: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut chunk = [0; 2048];
            let n = stream.read(&mut chunk).await.unwrap();
            if n == 0 {
                return;
            }
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                assert!(headers.starts_with("POST /oauth/cli/exchange "));
                assert!(!headers.to_lowercase().contains("authorization:"));
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    let data: serde_json::Value =
                        serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                    assert_eq!(data["code"].as_str().unwrap().len(), 43);
                    assert_eq!(data["code_verifier"].as_str().unwrap().len(), 43);
                    stream.write_all(response.as_bytes()).await.unwrap();
                    return;
                }
            }
        }
    });
    base
}

async fn session() -> LoginSession {
    let server = exchange_server(
        r#"{"token":"secret-token","username":"Safe","avatar_url":null}"#,
        "200 OK",
    )
    .await;
    LoginSession::with_limits(
        &server,
        Limits {
            total: Duration::from_secs(5),
            read: Duration::from_millis(100),
            write: Duration::from_secs(1),
        },
    )
    .await
    .unwrap()
}

fn request(session: &LoginSession, query: &str) -> Vec<u8> {
    format!(
        "GET {}?{query} HTTP/1.1\r\nHost: {}\r\n\r\n",
        session.path, session.authority
    )
    .into_bytes()
}

async fn exchange(address: std::net::SocketAddr, bytes: &[u8]) -> String {
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(bytes).await.unwrap();
    // Stop at Content-Length, as a browser does. On macOS closing a rejected
    // oversized request with unread bytes can reset the socket after the reply.
    timeout(Duration::from_secs(2), async {
        let mut response = Vec::new();
        loop {
            let mut buffer = [0; 2048];
            let count = stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0, "truncated callback response");
            response.extend_from_slice(&buffer[..count]);
            assert!(response.len() < 32768);
            if let Some(end) = response.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&response[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .map(|value| value.parse().unwrap())
                    })
                    .unwrap();
                if response.len() >= end + 4 + length {
                    return String::from_utf8(response).unwrap();
                }
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn callback_is_loopback_nonce_bound_and_pkce_verifier_is_never_in_the_authorize_url() {
    let a = LoginSession::bind("https://jorik.gleckson.com")
        .await
        .unwrap();
    let b = LoginSession::bind("https://jorik.gleckson.com")
        .await
        .unwrap();
    assert_eq!(a.address().ip(), std::net::Ipv4Addr::LOCALHOST);
    assert_ne!(a.path, b.path);
    let authorize = Url::parse(a.authorization_url()).unwrap();
    assert_eq!(
        authorize.origin().ascii_serialization(),
        "https://jorik.gleckson.com"
    );
    let values = authorize
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        values["callback"],
        format!("http://{}{}", a.authority, a.path)
    );
    assert_eq!(values["code_challenge_method"], "S256");
    assert_eq!(
        values["code_challenge"],
        URL_SAFE_NO_PAD.encode(Sha256::digest(a.verifier.as_bytes()))
    );
    assert!(!a.authorization_url().contains(a.verifier.as_str()));
    assert!(!a.path.contains(a.verifier.as_str()));
    assert!(
        b.parse_request(&request(&a, &format!("code={CODE}")))
            .is_err()
    );
}

#[tokio::test]
async fn parser_rejects_wrong_paths_hosts_methods_origins_duplicates_and_bearer_callbacks() {
    let session = session().await;
    let normal = String::from_utf8(request(&session, &format!("code={CODE}"))).unwrap();
    for bytes in [
        normal.replace(&session.path, "/oauth-callback/incorrect"),
        normal.replace(
            &format!("Host: {}", session.authority),
            "Host: evil.example",
        ),
        normal.replace("GET ", "POST "),
        normal.replace("GET /", "GET http://evil.example/"),
        normal.replace("\r\n\r\n", "\r\nOrigin: https://evil.example\r\n\r\n"),
        normal.replace("\r\n\r\n", "\r\nHost: duplicate\r\n\r\n"),
        normal.replace("\r\n\r\n", "\r\nContent-Length: 1\r\n\r\nx"),
        normal.replace("\r\n\r\n", "\r\nTransfer-Encoding: chunked\r\n\r\n"),
        normal.replace("\r\n\r\n", "\r\nSec-Fetch-Mode: cors\r\n\r\n"),
        normal.replace(
            &format!("code={CODE}"),
            &format!("code={CODE}&%63ode=second"),
        ),
        normal.replace(&format!("code={CODE}"), "code="),
        normal.replace(&format!("code={CODE}"), "token=secret-token"),
    ] {
        let rejection = session.parse_request(bytes.as_bytes()).unwrap_err();
        assert!(!format!("{rejection:?}").contains("secret-token"));
    }
    assert_eq!(
        session
            .parse_request(&request(&session, &format!("code={CODE}")))
            .unwrap()
            .unwrap()
            .as_str(),
        CODE
    );
    assert!(
        session
            .parse_request(&normal.as_bytes()[..10])
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn invalid_requests_do_not_consume_login_and_success_never_contains_a_bearer() {
    let session = session().await;
    let address = session.address();
    let valid = request(&session, &format!("code={CODE}"));
    let invalid = request(&session, "code=one&code=two");
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(async move {
        let pending = session.wait(&cancel).await.unwrap();
        let auth = pending.auth.clone();
        pending.respond(true).await.unwrap();
        auth
    });
    assert!(
        exchange(address, &invalid)
            .await
            .starts_with("HTTP/1.1 400")
    );
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(&valid[..20]).await.unwrap();
    tokio::task::yield_now().await;
    stream.write_all(&valid[20..]).await.unwrap();
    let mut page = String::new();
    stream.read_to_string(&mut page).await.unwrap();
    assert!(page.starts_with("HTTP/1.1 200"));
    for header in [
        "Cache-Control: no-store",
        "Referrer-Policy: no-referrer",
        "Content-Security-Policy: default-src 'none'",
        "X-Frame-Options: DENY",
    ] {
        assert!(page.contains(header));
    }
    assert!(page.contains("history.replaceState"));
    assert!(!page.contains("secret-token") && !page.contains(CODE) && !page.contains("src="));
    assert_eq!(worker.await.unwrap().token, "secret-token");
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn cancellation_closes_the_listener_without_accepting_a_code() {
    let session = session().await;
    let address = session.address();
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let worker = tokio::spawn(async move { session.wait(&worker_cancel).await.is_err() });
    cancel.cancel();
    assert!(
        timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
    );
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn overall_deadline_includes_slow_request_reads() {
    let session = LoginSession::with_limits(
        "http://127.0.0.1:8080",
        Limits {
            total: Duration::from_millis(50),
            read: Duration::from_secs(5),
            write: Duration::from_secs(1),
        },
    )
    .await
    .unwrap();
    let address = session.address();
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(async move { session.wait(&cancel).await.is_err() });
    let mut slow = TcpStream::connect(address).await.unwrap();
    slow.write_all(b"GET /incomplete").await.unwrap();
    assert!(
        timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
    );
}

#[tokio::test]
async fn stalled_and_oversized_requests_do_not_consume_the_session() {
    let session = session().await;
    let address = session.address();
    let valid = request(&session, &format!("code={CODE}"));
    let oversized = format!(
        "GET {}?code={CODE} HTTP/1.1\r\nHost: {}\r\nX-Padding: {}\r\n\r\n",
        session.path,
        session.authority,
        "x".repeat(MAX_REQUEST + 1)
    );
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(async move {
        session
            .wait(&cancel)
            .await
            .unwrap()
            .respond(true)
            .await
            .unwrap()
    });
    let mut slow = TcpStream::connect(address).await.unwrap();
    slow.write_all(b"GET /incomplete").await.unwrap();
    let mut page = String::new();
    timeout(Duration::from_secs(2), slow.read_to_string(&mut page))
        .await
        .unwrap()
        .unwrap();
    assert!(page.starts_with("HTTP/1.1 408"));
    assert!(
        exchange(address, oversized.as_bytes())
            .await
            .starts_with("HTTP/1.1 431")
    );
    assert!(exchange(address, &valid).await.starts_with("HTTP/1.1 200"));
    worker.await.unwrap();
}

#[tokio::test]
async fn exchange_rejects_invalid_or_oversized_credentials_without_reflecting_them() {
    for body in [
        r#"{"token":"bad secret","username":null,"avatar_url":null}"#.to_string(),
        "secret".repeat(2000),
    ] {
        let base = exchange_server(&body, "200 OK").await;
        let session = LoginSession::bind(&base).await.unwrap();
        let address = session.address();
        let request = request(&session, &format!("code={CODE}"));
        let cancel = CancellationToken::new();
        let worker = tokio::spawn(async move { session.wait(&cancel).await.is_err() });
        let page = exchange(address, &request).await;
        assert!(page.starts_with("HTTP/1.1 502"));
        assert!(!page.contains("bad secret"));
        assert!(worker.await.unwrap());
    }
}

#[tokio::test]
#[ignore = "requires Bun and JORIK_OAUTH_FIXTURE pointing to the paired backend's test-only fixture"]
async fn paired_backend_pkce_round_trip() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let fixture =
        std::env::var("JORIK_OAUTH_FIXTURE").expect("set the absolute backend fixture path");
    let mut child = tokio::process::Command::new("bun")
        .arg(fixture)
        .env_remove("DISCORD_TOKEN")
        .env_remove("OAUTH_CLIENT_SECRET")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut base = String::new();
    timeout(Duration::from_secs(5), reader.read_line(&mut base))
        .await
        .unwrap()
        .unwrap();
    let session = LoginSession::bind(base.trim()).await.unwrap();
    let authorize = session.authorization_url().to_string();
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(async move {
        let pending = session.wait(&cancel).await.unwrap();
        let auth = pending.auth.clone();
        pending.respond(true).await.unwrap();
        auth
    });
    let browser = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let redirect = browser.get(authorize).send().await.unwrap();
    assert_eq!(redirect.status().as_u16(), 303);
    let location = redirect.headers()["location"].to_str().unwrap();
    assert!(!location.contains("token="));
    let page = browser
        .get(location)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!page.contains("test-pkce-token"));
    assert_eq!(worker.await.unwrap().token, "test-pkce-token");
    child.kill().await.unwrap();
}
