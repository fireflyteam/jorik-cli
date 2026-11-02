use super::*;

#[tokio::test]
async fn decoded_response_limits_apply_with_and_without_content_length() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for chunked in [false, true] {
        for length in [8, MAX_RESPONSE_BYTES, MAX_RESPONSE_BYTES + 1] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).await;
                let body = "x".repeat(length);
                let response = if chunked {
                    format!(
                        "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{length:x}\r\n{body}\r\n0\r\n\r\n"
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}"
                    )
                };
                let _ = stream.write_all(response.as_bytes()).await;
            });
            let response = reqwest::Client::new()
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap();
            let result = response_text(response).await;
            if length > MAX_RESPONSE_BYTES {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap().len(), length);
            }
        }
    }
}

#[test]
fn retired_saved_default_moves_to_the_new_api_without_changing_preferences() {
    for base_url in ["https://jorik.xserv.pp.ua", "https://jorik.xserv.pp.ua/"] {
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "base_url": base_url, "theme": "Ocean", "visualizer_offset": 123
        }))
        .unwrap();
        let migrated = migrate_saved_settings(settings);
        assert_eq!(migrated.base_url, DEFAULT_BASE_URL);
        assert_eq!(migrated.theme, "Ocean");
        assert_eq!(migrated.visualizer_offset, 123);
    }
}

#[test]
fn custom_and_current_saved_hosts_remain_unchanged() {
    for base_url in [
        DEFAULT_BASE_URL,
        "https://custom.example/api",
        "http://127.0.0.1:8080",
    ] {
        let settings: Settings =
            serde_json::from_value(serde_json::json!({"base_url": base_url})).unwrap();
        assert_eq!(migrate_saved_settings(settings).base_url, base_url);
    }
}

#[test]
fn server_urls_are_validated() {
    for url in [
        "https://example.com",
        "http://127.0.0.1:8080",
        "https://example.com/api/",
    ] {
        assert!(validate_base_url(url).is_ok(), "{url}");
    }
    for url in [
        "not a url",
        "file:///tmp/server",
        "ws://example.com",
        "https://user:secret@example.com",
        "https://example.com?token=secret",
        "https://example.com#fragment",
    ] {
        assert!(validate_base_url(url).is_err(), "{url}");
    }
}
