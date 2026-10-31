use super::tests::app;
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn key(code: KeyCode) -> event::KeyEvent {
    event::KeyEvent::new(code, event::KeyModifiers::NONE)
}

fn press(app: &mut App, code: KeyCode) -> UiOutcome {
    handle_key_event(app, key(code), Arc::new(Mutex::new(self::app())))
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn compact_player_snapshot() {
    let mut app = app();
    app.ws_connected = true;
    app.current_track = Some("Midnight City - M83".to_string());
    app.paused = false;
    app.has_playback_state = true;
    app.elapsed_ms = 124_000;
    app.duration_ms = 244_000;
    app.queue = vec![
        "Intro - The xx".to_string(),
        "Nightcall - Kavinsky".to_string(),
        "A Real Hero - College".to_string(),
    ];
    let actual = screen(&mut app, 80, 24);
    let expected = include_str!("snapshots/player-80x24.txt").replace("\r\n", "\n");
    assert_eq!(actual, expected.trim_end());
    assert!(actual.contains("Midnight City"));
    assert!(actual.contains("Queue · 3 tracks"));
    assert!(actual.contains("Space pause"));
    assert!(!actual.contains("Visualizer"));
}

#[test]
fn visualizer_yields_to_queue_on_narrow_or_short_screens() {
    let mut app = app();
    for (width, height) in [(80, 24), (120, 12), (40, 10)] {
        let actual = screen(&mut app, width, height);
        assert!(actual.contains("Queue"));
        assert!(!actual.contains("Visualizer"));
    }
    assert!(screen(&mut app, 120, 32).contains("Visualizer"));
    app.visualizer_enabled = false;
    assert!(!screen(&mut app, 120, 32).contains("Visualizer"));
}

#[test]
fn zen_mode_exposes_queue_when_queue_is_focused() {
    let mut app = app();
    app.layout = "Zen".to_string();
    app.focus = PlayerFocus::Playback;
    assert!(!screen(&mut app, 80, 24).contains("Queue ·"));
    press(&mut app, KeyCode::Tab);
    assert!(screen(&mut app, 80, 24).contains("Queue ·"));
}

#[test]
fn queue_navigation_scrolls_and_clamps_to_available_rows() {
    let mut app = app();
    app.queue = (1..=100).map(|i| format!("Track {i:03}")).collect();
    screen(&mut app, 80, 24);
    press(&mut app, KeyCode::End);
    assert_eq!(app.queue_state.selected(), Some(99));
    let actual = screen(&mut app, 80, 24);
    assert!(actual.contains("Track 100"));
    assert!(!actual.contains("Track 001"));
    let page = app.queue_page_size;
    press(&mut app, KeyCode::PageUp);
    assert_eq!(app.queue_state.selected(), Some(99 - page));
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.queue_state.selected(), Some(0));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.queue_state.selected(), Some(1));
    assert!(app.input_mode == InputMode::Normal);
    app.parse_queue_response(&json!({"upcoming": [{"title": "Remaining"}]}));
    assert_eq!(app.queue_state.selected(), Some(0));
    app.parse_queue_response(&json!({"upcoming": []}));
    assert_eq!(app.queue_state.selected(), None);
    press(&mut app, KeyCode::End); // Empty queues stay safe.
}

#[test]
fn playback_status_is_not_guessed_and_old_track_data_is_cleared() {
    let mut app = app();
    app.parse_queue_response(&json!({"current": {"title": "First", "author": "Artist"}}));
    assert!(screen(&mut app, 80, 24).contains("State unavailable"));
    app.parse_queue_response(&json!({
        "current": {"title": "First", "author": "Artist"},
        "playback": {"elapsedMs": 1000, "durationMs": 60_000, "paused": false, "spectrogram": [[50]]}
    }));
    assert!(app.has_playback_state);
    assert!(screen(&mut app, 80, 24).contains("Playing"));
    app.parse_queue_response(&json!({"current": {"title": "Second", "author": "Artist"}}));
    assert!(!app.has_playback_state);
    assert_eq!(app.elapsed_ms, 0);
    assert!(app.spectrogram.is_none());
}

#[test]
fn focus_cycles_in_both_directions() {
    let mut app = app();
    press(&mut app, KeyCode::Tab);
    assert!(app.focus == PlayerFocus::Playback);
    press(&mut app, KeyCode::BackTab);
    assert!(app.focus == PlayerFocus::Queue);
}

#[test]
fn palette_filters_by_text_and_preserves_the_underlying_screen() {
    let mut app = app();
    app.view = View::Settings;
    press(&mut app, KeyCode::Char(':'));
    assert!(app.view == View::Menu);
    assert!(app.content_view() == View::Settings);
    for c in "queue".chars() {
        assert!(matches!(
            press(&mut app, KeyCode::Char(c)),
            UiOutcome::Continue
        ));
    }
    assert_eq!(app.palette_query, "queue");
    assert!(
        actions::filtered_commands("QUEUE")
            .iter()
            .all(|c| c.label.to_lowercase().contains("queue"))
    );
    press(&mut app, KeyCode::Char('3'));
    assert!(app.view == View::Menu);
    press(&mut app, KeyCode::Esc);
    assert!(app.view == View::Settings);
}

#[test]
fn control_p_opens_the_palette_without_pausing() {
    let mut app = app();
    let control_p = event::KeyEvent::new(KeyCode::Char('p'), event::KeyModifiers::CONTROL);
    handle_key_event(&mut app, control_p, Arc::new(Mutex::new(self::app())));
    assert!(app.view == View::Menu);
    assert!(app.pending_commands.is_empty());
}

#[test]
fn palette_dispatches_a_typed_action() {
    let mut app = app();
    press(&mut app, KeyCode::Char(':'));
    handle_paste(&mut app, "visualizer");
    press(&mut app, KeyCode::Enter);
    assert!(!app.visualizer_enabled);
    assert!(app.view == View::Main);
    assert!(app.notice.is_some());
}

#[test]
fn empty_palette_results_are_safe_and_query_is_not_a_quit_shortcut() {
    let mut app = app();
    press(&mut app, KeyCode::Char(':'));
    handle_paste(&mut app, "no matching q12345");
    let actual = screen(&mut app, 80, 24);
    assert!(actual.contains("No matching actions"));
    assert!(matches!(
        press(&mut app, KeyCode::Char('q')),
        UiOutcome::Continue
    ));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Up);
    press(&mut app, KeyCode::Enter);
    assert!(app.view == View::Menu);
}

#[test]
fn help_returns_to_the_previous_screen() {
    let mut app = app();
    app.view = View::Lyrics;
    press(&mut app, KeyCode::Char('?'));
    assert!(app.view == View::Help);
    assert!(screen(&mut app, 80, 24).contains("Keyboard help"));
    press(&mut app, KeyCode::Down);
    assert_eq!(app.help_scroll, 1);
    press(&mut app, KeyCode::Esc);
    assert!(app.view == View::Lyrics);
}

#[test]
fn updates_do_not_steal_focus_or_input() {
    let mut app = app();
    app.view = View::Lyrics;
    app.begin_search("a query");
    app.offer_update(("v9.0.0".to_string(), vec![]));
    assert!(app.view == View::Lyrics);
    assert!(app.input_mode == InputMode::Editing);
    assert_eq!(app.input, "a query");
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('u'));
    assert!(app.view == View::UpdateFound);
    press(&mut app, KeyCode::Esc);
    assert!(app.view == View::Lyrics);
}

#[test]
fn escape_dismisses_notifications_without_losing_queue_focus() {
    let mut app = app();
    app.queue = vec!["A track".to_string()];
    app.queue_state.select(Some(0));
    app.error_message = Some("Rejected".to_string());
    app.offer_update(("v9.0.0".to_string(), vec![]));
    press(&mut app, KeyCode::Esc);
    assert!(app.error_message.is_none());
    assert!(!app.update_notice);
    assert_eq!(app.queue_state.selected(), Some(0));
    assert!(app.view == View::Main);
}

#[test]
fn unicode_editing_supports_insertion_deletion_and_paste() {
    let mut app = app();
    press(&mut app, KeyCode::Char('/'));
    handle_paste(&mut app, "a音b\n");
    press(&mut app, KeyCode::Left);
    press(&mut app, KeyCode::Char('é'));
    assert_eq!(app.input, "a音éb");
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.input, "a音b");
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Delete);
    assert_eq!(app.input, "音b");
    press(&mut app, KeyCode::End);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.input, "音");
    assert_eq!(app.input_cursor, "音".len());
    let (text, column) = input_window("abc音楽xyz", "abc音".len(), 4);
    assert!(Line::from(text).width() <= 4);
    assert!(column < 4);
}

#[test]
fn dialogs_render_at_tiny_sizes_without_invalid_cursors() {
    for (width, height) in [(1, 1), (2, 2), (10, 8), (40, 12), (80, 24)] {
        for view in [
            View::Menu,
            View::Help,
            View::ConfirmClear,
            View::UpdateFound,
            View::LoginRequired,
        ] {
            let mut app = app();
            app.view = view;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| ui(frame, &mut app)).unwrap();
            let cursor = terminal.get_cursor_position().unwrap();
            assert!(cursor.x < width && cursor.y < height);
        }
    }
}

#[tokio::test]
async fn space_and_legacy_pause_share_the_same_transport_action() {
    let mut app = app();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    app.ws_sender = Some(tx);
    app.ws_connected = true;
    let shared = Arc::new(Mutex::new(app));
    for code in [KeyCode::Char(' '), KeyCode::Char('p')] {
        let mut guard = shared.lock().await;
        handle_key_event(&mut guard, key(code), shared.clone());
        drop(guard);
        let message = timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let Message::Text(text) = message else {
            panic!("expected a WS action");
        };
        let data: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(data["action"], "pause");
    }
}

#[tokio::test]
async fn clear_requires_explicit_confirmation_and_enter_never_confirms() {
    let mut app = app();
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    app.ws_sender = Some(tx);
    app.ws_connected = true;
    let shared = Arc::new(Mutex::new(app));
    let mut guard = shared.lock().await;
    handle_key_event(&mut guard, key(KeyCode::Char('c')), shared.clone());
    assert!(guard.view == View::ConfirmClear);
    handle_key_event(&mut guard, key(KeyCode::Enter), shared.clone());
    assert!(rx.try_recv().is_err());
    handle_key_event(&mut guard, key(KeyCode::Esc), shared.clone());
    assert!(guard.view == View::Main);
    handle_key_event(&mut guard, key(KeyCode::Char('c')), shared.clone());
    handle_key_event(&mut guard, key(KeyCode::Char('y')), shared.clone());
    assert!(guard.view == View::Main);
    drop(guard);
    let Message::Text(text) = timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected a WS action");
    };
    let data: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(data["action"], "clear");
}
