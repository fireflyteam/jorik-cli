use super::tests::app;

#[test]
fn untrusted_metadata_and_logs_are_scrubbed_and_elapsed_time_saturates() {
    let mut app = app();
    app.token = Some("secret".into());
    app.parse_queue_response(&serde_json::json!({"current":{"title":"title\u{001b}]52;c;payload\u{0007}\u{202e} secret","author":"Artist"}, "upcoming":[]}));
    let track = app.current_track.as_ref().unwrap();
    assert!(
        !track.contains("secret") && !track.contains('\u{001b}') && !track.contains('\u{202e}')
    );
    app.log(format!("secret\n{}", "x".repeat(10_000)));
    let log = app.debug_logs.last().unwrap();
    assert!(!log.contains("secret") && !log.contains('\n'));
    assert!(log.len() < 2100);
    app.paused = false;
    app.elapsed_ms = u64::MAX;
    app.duration_ms = 0;
    app.update_realtime();
    assert_eq!(app.elapsed_ms, u64::MAX);
    app.pending_commands
        .insert("cmd-test".into(), Instant::now());
    let event: WsEvent = serde_json::from_value(serde_json::json!({"type":"action_response","id":"cmd-test","success":false,"error":{"token":"another-secret","message":"secret"}})).unwrap();
    app.finish_ws_action(&event);
    let error = app.error_message.as_ref().unwrap();
    assert!(!error.contains("another-secret") && !error.contains("secret"));
}

#[tokio::test]
async fn unauthenticated_player_operations_do_not_make_network_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut app = app();
    app.base_url = format!("http://{}", listener.local_addr().unwrap());
    app.token = None;
    let shared = Arc::new(Mutex::new(app));
    async_fetch_queue(shared.clone()).await;
    async_fetch_lyrics(shared.clone()).await;
    async_simple_command(
        shared.clone(),
        "/webhook/audio".to_string(),
        serde_json::json!({"action": "skip"}),
    )
    .await;
    assert!(
        timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    assert!(
        shared
            .lock()
            .await
            .error_message
            .as_deref()
            .unwrap()
            .contains("Log in")
    );
}

use super::*;

#[test]
fn websocket_credentials_are_header_only_sensitive_and_keep_the_api_prefix() {
    let request = websocket_request("https://example.com:8443/api", "private-token").unwrap();
    assert_eq!(request.uri().to_string(), "wss://example.com:8443/api/ws");
    assert!(!request.uri().to_string().contains("private-token"));
    assert_eq!(request.headers()["Origin"], "https://example.com:8443");
    assert_eq!(request.headers()["Authorization"], "Bearer private-token");
    assert!(request.headers()["Authorization"].is_sensitive());
    assert!(!format!("{request:?}").contains("private-token"));
    assert!(websocket_request("http://external.example", "token").is_err());
    assert!(websocket_request("https://example.com", "invalid\r\nheader").is_err());
    assert_eq!(
        websocket_url("http://127.0.0.1:8080").unwrap().as_str(),
        "ws://127.0.0.1:8080/ws"
    );
}

#[test]
fn cancellation_and_quit_invalidate_pending_login_generations() {
    for code in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('3')] {
        let mut app = app();
        let cancel = CancellationToken::new();
        app.login_cancel = Some(cancel.clone());
        app.login_generation = 7;
        app.view = View::AuthResult;
        app.is_loading = true;
        let shared = Arc::new(Mutex::new(self::app()));
        handle_key_event(
            &mut app,
            event::KeyEvent::new(code, event::KeyModifiers::NONE),
            shared,
        );
        assert!(cancel.is_cancelled());
        assert!(app.login_cancel.is_none());
        assert_eq!(app.login_generation, 8);
        assert!(!app.is_loading);
        assert_eq!(app.token.as_deref(), Some("secret-token")); // Cancelling re-login preserves the current session.
    }
}

#[test]
fn repeated_login_attempts_and_login_during_signout_do_not_spawn_work() {
    let mut app = app();
    let shared = Arc::new(Mutex::new(self::app()));
    app.login_generation = 4;
    app.login_cancel = Some(CancellationToken::new());
    start_login(&mut app, shared.clone());
    assert_eq!(app.login_generation, 4);
    app.login_cancel = None;
    app.signing_out = true;
    start_login(&mut app, shared);
    assert_eq!(app.login_generation, 4);
    assert!(app.login_cancel.is_none());
}

#[test]
fn clearing_a_session_discards_tokens_cached_tracks_and_pending_actions() {
    let mut app = app();
    app.queue = vec!["Private track".to_string()];
    app.current_track = Some("Private current track".to_string());
    app.spectrogram = Some(vec![vec![50]]);
    app.ws_connected = true;
    app.pending_commands
        .insert("pending".to_string(), Instant::now());
    app.error_message = Some("Private old-server error".to_string());
    app.auth_info_text = Some("Private profile".to_string());
    app.debug_logs.push("Old server events".to_string());
    app.clear_session();
    assert!(app.token.is_none());
    assert!(app.guild_id.is_none() && app.user_id.is_none());
    assert!(app.queue.is_empty() && app.current_track.is_none());
    assert!(app.pending_commands.is_empty());
    assert!(app.error_message.is_none() && app.auth_info_text.is_none());
    assert!(!app.ws_connected && app.needs_reconnect);
    assert!(app.debug_logs.is_empty());
}
