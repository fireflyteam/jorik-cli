use super::tests::app;
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use tokio::sync::oneshot;

fn press(app: &mut App, code: KeyCode) -> UiOutcome {
    handle_key_event(
        app,
        event::KeyEvent::new(code, event::KeyModifiers::NONE),
        Arc::new(Mutex::new(self::app())),
    )
}

fn draw(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui(frame, app)).unwrap();
    terminal
}

fn text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn page(offset: usize, count: usize, total: usize) -> Value {
    json!({"offset":offset,"total_upcoming":total,"upcoming":(offset..offset+count)
        .map(|i| json!({"id":i.to_string(),"title":format!("Track {i}")})).collect::<Vec<_>>()})
}

async fn receive<T>(rx: oneshot::Receiver<T>) -> T {
    timeout(Duration::from_secs(3), rx).await.unwrap().unwrap()
}

// A local response gate: tests can inspect the complete request while the server stalls.
async fn held_http() -> (
    String,
    oneshot::Receiver<Value>,
    oneshot::Sender<String>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (seen_tx, seen_rx) = oneshot::channel();
    let (reply_tx, reply_rx) = oneshot::channel::<String>();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let body_start = loop {
            let mut chunk = [0; 4096];
            let count = socket.read(&mut chunk).await.unwrap();
            assert!(count > 0 && bytes.len() < 65536);
            bytes.extend_from_slice(&chunk[..count]);
            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break end + 4;
                }
            }
        };
        seen_tx
            .send(serde_json::from_slice(&bytes[body_start..]).unwrap())
            .unwrap();
        let response = reply_rx.await.unwrap();
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    (url, seen_rx, reply_tx, task)
}

fn response(status: &str, body: Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn wait_for(shared: &Arc<Mutex<App>>, predicate: impl Fn(&App) -> bool) {
    let redraw = shared.lock().await.redraw.clone();
    timeout(Duration::from_secs(3), async {
        loop {
            let notification = redraw.notified();
            if predicate(&*shared.lock().await) {
                break;
            }
            notification.await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn queue_requests_coalesce_and_only_the_latest_page_owns_completion() {
    let mut app = app();
    let old = app.begin_queue_request().unwrap();
    assert!(app.begin_queue_request().is_none());
    app.queue_target_offset = 20;
    let new = app.begin_queue_request().unwrap();
    assert!(!app.finish_queue_request(&old, Ok(page(0, 20, 60))));
    assert!(app.queue.is_empty() && app.is_busy());
    assert!(!app.finish_queue_request(&new, Ok(page(20, 20, 60))));
    assert_eq!(app.queue[0].title, "Track 20");
    assert_eq!(app.queue_offset, 20);
    assert_eq!(app.queue_state.selected(), Some(0));
    assert!(!app.is_busy());
}

#[test]
fn websocket_queue_and_playback_snapshots_win_over_older_rest_reads() {
    let mut app = super::polish_tests::playing();
    let old = app.begin_queue_request().unwrap();
    app.parse_ws_state(&page(0, 2, 2));
    app.finish_queue_request(&old, Ok(page(0, 20, 20)));
    assert_eq!(app.queue.len(), 2);
    assert!(app.queue_error.is_none());

    let old = app.begin_queue_request().unwrap();
    app.apply_playback(PlaybackState {
        elapsed_ms: 9000,
        duration_ms: 243_000,
        paused: false,
        spectrogram: None,
    });
    let mut stale = page(0, 3, 3);
    stale["current"] = Value::Null;
    stale["playback"] = json!({"elapsedMs":1000,"durationMs":243000,"paused":true});
    app.finish_queue_request(&old, Ok(stale));
    assert_eq!(app.elapsed_ms, 9000);
    assert!(app.has_playback_state && !app.paused && app.current_track.is_some());
    assert_eq!(app.queue.len(), 3);
}

#[test]
fn successful_mutations_invalidate_reads_started_before_the_command() {
    let mut app = app();
    let old = app.begin_queue_request().unwrap();
    app.finish_command(CommandKind::Other, Ok(()));
    let new = app.begin_queue_request().unwrap();
    app.finish_queue_request(&old, Ok(page(0, 20, 20)));
    assert!(app.queue.is_empty() && app.is_busy());
    app.finish_queue_request(&new, Ok(page(0, 1, 1)));
    assert_eq!(app.queue.len(), 1);
    assert!(!app.is_busy());
}

#[test]
fn same_credentials_after_a_session_switch_cannot_accept_old_work() {
    let mut app = app();
    let session = Session::capture(&app);
    let old = app.begin_queue_request().unwrap();
    app.clear_session();
    app.token = Some("secret-token".into());
    app.guild_id = Some("guild".into());
    assert!(!session.matches(&app));
    let new = app.begin_queue_request().unwrap();
    app.finish_queue_request(&old, Ok(page(0, 20, 20)));
    assert!(app.queue.is_empty() && app.owns_queue_request(&new));
}

#[test]
fn guild_discovery_keeps_request_ownership_but_cannot_switch_a_selected_guild() {
    let mut app = app();
    app.guild_id = None;
    let session = Session::capture(&app);
    app.parse_queue_response(&json!({"guildId":"guild","upcoming":[]}));
    assert!(session.matches(&app));
    app.parse_queue_response(&json!({"guildId":"other","upcoming":[{"title":"Wrong guild"}]}));
    assert!(app.queue.is_empty());
    assert_eq!(app.guild_id.as_deref(), Some("guild"));
}

#[test]
fn busy_flags_belong_to_each_operation_not_the_last_completion() {
    let mut app = app();
    let queue = app.begin_queue_request().unwrap();
    app.lyrics_loading = true;
    app.auth_loading = true;
    app.rest_commands.insert(100, CommandKind::Other);
    app.finish_queue_request(&queue, Ok(page(0, 0, 0)));
    assert!(app.is_busy());
    app.auth_loading = false;
    app.lyrics_loading = false;
    assert!(app.is_busy());
    app.rest_commands.remove(&100);
    assert!(!app.is_busy());
}

#[tokio::test]
async fn repeated_refreshes_share_one_http_request_and_leave_the_ui_unlocked() {
    let (url, seen, reply, server) = held_http().await;
    let mut app = app();
    app.base_url = url;
    let shared = Arc::new(Mutex::new(app));
    let first = tokio::spawn(async_fetch_queue(shared.clone()));
    let payload = receive(seen).await;
    assert_eq!(payload["limit"], 20);
    timeout(
        Duration::from_millis(250),
        async_fetch_queue(shared.clone()),
    )
    .await
    .unwrap();
    assert!(shared.lock().await.is_busy());
    reply.send(response("200 OK", page(0, 2, 2))).unwrap();
    first.await.unwrap();
    server.await.unwrap();
    assert_eq!(shared.lock().await.queue.len(), 2);
}

#[test]
fn voice_discovery_failure_preserves_navigation_and_does_not_intercept_text() {
    let mut app = app();
    app.queue = vec!["Cached".into()];
    let request = app.begin_queue_request().unwrap();
    app.finish_queue_request(
        &request,
        Err("user_not_in_voice_channel_or_guild_unknown".into()),
    );
    assert_eq!(app.queue[0].title, "Cached");
    assert!(
        app.queue_error
            .as_deref()
            .unwrap()
            .contains("Join a voice channel")
    );
    assert!(text(&draw(&mut app, 80, 24)).contains("r retry"));
    for modifier in [event::KeyModifiers::CONTROL, event::KeyModifiers::ALT] {
        for c in ['s', 'w', 'l', 'q', '3'] {
            assert!(matches!(
                handle_key_event(
                    &mut app,
                    event::KeyEvent::new(KeyCode::Char(c), modifier),
                    Arc::new(Mutex::new(self::app()))
                ),
                UiOutcome::Continue
            ));
            assert!(app.view == View::Main && app.pending_commands.is_empty());
        }
    }
    press(&mut app, KeyCode::Char('3'));
    assert!(app.view == View::Settings);
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('?'));
    assert!(app.view == View::Help);
    press(&mut app, KeyCode::Esc);
    execute_action(Action::Auth, &mut app, Arc::new(Mutex::new(self::app())));
    assert!(app.view == View::AuthMenu);
    press(&mut app, KeyCode::Esc);
    app.begin_search("");
    assert!(matches!(
        press(&mut app, KeyCode::Char('q')),
        UiOutcome::Continue
    ));
    assert_eq!(app.input, "q");
}

#[tokio::test]
async fn rejected_add_keeps_the_original_query_and_repeated_enter_does_not_submit_twice() {
    let (url, seen, reply, server) = held_http().await;
    let mut app = app();
    app.base_url = url;
    app.begin_search("  日本語 - a song  ");
    let shared = Arc::new(Mutex::new(app));
    {
        let mut app = shared.lock().await;
        handle_key_event(
            &mut app,
            event::KeyEvent::new(KeyCode::Enter, event::KeyModifiers::NONE),
            shared.clone(),
        );
    }
    let request = receive(seen).await;
    assert_eq!(request["query"], "日本語 - a song");
    {
        let mut app = shared.lock().await;
        assert_eq!(app.input, "  日本語 - a song  ");
        assert!(app.input_mode == InputMode::Editing);
        handle_key_event(
            &mut app,
            event::KeyEvent::new(KeyCode::Enter, event::KeyModifiers::NONE),
            shared.clone(),
        );
        assert_eq!(app.rest_commands.len(), 1);
    }
    reply
        .send(response(
            "403 Forbidden",
            json!({"message":"Cannot add secret-token"}),
        ))
        .unwrap();
    server.await.unwrap();
    wait_for(&shared, |app| app.add_pending.is_none()).await;
    let app = shared.lock().await;
    assert_eq!(app.input, "  日本語 - a song  ");
    assert!(app.input_mode == InputMode::Editing && !app.is_busy());
    assert!(app.input_error.as_deref().unwrap().contains("403"));
    assert!(!app.input_error.as_deref().unwrap().contains("secret-token"));
}

#[test]
fn unconfirmed_add_retains_a_recoverable_draft_without_replaying_the_request() {
    let mut app = app();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    app.ws_sender = Some(tx);
    app.ws_connected = true;
    app.begin_search("Original query");
    app.add_pending = Some(app.input.clone());
    assert!(app.send_ws_command(
        &json!({"action":"play"}),
        CommandKind::Add(app.input.clone())
    ));
    rx.try_recv().unwrap();
    press(&mut app, KeyCode::Esc);
    app.pending_commands
        .values_mut()
        .for_each(|pending| pending.started = Instant::now() - Duration::from_secs(11));
    app.update_realtime();
    assert!(rx.try_recv().is_err());
    assert!(
        app.error_message
            .as_deref()
            .unwrap()
            .contains("Refresh before retrying")
    );
    assert!(app.add_pending.is_none());
    app.begin_search("");
    assert_eq!(app.input, "Original query");
}

#[test]
fn successful_add_does_not_close_an_unrelated_new_draft_or_move_the_user() {
    let mut app = app();
    app.begin_search("A different draft");
    app.view = View::Settings;
    app.finish_command(CommandKind::Add("Submitted song".into()), Ok(()));
    assert_eq!(app.input, "A different draft");
    assert!(app.input_mode == InputMode::Editing && app.view == View::Settings);
}

#[tokio::test]
async fn repeat_is_pending_until_a_snapshot_confirms_it_and_rejection_does_not_change_it() {
    let mut app = app();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    app.ws_sender = Some(tx);
    app.ws_connected = true;
    let shared = Arc::new(Mutex::new(app));
    {
        let mut app = shared.lock().await;
        execute_action(Action::LoopTrack, &mut app, shared.clone());
    }
    let Message::Text(message) = timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    let sent: Value = serde_json::from_str(&message).unwrap();
    let mut app = shared.lock().await;
    assert_eq!(app.loop_mode, "off");
    assert_eq!(app.repeat_label(), "Unknown");
    assert_eq!(app.pending_repeat(), Some("track"));
    app.finish_ws_action(
        &serde_json::from_value(
            json!({"type":"action_response","id":sent["id"],"success":false,"message":"Denied"}),
        )
        .unwrap(),
    );
    assert_eq!(app.loop_mode, "off");
    assert!(app.pending_repeat().is_none());
    app.finish_command(CommandKind::Repeat("track".into()), Ok(()));
    assert_eq!(app.loop_mode, "off");
    assert_eq!(app.repeat_label(), "Unknown");
    app.parse_queue_response(&json!({"settings":{"loopMode":"track"}}));
    assert_eq!(app.loop_mode, "track");
    assert_eq!(app.repeat_label(), "track");
}

#[test]
fn bounded_paste_and_typing_cannot_submit_a_silently_truncated_request() {
    let mut app = app();
    app.begin_search("");
    handle_paste(&mut app, &"界".repeat(MAX_INPUT_BYTES));
    assert!(app.input.len() <= MAX_INPUT_BYTES && app.input.is_char_boundary(app.input_cursor));
    assert!(app.input_truncated && app.input_error.is_some());
    handle_paste(&mut app, "\x00\u{202e}");
    assert!(app.input_truncated);
    press(&mut app, KeyCode::Enter);
    assert!(app.add_pending.is_none());
    press(&mut app, KeyCode::Backspace);
    assert!(!app.input_truncated);
    handle_paste(&mut app, "\x1b\x00\u{202e}safe\u{2066}");
    assert!(!app.input.chars().any(|c| !input_char(c)));
    press(&mut app, KeyCode::Esc);
    app.view = View::Menu;
    handle_paste(&mut app, &"x".repeat(100_000));
    assert_eq!(app.palette_query.len(), MAX_PALETTE_BYTES);
    app.view = View::Settings;
    app.is_settings_editing = true;
    app.settings_input.clear();
    app.settings_cursor = 0;
    handle_paste(
        &mut app,
        &format!("https://example.com/{}", "a".repeat(100_000)),
    );
    assert_eq!(app.settings_input.len(), MAX_HOST_BYTES);
    handle_paste(&mut app, "");
    assert!(draft_settings(&app).is_err());
    press(&mut app, KeyCode::Backspace);
    assert!(!app.settings_truncated);
}

#[test]
fn editing_and_horizontal_windows_respect_graphemes_and_cell_width() {
    let family = "👨‍👩‍👧‍👦";
    let mut value = format!("Ae\u{301}{family}界");
    let mut cursor = value.len();
    let key = |code| event::KeyEvent::new(code, event::KeyModifiers::NONE);
    edit_text(&mut value, &mut cursor, key(KeyCode::Left), |_, _, _| true);
    assert_eq!(cursor, value.len() - "界".len());
    edit_text(
        &mut value,
        &mut cursor,
        key(KeyCode::Backspace),
        |_, _, _| true,
    );
    assert_eq!(value, "Ae\u{301}界");
    edit_text(
        &mut value,
        &mut cursor,
        key(KeyCode::Backspace),
        |_, _, _| true,
    );
    assert_eq!(value, "A界");
    for width in 1..12 {
        let input = format!("a{family}e\u{301}界z");
        let (window, column) = input_window(&input, input.len(), width);
        assert!(window.width() <= usize::from(width) && column < width);
    }
    let large = "a".repeat(100_000);
    assert_eq!(input_tail(&large, 10), "aaaaaaaaaa");
}

#[test]
fn stale_playback_stops_interpolation_even_with_a_connected_socket() {
    let mut app = super::polish_tests::playing();
    let recent = Instant::now();
    app.last_playback_received = Some(recent - Duration::from_secs(31));
    app.last_queue_received = Some(recent);
    assert_eq!(app.latest_received(), Some(recent));
    let elapsed = app.elapsed_ms;
    app.update_realtime();
    assert!(app.ws_connected && !app.has_playback_state && app.playback_is_stale());
    assert_eq!(app.elapsed_ms, elapsed);
    let screen = text(&draw(&mut app, 80, 24));
    assert!(screen.contains("Stale") && screen.contains("Connected"));
    app.apply_playback(PlaybackState {
        elapsed_ms: 50_000,
        duration_ms: 243_000,
        paused: true,
        spectrogram: None,
    });
    assert!(!app.playback_is_stale() && app.has_playback_state);
}

#[test]
fn focus_is_visible_and_enter_inspects_the_focused_track() {
    let mut app = super::polish_tests::playing();
    let terminal = draw(&mut app, 80, 24);
    let buffer = terminal.backend().buffer();
    let queue_at = buffer
        .content
        .windows(5)
        .position(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>() == "Queue")
        .unwrap();
    assert!(
        buffer.content[queue_at]
            .modifier
            .contains(Modifier::UNDERLINED)
    );
    press(&mut app, KeyCode::Tab);
    assert!(app.focus == PlayerFocus::Playback);
    press(&mut app, KeyCode::Enter);
    assert!(text(&draw(&mut app, 80, 24)).contains("Midnight City"));
    assert_eq!(app.selected_track().unwrap().title, "Midnight City");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.queue_state.selected(), Some(0));
}

#[test]
fn monochrome_uses_terminal_defaults_and_never_emits_artwork() {
    let mut app = super::polish_tests::playing();
    app.monochrome = true;
    for view in [
        View::Main,
        View::Settings,
        View::Lyrics,
        View::Menu,
        View::Help,
        View::ConfirmClear,
        View::TrackDetails,
        View::AuthMenu,
        View::LoginRequired,
    ] {
        app.view = view;
        app.overlay_return = View::Main;
        let terminal = draw(&mut app, 80, 24);
        assert!(app.artwork.last_area.is_none());
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );
    }
}

#[tokio::test]
async fn next_queue_page_uses_real_server_offsets_and_broadcasts_do_not_replace_it() {
    let (url, seen, reply, server) = held_http().await;
    let mut app = app();
    app.base_url = url;
    app.parse_queue_response(&page(0, 20, 45));
    let shared = Arc::new(Mutex::new(app));
    {
        let mut app = shared.lock().await;
        handle_key_event(
            &mut app,
            event::KeyEvent::new(KeyCode::Char(']'), event::KeyModifiers::NONE),
            shared.clone(),
        );
    }
    let payload = receive(seen).await;
    assert_eq!(payload["offset"], 20);
    assert_eq!(payload["limit"], 20);
    reply.send(response("200 OK", page(20, 20, 45))).unwrap();
    server.await.unwrap();
    wait_for(&shared, |app| app.queue_request.is_none()).await;
    let mut app = shared.lock().await;
    assert_eq!(app.queue[0].title, "Track 20");
    app.move_queue(5);
    assert!(app.parse_ws_state(&page(0, 20, 44)));
    assert_eq!(app.queue[0].title, "Track 20");
    assert_eq!(app.queue_total, Some(44));
    assert_eq!(app.queue_offset, 20);
    assert_eq!(app.queue_state.selected(), Some(5));
    assert!(text(&draw(&mut app, 80, 24)).contains("[ ] queue pages"));
}

#[test]
fn shrinking_and_unknown_queue_totals_do_not_trap_navigation_or_erase_cached_pages() {
    let mut app = app();
    app.queue_target_offset = 40;
    let request = app.begin_queue_request().unwrap();
    assert!(app.finish_queue_request(&request, Ok(page(40, 0, 3))));
    assert_eq!(app.queue_target_offset, 0);
    let request = app.begin_queue_request().unwrap();
    app.finish_queue_request(&request, Ok(page(0, 3, 3)));
    assert_eq!(app.queue.len(), 3);
    app.queue_total = None;
    app.queue_target_offset = 20;
    let request = app.begin_queue_request().unwrap();
    app.finish_queue_request(&request, Ok(json!({"upcoming":[]})));
    assert_eq!(app.queue.len(), 3);
    assert_eq!(app.queue_target_offset, 0);
    let request = app.begin_queue_request().unwrap();
    app.finish_queue_request(&request, Ok(json!({"upcoming":"malformed"})));
    assert_eq!(app.queue.len(), 3);
    assert!(app.queue_error.is_some());
}

#[test]
fn fast_redraws_require_a_visible_active_spectrum_not_just_a_playing_track() {
    let mut app = super::polish_tests::playing();
    app.spectrogram = Some(vec![vec![100; 64]]);
    draw(&mut app, 80, 24);
    assert_eq!(app.frame_interval(), Duration::from_secs(1));
    app.visualizer_enabled = true;
    draw(&mut app, 120, 32);
    assert_eq!(app.frame_interval(), state::FRAME_MIN_INTERVAL);
    app.open_overlay(View::Menu);
    draw(&mut app, 120, 32);
    assert_eq!(app.frame_interval(), Duration::from_secs(1));
    app.view = View::Main;
    draw(&mut app, 24, 8);
    assert_eq!(app.frame_interval(), Duration::from_secs(1));
}

#[tokio::test]
async fn switching_sessions_cancels_queued_dispatch_and_shutdown_cancels_background_work() {
    let mut app = app();
    let (dispatch_tx, dispatch_rx) = oneshot::channel();
    app.spawn(async move {
        let _ = dispatch_tx.send(());
    });
    app.clear_session();
    assert!(
        timeout(Duration::from_secs(3), dispatch_rx)
            .await
            .unwrap()
            .is_err()
    );

    struct Dropped(Option<oneshot::Sender<()>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    app.spawn_background(async move {
        let _guard = Dropped(Some(dropped_tx));
        started_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    receive(started_rx).await;
    app.clear_session();
    app.shutdown.cancel();
    receive(dropped_rx).await;
}
