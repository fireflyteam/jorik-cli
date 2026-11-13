use super::tests::app;
use super::*;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(app: &mut App, code: KeyCode) -> UiOutcome {
    handle_key_event(
        app,
        event::KeyEvent::new(code, event::KeyModifiers::NONE),
        Arc::new(Mutex::new(self::app())),
    )
}

#[test]
fn live_metadata_stays_structured_and_missing_values_are_explicit() {
    let mut app = app();
    app.parse_queue_response(&json!({
        "current":{"id":"current", "title":"Title - Live", "author":"Artist - Guests", "album":{"name":"Album - Expanded"}, "durationMs":243_000},
        "upcoming":[{"id":"next", "title":"Next", "author":"Band", "album":"Record", "durationMs":127_000}],
        "total_upcoming":37,
        "playback":{"elapsedMs":124_000,"durationMs":243_000,"paused":false}
    }));
    let current = app.current_track.as_ref().unwrap();
    assert_eq!(current.title, "Title - Live");
    assert_eq!(current.artist, "Artist - Guests");
    assert_eq!(current.album.as_deref(), Some("Album - Expanded"));
    let rendered = screen(&mut app, 120, 32);
    for value in [
        "Title - Live",
        "Artist - Guests",
        "Album - Expanded",
        "Record",
        "2:07",
        "4:03",
        "37 in queue",
    ] {
        assert!(rendered.contains(value), "missing {value}");
    }
    assert!(!rendered.contains("Length 4:03"));
    app.parse_queue_response(&json!({"current":{"id":"current", "title":"Title - Live", "author":"Artist - Guests"}, "upcoming":[{"title":"No metadata"}]}));
    let rendered = screen(&mut app, 80, 24);
    assert!(rendered.contains("Unknown album"));
    assert!(app.queue[0].duration_ms.is_none());
    assert!(app.queue_total.is_none());
}

#[test]
fn metadata_updates_do_not_reset_playback_or_lyrics_for_the_same_recording() {
    let mut app = app();
    app.parse_queue_response(&json!({"current":{"id":"same", "title":"Song", "author":"Artist"}, "playback":{"elapsedMs":50_000,"durationMs":100_000,"paused":false}}));
    app.lyrics_text = Some("Cached lyrics".into());
    app.parse_queue_response(&json!({"current":{"id":"same", "title":"Song", "author":"Artist", "album":"New metadata"}}));
    assert_eq!(app.lyrics_text.as_deref(), Some("Cached lyrics"));
    assert!(app.has_playback_state);
    assert_eq!(app.elapsed_ms, 50_000);
    app.parse_queue_response(&json!({"current":null, "upcoming":[]}));
    assert!(app.current_track.is_none() && !app.has_playback_state && app.lyrics_text.is_none());
}

#[test]
fn enter_inspects_without_a_remote_action_and_add_keeps_q_as_text() {
    let mut app = app();
    app.parse_queue_response(&json!({"upcoming":[{"title":"First"},{"title":"Second", "album":"Selected album", "durationMs":62_000}]}));
    app.move_queue(1);
    press(&mut app, KeyCode::Enter);
    assert!(app.view == View::TrackDetails);
    assert!(app.pending_commands.is_empty());
    assert!(screen(&mut app, 80, 24).contains("Selected album"));
    press(&mut app, KeyCode::Esc);
    assert!(app.view == View::Main && app.queue_state.selected() == Some(1));
    press(&mut app, KeyCode::Char('a'));
    assert!(app.input_mode == InputMode::Editing);
    assert!(matches!(
        press(&mut app, KeyCode::Char('q')),
        UiOutcome::Continue
    ));
    assert_eq!(app.input, "q");
    press(&mut app, KeyCode::Esc);
    assert!(app.input_mode == InputMode::Normal);
}

#[test]
fn artwork_keeps_full_sidebar_width_in_tall_layouts_and_hides_under_dialogs() {
    let mut app = super::polish_tests::playing();
    let _ = screen(&mut app, 120, 50);
    let area = app.artwork.last_area.unwrap();
    assert_eq!(area.width, 38);
    assert_eq!(area.x, 1);
    assert_eq!(area.bottom(), 47);
    press(&mut app, KeyCode::Enter);
    let _ = screen(&mut app, 120, 50);
    assert!(app.artwork.last_area.is_none());
    press(&mut app, KeyCode::Esc);
    app.clear_session();
    let _ = screen(&mut app, 120, 50);
    assert!(app.artwork.last_area.is_none());
}

#[test]
fn new_and_existing_standard_settings_use_desk_and_other_layouts_are_preserved() {
    let mut settings = api::Settings {
        base_url: api::DEFAULT_BASE_URL.into(),
        visualizer_offset: 0,
        theme: "Default".into(),
        visualizer_style: "Bars".into(),
        layout: "Standard".into(),
    };
    let standard = App::new(Client::new(), settings.clone(), None, None, None);
    assert_eq!(standard.layout, "Desk");
    assert!(!standard.visualizer_enabled);
    settings.layout = "Studio".into();
    assert_eq!(
        App::new(Client::new(), settings, None, None, None).layout,
        "Studio"
    );
}

#[test]
fn background_errors_do_not_consume_back_navigation_from_settings_or_lyrics() {
    for view in [View::Settings, View::Lyrics] {
        let mut app = app();
        app.view = view;
        app.queue_error = Some("Queue refresh failed".into());
        press(&mut app, KeyCode::Esc);
        assert!(app.view == View::Main);
        assert!(app.queue_error.is_some());
        press(&mut app, KeyCode::Esc);
        assert!(app.queue_error.is_none());
    }
}
