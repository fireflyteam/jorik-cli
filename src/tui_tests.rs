use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

pub(super) fn app() -> App {
    App::new(
        Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
        api::Settings {
            base_url: "https://example.com".to_string(),
            visualizer_offset: 0,
            theme: "Default".to_string(),
            visualizer_style: "Bars".to_string(),
            layout: "Standard".to_string(),
        },
        Some("secret-token".to_string()),
        Some("guild".to_string()),
        None,
    )
}

#[test]
fn connection_urls_and_logs_do_not_reveal_credentials() {
    let url =
        Url::parse("wss://user:password@example.com:8443/ws?token=secret-token#secret").unwrap();
    assert_eq!(redacted_url(&url), "wss://example.com:8443/ws");
    let mut app = app();
    app.log("Error with secret-token");
    assert!(!app.debug_logs[0].contains("secret-token"));
    assert!(app.debug_logs[0].contains("[redacted]"));
}

#[test]
fn pending_commands_wait_for_their_own_acknowledgement() {
    let mut app = app();
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    app.ws_sender = Some(tx);
    app.ws_connected = true;
    assert!(app.send_ws_action(&json!({"action": "skip"})));
    assert!(app.is_busy());
    let Message::Text(message) = rx.try_recv().unwrap() else {
        panic!("expected text");
    };
    let message: Value = serde_json::from_str(&message).unwrap();

    let unrelated: WsEvent = serde_json::from_value(json!({
        "type": "action_response", "id": "other", "success": true
    }))
    .unwrap();
    app.finish_ws_action(&unrelated);
    assert!(app.is_busy());

    let failed: WsEvent = serde_json::from_value(json!({
        "type": "action_response", "id": message["id"],
        "success": false, "message": "Not allowed"
    }))
    .unwrap();
    app.finish_ws_action(&failed);
    assert!(!app.is_busy());
    assert!(
        app.error_message
            .as_deref()
            .unwrap()
            .contains("Not allowed")
    );
}

#[test]
fn successful_commands_clear_pending_state() {
    let mut app = app();
    app.pending_commands
        .insert("cmd-1".to_string(), Instant::now());
    let event = serde_json::from_value(json!({
        "type": "action_response", "id": "cmd-1", "success": true
    }))
    .unwrap();
    app.finish_ws_action(&event);
    assert!(!app.is_busy());
    assert!(app.error_message.is_none());
}

#[test]
fn full_websocket_channel_falls_back_without_marking_an_action_pending() {
    let mut app = app();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    tx.try_send(Message::Text("occupied".into())).unwrap();
    app.ws_sender = Some(tx);
    app.ws_connected = true;
    assert!(!app.send_ws_action(&json!({"action": "skip"})));
    assert!(app.pending_commands.is_empty());
}

#[test]
fn timeouts_and_disconnects_report_uncertain_command_outcomes() {
    let mut app = app();
    app.pending_commands.insert(
        "expired".to_string(),
        Instant::now() - Duration::from_secs(11),
    );
    app.update_realtime();
    assert!(app.pending_commands.is_empty());
    assert!(app.error_message.as_deref().unwrap().contains("timed out"));

    app.pending_commands
        .insert("pending".to_string(), Instant::now());
    app.ws_connected = true;
    app.disconnect_ws();
    assert!(app.pending_commands.is_empty());
    assert!(!app.ws_connected);
    assert!(
        app.error_message
            .as_deref()
            .unwrap()
            .contains("Connection lost")
    );
}

#[test]
fn escape_discards_text_settings_without_saving() {
    let mut app = app();
    app.view = View::Settings;
    app.is_settings_editing = true;
    app.settings_input = "https://changed.example".to_string();
    app.offset_input = "1234".to_string();
    let unused = Arc::new(Mutex::new(self::app()));
    handle_settings_keys(
        &mut app,
        event::KeyEvent::new(KeyCode::Esc, event::KeyModifiers::NONE),
        unused,
    );
    assert!(!app.is_settings_editing);
    assert_eq!(app.settings_input, app.base_url);
    assert_eq!(app.offset_input, "0");
    assert_eq!(app.visualizer_offset, 0);
    assert!(!app.needs_reconnect);
}

#[test]
fn quitting_works_from_fatal_errors_without_intercepting_text_input() {
    let mut app = app();
    let quit = event::KeyEvent::new(KeyCode::Char('q'), event::KeyModifiers::NONE);
    let interrupt = event::KeyEvent::new(KeyCode::Char('c'), event::KeyModifiers::CONTROL);
    assert!(should_quit(&app, quit));
    app.input_mode = InputMode::Editing;
    assert!(!should_quit(&app, quit));
    assert!(should_quit(&app, interrupt));
    app.view = View::Settings;
    app.is_settings_editing = true;
    app.fatal_error = Some("Join a voice channel".to_string());
    assert!(should_quit(&app, quit));
    assert!(should_quit(&app, interrupt));
}

#[test]
fn invalid_settings_do_not_commit() {
    let mut app = app();
    app.settings_input = "javascript:alert(1)".to_string();
    assert!(draft_settings(&app).is_err());
    app.settings_input = app.base_url.clone();
    app.offset_input = "-".to_string();
    assert!(draft_settings(&app).is_err());
    assert!(!save_app_settings(&mut app));
    assert!(
        app.error_message
            .as_deref()
            .unwrap()
            .contains("valid integer")
    );
    assert_eq!(app.visualizer_offset, 0);
}

#[test]
fn input_tail_uses_display_cells() {
    assert_eq!(input_tail("hello", 3), "llo");
    assert_eq!(input_tail("音楽", 2), "楽");
    assert_eq!(input_tail("aé", 1), "é");
    assert_eq!(input_tail("e\u{301}", 1), "e\u{301}");
    assert_eq!(input_tail("anything", 0), "");
    assert_eq!(input_tail("", 10), "");
}

#[test]
fn every_visualizer_and_layout_handles_small_terminals() {
    let mut app = app();
    app.current_track = Some("Title - Artist".to_string());
    app.duration_ms = 60_000;
    app.spectrogram = Some(vec![vec![100; 64]]);
    app.focus = PlayerFocus::Playback;
    for width in [1, 2, 10, 40, 63, 64, 80, 120] {
        for height in [1, 2, 8, 24, 32, 40] {
            for layout in ["Standard", "Sidebar", "Studio", "Zen"] {
                app.layout = layout.to_string();
                for style in ["Bars", "Blocky", "Line", "Wave", "Dots"] {
                    app.viz_style = style.to_string();
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    terminal.draw(|frame| ui(frame, &mut app)).unwrap();
                }
            }
        }
    }
}

#[test]
fn unicode_input_cursor_stays_inside_the_terminal() {
    for (width, height) in [(1, 1), (10, 8), (40, 12), (80, 24)] {
        let mut app = app();
        app.input = "音楽é".repeat(50);
        app.input_mode = InputMode::Editing;
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui(frame, &mut app)).unwrap();
        let position = terminal.get_cursor_position().unwrap();
        assert!(position.x < width && position.y < height);

        app.input_mode = InputMode::Normal;
        app.view = View::Settings;
        app.is_settings_editing = true;
        app.settings_input = format!("https://example.com/{}", "音楽é".repeat(50));
        terminal.draw(|frame| ui(frame, &mut app)).unwrap();
        let position = terminal.get_cursor_position().unwrap();
        assert!(position.x < width && position.y < height);
    }
}

#[test]
fn errors_do_not_replace_the_cached_queue() {
    let mut app = app();
    app.queue = vec!["Cached track".to_string()];
    app.error_message = Some("Command rejected".to_string());
    let mut terminal = Terminal::new(TestBackend::new(80, 40)).unwrap();
    terminal.draw(|frame| ui(frame, &mut app)).unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Cached track"));
    assert!(screen.contains("Command rejected"));
}

async fn mock_response(status: &str, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 8192];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    url
}

#[tokio::test]
async fn http_command_failures_are_visible() {
    let mut app = app();
    app.base_url = mock_response("403 Forbidden", r#"{"message":"Not allowed"}"#).await;
    let app = Arc::new(Mutex::new(app));
    async_simple_command(
        app.clone(),
        "/webhook/audio".to_string(),
        json!({"action": "skip"}),
    )
    .await;
    let app = app.lock().await;
    assert!(!app.is_loading);
    assert!(app.error_message.as_deref().unwrap().contains("403"));
    assert!(
        app.error_message
            .as_deref()
            .unwrap()
            .contains("Not allowed")
    );
}

#[tokio::test]
async fn malformed_queue_responses_are_visible_without_losing_cached_tracks() {
    let mut app = app();
    app.base_url = mock_response("200 OK", "not json").await;
    app.queue = vec!["Cached track".to_string()];
    let app = Arc::new(Mutex::new(app));
    async_fetch_queue(app.clone()).await;
    let app = app.lock().await;
    assert!(!app.is_loading);
    assert!(
        app.queue_error
            .as_deref()
            .unwrap()
            .contains("Invalid server response")
    );
    assert_eq!(app.queue, ["Cached track"]);
}

#[tokio::test]
async fn lyrics_use_only_rest_and_preserve_the_selected_view() {
    let mut app = app();
    app.base_url = mock_response("200 OK", r#"{"data":{"text":"Lyrics text"}}"#).await;
    app.view = View::Debug;
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    app.ws_sender = Some(tx);
    app.ws_connected = true;
    let app = Arc::new(Mutex::new(app));
    async_fetch_lyrics(app.clone()).await;
    let app = app.lock().await;
    assert!(app.view == View::Debug);
    assert_eq!(app.lyrics_text.as_deref(), Some("Lyrics text"));
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn slow_response_body_does_not_lock_the_ui() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut app = app();
    app.base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = Arc::new(Mutex::new(app));
    let (headers_tx, headers_rx) = tokio::sync::oneshot::channel();
    let (body_tx, body_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 8192];
        assert!(stream.read(&mut request).await.unwrap() > 0);
        let body = r#"{"queue":{"upcoming":[]}}"#;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        headers_tx.send(()).unwrap();
        body_rx.await.unwrap();
        stream.write_all(body.as_bytes()).await.unwrap();
    });
    let worker = tokio::spawn(async_fetch_queue(app.clone()));
    headers_rx.await.unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let guard = timeout(Duration::from_millis(250), app.lock())
        .await
        .expect("network body read must not hold the UI mutex");
    drop(guard);
    body_tx.send(()).unwrap();
    worker.await.unwrap();
    server.await.unwrap();
}
