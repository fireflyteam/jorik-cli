use super::*;

pub(super) fn render_header(f: &mut Frame, app: &App, theme: &Theme, view: View, area: Rect) {
    let tabs = [
        (View::Main, "[1] Player"),
        (View::Lyrics, "[2] Lyrics"),
        (View::Settings, "[3] Settings"),
    ];
    let mut title = vec![Span::styled(
        " JORIK  ",
        Style::default().fg(theme.highlight).bold(),
    )];
    for (tab, label) in tabs {
        title.push(Span::styled(
            format!("{label}  "),
            Style::default().fg(if view == tab {
                theme.highlight
            } else {
                theme.text_secondary
            }),
        ));
    }
    let (connection, color) = if app.ws_connected {
        ("Connected", Color::Green)
    } else if app.ws_connecting {
        ("Connecting", Color::Yellow)
    } else {
        ("REST fallback", theme.text_secondary)
    };
    let host = Url::parse(&app.base_url)
        .map(|url| url[url::Position::BeforeHost..url::Position::AfterPort].to_string())
        .unwrap_or_else(|_| "Invalid server".to_string());
    let guild = app.guild_id.as_deref().unwrap_or("join a voice channel");
    let context = Line::from(vec![
        Span::styled(format!(" {connection}"), Style::default().fg(color)),
        Span::styled(
            format!(" · {host} · Guild {guild}"),
            Style::default().fg(theme.text_secondary),
        ),
    ]);
    f.render_widget(Paragraph::new(vec![Line::from(title), context]), area);
}

pub(super) fn render_footer(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let error = app.error_message.as_ref().or(app.queue_error.as_ref());
    let (status, color) = if let Some(error) = error {
        (format!(" {error} · Esc dismiss"), Color::Red)
    } else if let Some((notice, _)) = &app.notice {
        (format!(" {notice}"), theme.highlight)
    } else if app.update_notice {
        let version = app
            .update_info
            .as_ref()
            .map(|(version, _)| version.as_str())
            .unwrap_or("");
        (
            format!(" Update {version} available · u review · Esc dismiss"),
            theme.highlight,
        )
    } else if app.is_busy() {
        (
            " Working… waiting for server confirmation".to_string(),
            theme.text_secondary,
        )
    } else {
        let focus = if app.focus == PlayerFocus::Queue {
            "Queue"
        } else {
            "Playback"
        };
        (
            format!(" {focus} focus · {} tracks loaded", app.queue.len()),
            theme.text_secondary,
        )
    };
    let hints = match app.view {
        View::Menu => " Type to filter · ↑↓ select · Enter run · Esc cancel",
        View::Help => " ↑↓ scroll help · Esc close",
        View::ConfirmClear => " y clear queue · n / Esc cancel",
        View::Settings if app.is_settings_editing => " Enter save · Esc discard",
        View::Settings => " ↑↓ select · ←→ change · Enter edit · : actions · ? help",
        View::Lyrics => " ↑↓ scroll lyrics · Esc player · : actions · ? help",
        View::Debug => " s save spectrogram · Esc player · : actions · ? help",
        _ if app.input_mode == InputMode::Editing => {
            " Enter add track · ←→ move · Home/End · Esc cancel"
        }
        _ => " Space pause  / add  Tab focus  ↑↓ queue  : actions  ? help  q quit",
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                status.replace(['\n', '\r'], " "),
                Style::default().fg(color),
            )),
            Line::from(Span::styled(
                hints,
                Style::default().fg(theme.text_secondary),
            )),
        ]),
        area,
    );
}

pub(super) fn popup_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

pub(super) fn render_palette(f: &mut Frame, app: &mut App, theme: &Theme) {
    let commands = actions::filtered_commands(&app.palette_query);
    let area = popup_rect(68, (commands.len() as u16 + 5).clamp(8, 18), f.area());
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .title(" Actions · type to filter ")
        .border_style(Style::default().fg(theme.highlight))
        .bg(theme.bg);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);
    let query = input_tail(&app.palette_query, rows[0].width.saturating_sub(3));
    f.render_widget(Paragraph::new(format!(": {query}")), rows[0]);
    if commands.is_empty() {
        app.menu_state.select(None);
        f.render_widget(Paragraph::new("No matching actions"), rows[1]);
    } else {
        let selected = app
            .menu_state
            .selected()
            .unwrap_or(0)
            .min(commands.len() - 1);
        app.menu_state.select(Some(selected));
        let items = commands
            .iter()
            .map(|command| {
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{}  ", command.label)),
                    Span::styled(command.shortcut, Style::default().fg(theme.text_secondary)),
                ]))
            })
            .collect::<Vec<_>>();
        f.render_stateful_widget(
            List::new(items)
                .highlight_symbol("› ")
                .highlight_style(Style::default().bg(theme.primary).fg(Color::Black).bold()),
            rows[1],
            &mut app.menu_state,
        );
    }
    f.render_widget(
        Paragraph::new("↑↓ select · Enter run · Esc cancel")
            .style(Style::default().fg(theme.text_secondary)),
        rows[2],
    );
    if rows[0].height > 0 && rows[0].width >= 3 {
        f.set_cursor_position((rows[0].x + 2 + Line::from(query).width() as u16, rows[0].y));
    }
}

pub(super) fn render_help(f: &mut Frame, app: &App, theme: &Theme) {
    let area = popup_rect(74, 21, f.area());
    f.render_widget(Clear, area);
    let text = [
        "PLAYER",
        "Space / p       Pause or resume",
        "/ / Enter       Add a track or URL",
        "Tab / Shift+Tab Switch playback / queue focus",
        "↑↓ / j k        Select a queue row",
        "PageUp/PageDown Scroll by a page",
        "Home / End      First / last queue row",
        "s  skip   w  stop   l  loop   c  clear (confirmation)",
        "v               Toggle visualization when space permits",
        "",
        "NAVIGATION",
        "1 player · 2 lyrics · 3 settings · 4 diagnostics",
        ":               Search all actions (formerly Tab menu)",
        "?               This help",
        "u               Review an available update",
        "Esc             Dismiss notification / close overlay",
        "q / Ctrl+c      Quit (q is text while editing)",
        "",
        "Queue selection is navigation only; remove/reorder/jump",
        "are not exposed without backend support.",
    ];
    f.render_widget(
        Paragraph::new(text.join("\n"))
            .block(
                Block::bordered()
                    .title(" Keyboard help · ↑↓ scroll · Esc close ")
                    .border_style(Style::default().fg(theme.highlight))
                    .bg(theme.bg),
            )
            .wrap(Wrap { trim: false })
            .scroll((app.help_scroll, 0)),
        area,
    );
}

pub(super) fn render_clear_confirmation(f: &mut Frame, theme: &Theme) {
    let area = popup_rect(54, 7, f.area());
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(
            "Clear all upcoming tracks?\nThis cannot be undone.\n\ny clear queue · n / Esc cancel",
        )
        .block(
            Block::bordered()
                .title(" Clear queue? ")
                .bg(theme.bg)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .wrap(Wrap { trim: true }),
        area,
    );
}
