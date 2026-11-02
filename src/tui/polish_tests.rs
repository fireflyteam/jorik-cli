use super::tests::app;
use super::*;
use ratatui::{Terminal, backend::TestBackend};

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
        .chunks(usize::from(terminal.size().unwrap().width))
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

pub(super) fn assert_snapshot(name: &str, actual: &str) {
    let actual = actual.trim_end_matches('\n');
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("src/tui/snapshots/{name}.txt"));
    if std::env::var("JORIK_UPDATE_SNAPSHOTS").as_deref() == Ok("1") {
        std::fs::write(path, format!("{actual}\n")).unwrap();
    } else {
        let expected = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
        assert_eq!(actual, expected.strip_suffix('\n').unwrap_or(&expected));
    }
}

fn snapshot(name: &str, app: &mut App, width: u16, height: u16) {
    let terminal = draw(app, width, height);
    assert_snapshot(name, &text(&terminal));
    if let Some(directory) = std::env::var_os("JORIK_UI_PREVIEWS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let rgb = |color: Color| match color {
            Color::Rgb(r, g, b) => [r, g, b],
            Color::Black => [0, 0, 0],
            _ => [231, 234, 244],
        };
        let cells=terminal.backend().buffer().content.iter().map(|cell|serde_json::json!({"text":cell.symbol(),"fg":rgb(cell.fg),"bg":rgb(cell.bg),"bold":cell.modifier.contains(ratatui::style::Modifier::BOLD)})).collect::<Vec<_>>();
        std::fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec(&serde_json::json!({"width":width,"height":height,"cells":cells}))
                .unwrap(),
        )
        .unwrap();
    }
}

fn playing() -> App {
    let mut app = app();
    app.ws_connected = true;
    app.current_track = Some("Midnight City - M83".into());
    app.paused = false;
    app.has_playback_state = true;
    app.elapsed_ms = 124_000;
    app.duration_ms = 244_000;
    app.queue = vec![
        "Intro - The xx".into(),
        "Nightcall - Kavinsky".into(),
        "A Real Hero - College".into(),
    ];
    app
}

fn key(app: &mut App, code: KeyCode) {
    handle_key_event(
        app,
        event::KeyEvent::new(code, event::KeyModifiers::NONE),
        Arc::new(Mutex::new(self::app())),
    );
}

#[test]
fn polished_screen_snapshots() {
    let mut app = playing();
    snapshot("player-80x24", &mut app, 80, 24);
    app.spectrogram = Some(vec![vec![50; 64]]);
    app.smoothed_bars = (0..64)
        .map(|i| ((i as f32 * 0.27).sin().abs() * 80.0) + 8.0)
        .collect();
    snapshot("player-120x32", &mut app, 120, 32);
    app.view = View::Settings;
    snapshot("settings-80x24", &mut app, 80, 24);
    app.view = View::Main;
    app.open_overlay(View::Menu);
    app.palette_query = "queue".into();
    snapshot("palette-80x24", &mut app, 80, 24);
    app.view = View::LoginRequired;
    app.token = None;
    snapshot("welcome-80x24", &mut app, 80, 24);
    app.theme = "Emerald".into();
    app.legacy_credentials = true;
    snapshot("welcome-legacy-80x24", &mut app, 80, 24);
    snapshot("welcome-legacy-98x24", &mut app, 98, 24);
    app.open_overlay(View::Help);
    snapshot("sign-in-help-80x24", &mut app, 80, 24);
}

#[test]
fn failed_websocket_upgrade_preserves_http_playback_but_a_live_disconnect_invalidates_it() {
    let mut app = playing();
    app.ws_connected = false;
    app.disconnect_ws();
    assert!(app.has_playback_state);
    app.ws_connected = true;
    app.disconnect_ws();
    assert!(!app.has_playback_state);
}

#[test]
fn named_themes_keep_secondary_text_and_selected_text_readable() {
    let luminance = |color: Color| {
        let Color::Rgb(r, g, b) = color else {
            panic!("expected an RGB theme");
        };
        let linear = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    };
    let contrast = |a, b| {
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    };
    for name in [
        "Default",
        "Midnight",
        "Emerald",
        "Ruby",
        "Ocean",
        "Synthwave",
        "Sepia",
    ] {
        let theme = get_theme(name);
        assert!(
            contrast(theme.text_secondary, theme.raised()) >= 7.0,
            "{name}: secondary text"
        );
        assert!(
            contrast(theme.highlight, theme.selection()) >= 4.5,
            "{name}: selection"
        );
        assert!(
            contrast(theme.bg, theme.highlight) >= 4.5,
            "{name}: primary action"
        );
    }
}

#[test]
fn unicode_clipping_respects_cell_width_and_neutralizes_controls() {
    for width in 0..20 {
        for value in [
            "音楽 — 日本語",
            "aé e\u{301} music",
            "very long artist name",
            "\x1b[31mtest",
            "\u{301}",
        ] {
            let value = design::fit_text(value, width);
            assert!(Line::from(value.as_str()).width() <= usize::from(width));
            assert!(!value.contains('\x1b'));
        }
    }
    assert_eq!(design::fit_text("Music", 5), "Music");
    assert!(design::fit_text("音楽", 3).ends_with('…'));
}

#[test]
fn empty_queries_stay_open_with_inline_feedback() {
    let mut app = app();
    app.begin_search("  ");
    key(&mut app, KeyCode::Enter);
    assert!(app.input_mode == InputMode::Editing);
    assert!(app.input_error.is_some());
    assert!(text(&draw(&mut app, 80, 24)).contains("Enter a song name"));
    key(&mut app, KeyCode::Char('q'));
    assert!(app.input_error.is_none());
    assert_eq!(app.input, "  q");
}

#[test]
fn settings_support_cursor_editing_paste_and_reverse_navigation_without_saving() {
    let mut app = app();
    app.view = View::Settings;
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.settings_cursor, app.settings_input.len());
    app.settings_input = "é音".into();
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Char('a'));
    key(&mut app, KeyCode::Delete);
    handle_paste(&mut app, "β\n");
    assert_eq!(app.settings_input, "aβ音");
    assert_eq!(app.settings_cursor, "aβ".len());
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.settings_input, "aβ");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.settings_input, app.base_url);
    key(&mut app, KeyCode::BackTab);
    assert!(app.settings_field == SettingsField::Layout);
}

#[test]
fn legacy_warning_does_not_replace_onboarding_or_expose_commands_until_help() {
    let mut app = app();
    app.view = View::LoginRequired;
    app.legacy_credentials = true;
    app.theme = "Emerald".into();
    let terminal = draw(&mut app, 80, 24);
    let screen = text(&terminal);
    assert!(screen.contains("Sign in to Jorik") && screen.contains("Connect your Discord account"));
    assert!(!screen.contains("1. Sign in") && !screen.contains("3. Add a song"));
    assert!(screen.contains("Older login files found"));
    assert!(!screen.contains("auth migrate"));
    let theme = get_theme(&app.theme);
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .filter(|cell| cell.bg == theme.highlight)
            .count()
            > 20
    );
    key(&mut app, KeyCode::Char('?'));
    assert!(app.view == View::Help && app.content_view() == View::LoginRequired);
    let help = text(&draw(&mut app, 80, 24));
    assert!(help.contains("jorik auth migrate --confirm-server SERVER_URL"));
    assert!(help.contains("original issuing server"));
    key(&mut app, KeyCode::Esc);
    assert!(app.view == View::LoginRequired);
    assert!(app.legacy_credentials);
}

#[test]
fn welcome_sections_have_real_blank_rows_at_screenshot_sizes() {
    for (width, height) in [(80, 24), (98, 24), (80, 20)] {
        let mut app = app();
        app.view = View::LoginRequired;
        app.legacy_credentials = true;
        let terminal = draw(&mut app, width, height);
        let screen = text(&terminal);
        let rows = screen.lines().collect::<Vec<_>>();
        let at = |label: &str| rows.iter().position(|line| line.contains(label)).unwrap();
        let blank = |row: usize| rows[row].chars().all(|c| c.is_whitespace() || c == '│');
        let title = at("Sign in to Jorik");
        let server = at("https://example.com");
        let action = at("Enter");
        let shortcuts = at("change server");
        let notice = at("Older login files found");
        assert!(blank(title + 1));
        assert!(blank(server + 1) && blank(server + 2));
        assert!(action >= server + 3 && blank(action + 1));
        assert!(shortcuts >= action + 2 && blank(shortcuts + 1));
        assert!(notice >= shortcuts + 2);
    }
}

#[test]
fn compact_sign_in_and_confirmation_keep_the_actions_visible() {
    let mut app = app();
    app.view = View::LoginRequired;
    for (width, height) in [(40, 12), (24, 8)] {
        let screen = text(&draw(&mut app, width, height));
        assert!(
            screen.contains("Enter") && screen.contains("sign in") && screen.contains("q quit")
        );
    }
    app.view = View::ConfirmClear;
    let screen = text(&draw(&mut app, 24, 8));
    assert!(screen.contains("keep queue") && screen.contains("clear queue"));
}

#[test]
fn changing_tracks_invalidates_cached_lyrics() {
    let mut app = playing();
    app.lyrics_text = Some("Old lyrics".into());
    app.lyrics_scroll = 10;
    app.parse_queue_response(
        &serde_json::json!({"current":{"title":"New track","author":"Artist"},"upcoming":[]}),
    );
    assert!(app.lyrics_text.is_none());
    assert_eq!(app.lyrics_scroll, 0);
}

#[test]
fn nested_dialogs_return_to_the_screen_that_opened_them() {
    for action in [Action::Filters, Action::Auth, Action::Info] {
        let mut app = app();
        app.view = View::Settings;
        app.open_overlay(View::Menu);
        execute_action(action, &mut app, Arc::new(Mutex::new(self::app())));
        assert!(app.content_view() == View::Settings);
        key(&mut app, KeyCode::Esc);
        assert!(app.view == View::Settings);
    }
}

#[test]
fn lyrics_scroll_uses_actual_wrapping_and_stops_at_the_last_line() {
    let mut app = app();
    app.view = View::Lyrics;
    app.lyrics_text = Some(
        (0..70)
            .map(|i| format!("Line {i} with enough words to wrap around a narrow view"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    draw(&mut app, 40, 12);
    key(&mut app, KeyCode::End);
    let screen = text(&draw(&mut app, 40, 12));
    assert!(screen.contains("Line 69") || screen.contains("narrow view"));
    let max = app.lyrics_max_scroll;
    assert!(max > 70);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.lyrics_scroll, max);
    key(&mut app, KeyCode::Home);
    assert_eq!(app.lyrics_scroll, 0);
    app.lyrics_text = Some("Short lyrics".into());
    app.lyrics_scroll = u16::MAX;
    draw(&mut app, 80, 24);
    assert_eq!(app.lyrics_scroll, 0);
}

#[test]
fn all_screens_and_editors_keep_the_cursor_inside_small_terminals() {
    for (width, height) in [(1, 1), (24, 8), (40, 12), (80, 24), (120, 32)] {
        for view in [
            View::Main,
            View::Settings,
            View::Lyrics,
            View::Menu,
            View::Help,
            View::ConfirmClear,
            View::AuthMenu,
            View::AuthResult,
            View::LoginRequired,
            View::FilterMenu,
            View::UpdateFound,
            View::AppInfo,
            View::Debug,
        ] {
            let mut app = playing();
            app.view = view;
            app.input = "音楽é".repeat(100);
            app.input_cursor = app.input.len();
            for editing in [false, true] {
                app.input_mode = if editing {
                    InputMode::Editing
                } else {
                    InputMode::Normal
                };
                let mut terminal = draw(&mut app, width, height);
                let cursor = terminal.get_cursor_position().unwrap();
                assert!(cursor.x < width && cursor.y < height);
            }
        }
    }
}

#[tokio::test]
async fn duplicate_lyrics_fetches_are_coalesced_and_old_track_results_are_not_mislabeled() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut app = playing();
    app.base_url = format!("http://{}", listener.local_addr().unwrap());
    let shared = Arc::new(Mutex::new(app));
    let first = tokio::spawn(async_fetch_lyrics(shared.clone()));
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut request = [0; 4096];
    assert!(stream.read(&mut request).await.unwrap() > 0);
    assert!(shared.lock().await.lyrics_loading);
    async_fetch_lyrics(shared.clone()).await;
    assert!(
        timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    shared.lock().await.current_track = Some("New song - Artist".into());
    let body = r#"{"data":{"text":"Old song lyrics"}}"#;
    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    first.await.unwrap();
    let app = shared.lock().await;
    assert!(!app.lyrics_loading);
    assert!(
        app.lyrics_text
            .as_deref()
            .unwrap()
            .contains("track changed")
    );
}
