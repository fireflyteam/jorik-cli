//! Production Desk layout: playback and session at left, metadata queue at right.
use super::design::{fit_text, padded, single_line, time_label};
use super::*;

pub(super) fn visible(area: Rect) -> bool {
    area.width >= 78 && area.height >= 18
}

fn row(area: Rect, y: u16) -> Rect {
    Rect::new(
        area.x,
        area.y + y.min(area.height),
        area.width,
        u16::from(y < area.height),
    )
}

fn text(f: &mut Frame, area: Rect, value: impl Into<String>, color: Color) {
    f.render_widget(Paragraph::new(value.into()).fg(color), area);
}

pub(super) fn render(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    if app.layout == "Desk" && visible(area) {
        let spacious = area.height >= 24;
        let cols = Layout::horizontal([
            Constraint::Length(if area.width >= 110 { 38 } else { 34 }),
            Constraint::Length(4),
            Constraint::Min(0),
        ])
        .split(area);
        f.render_widget(Block::default().bg(theme.surface()), cols[0]);
        let playback_height = if spacious { 9 } else { 6 };
        let session_height = if spacious { 9 } else { 4 };
        let cover_height = cols[0]
            .height
            .saturating_sub(playback_height + session_height + 2)
            .min(app.artwork.rows_for_width(cols[0].width));
        let left = Layout::vertical([
            Constraint::Length(playback_height),
            Constraint::Length(1),
            Constraint::Length(session_height),
            Constraint::Min(1),
            Constraint::Length(cover_height),
        ])
        .split(cols[0]);
        playback(f, app, theme, left[0], true);
        session(f, app, theme, left[2], spacious);
        // Keep graphics from painting over modal text on terminals with image layers.
        if app.current_track.is_some()
            && app.view == View::Main
            && app.input_mode == InputMode::Normal
            && !app.monochrome
            && cover_height >= 3
        {
            app.artwork.render(f, left[4], theme);
        }
        render_queue(f, cols[2], app, theme);
    } else {
        let now_height = if area.height >= 12 {
            6
        } else {
            3.min(area.height)
        };
        let rows = Layout::vertical([
            Constraint::Length(now_height),
            Constraint::Length(u16::from(area.height >= 12)),
            Constraint::Min(0),
        ])
        .split(area);
        playback(f, app, theme, rows[0], false);
        render_queue(f, rows[2], app, theme);
    }
}

pub(super) fn monitor(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(6),
        Constraint::Min(3),
        Constraint::Length(8),
    ])
    .split(area);
    playback(f, app, theme, rows[0], false);
    super::player::render_visualizer(f, app, theme, rows[1]);
    render_queue(f, rows[2], app, theme);
}

fn playback(f: &mut Frame, app: &App, theme: &Theme, area: Rect, desk: bool) {
    f.render_widget(Block::default().bg(theme.surface()), area);
    let inner = padded(
        area,
        if area.width >= 36 { 2 } else { 1 },
        u16::from(area.height >= 8),
    );
    if inner.is_empty() {
        return;
    }
    let Some(track) = app.current_track.as_ref() else {
        let state = if app.queue_request.is_some() {
            "Loading playback…"
        } else if app.queue_error.is_some() {
            "Playback unavailable"
        } else if app.last_queue_received.is_none() {
            "Waiting for playback"
        } else {
            "No track playing"
        };
        f.render_widget(
            Paragraph::new(state).fg(theme.text()).add_modifier(
                if app.focus == PlayerFocus::Playback {
                    Modifier::BOLD | Modifier::UNDERLINED
                } else {
                    Modifier::empty()
                },
            ),
            row(inner, 0),
        );
        text(
            f,
            row(inner, 1),
            "Press a to add music",
            theme.text_secondary,
        );
        return;
    };
    let (state, color) = if app.playback_is_stale() {
        ("Stale · r refresh", theme.warning())
    } else if !app.has_playback_state {
        ("State unavailable", theme.text_secondary)
    } else if app.paused {
        ("Paused", theme.warning())
    } else {
        ("Playing", theme.success())
    };
    f.render_widget(
        Paragraph::new(state)
            .fg(color)
            .add_modifier(if app.focus == PlayerFocus::Playback {
                Modifier::BOLD | Modifier::UNDERLINED
            } else {
                Modifier::empty()
            }),
        row(inner, 0),
    );
    f.render_widget(
        Paragraph::new(fit_text(&single_line(&track.title, app), inner.width))
            .bold()
            .fg(theme.text()),
        row(inner, 1),
    );
    text(
        f,
        row(inner, 2),
        fit_text(&single_line(&track.artist, app), inner.width),
        theme.text_secondary,
    );
    if desk && inner.height >= 6 {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Album  ", Style::default().fg(theme.text_secondary)),
                Span::styled(
                    fit_text(
                        track.album.as_deref().unwrap_or("Unknown album"),
                        inner.width.saturating_sub(7),
                    ),
                    Style::default().fg(theme.text()),
                ),
            ])),
            row(inner, 4),
        );
    } else if inner.height >= 5 {
        text(
            f,
            row(inner, 3),
            fit_text(
                track.album.as_deref().unwrap_or("Unknown album"),
                inner.width,
            ),
            theme.text_secondary,
        );
    }
    if inner.height < 4 {
        return;
    }
    let duration = track
        .duration_ms
        .or((app.duration_ms > 0).then_some(app.duration_ms));
    let timing_y = if desk && inner.height >= 6 {
        5
    } else {
        inner.height - 1
    };
    let timing = Layout::horizontal([
        Constraint::Length(6),
        Constraint::Min(0),
        Constraint::Length(6),
    ])
    .split(row(inner, timing_y));
    text(
        f,
        timing[0],
        if app.has_playback_state {
            time_label(app.elapsed_ms)
        } else {
            "--:--".into()
        },
        theme.text_secondary,
    );
    f.render_widget(
        Paragraph::new(duration.map(time_label).unwrap_or_else(|| "--:--".into()))
            .alignment(Alignment::Right)
            .fg(theme.text_secondary),
        timing[2],
    );
    let ratio = if app.has_playback_state {
        duration
            .filter(|value| *value > 0)
            .map(|value| app.elapsed_ms as f64 / value as f64)
            .unwrap_or(0.0)
    } else {
        0.0
    };
    f.render_widget(
        LineGauge::default()
            .ratio(ratio.clamp(0.0, 1.0))
            .label("")
            .filled_style(Style::default().fg(theme.primary))
            .unfilled_style(Style::default().fg(theme.border)),
        timing[1],
    );
}

fn session(f: &mut Frame, app: &App, theme: &Theme, area: Rect, spacious: bool) {
    f.render_widget(Block::default().bg(theme.raised()), area);
    let inner = padded(
        area,
        if area.width >= 36 { 2 } else { 1 },
        u16::from(spacious),
    );
    let host = Url::parse(&app.base_url)
        .map(|url| url[url::Position::BeforeHost..url::Position::AfterPort].to_owned())
        .unwrap_or_default();
    let updated = app
        .latest_received()
        .map(|at| {
            if at.elapsed().as_secs() < 2 {
                "Just now".into()
            } else {
                format!("{}s ago", at.elapsed().as_secs())
            }
        })
        .unwrap_or_else(|| "Waiting".into());
    let fields = [
        ("Server", host),
        (
            "Guild",
            app.guild_id
                .clone()
                .unwrap_or_else(|| "Not selected".into()),
        ),
        (
            "Transport",
            if app.ws_connected {
                "WebSocket".into()
            } else {
                "HTTP".into()
            },
        ),
        ("Updated", updated),
        (
            "Repeat",
            app.pending_repeat()
                .map(|mode| format!("{} → {mode}…", app.repeat_label()))
                .unwrap_or_else(|| app.repeat_label().to_string()),
        ),
    ];
    let mut lines = vec![Line::from("Session").fg(theme.highlight)];
    if spacious {
        lines.push(Line::from(""));
    }
    lines.extend(
        fields
            .into_iter()
            .filter(|(label, _)| spacious || !matches!(*label, "Transport" | "Updated"))
            .map(|(label, value)| {
                Line::from(vec![
                    Span::styled(
                        format!("{label:<11}"),
                        Style::default().fg(theme.text_secondary),
                    ),
                    Span::styled(
                        fit_text(&single_line(&value, app), inner.width.saturating_sub(11)),
                        Style::default().fg(theme.text()),
                    ),
                ])
            }),
    );
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_queue(f: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    f.render_widget(Block::default().bg(theme.tracklist()), area);
    let area = padded(
        area,
        if area.width >= 64 { 2 } else { 1 },
        u16::from(area.height >= 8),
    );
    if area.height < 3 {
        return;
    }
    app.clamp_queue_selection();
    let count = app.queue.len();
    let title = format!(
        "Queue   {count} loaded{}",
        if app.queue_error.is_some() || app.queue_is_stale() {
            "   Cached"
        } else {
            ""
        }
    );
    f.render_widget(
        Paragraph::new(title).fg(theme.highlight).add_modifier(
            if app.focus == PlayerFocus::Queue {
                Modifier::BOLD | Modifier::UNDERLINED
            } else {
                Modifier::empty()
            },
        ),
        row(area, 0),
    );
    let compact = area.width < 64;
    f.render_widget(
        Block::default().bg(theme.table_header()),
        Rect::new(area.x, area.y + 1, area.width, if compact { 2 } else { 1 }),
    );
    let columns = if compact {
        Layout::horizontal([
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Length(6),
        ])
        .split(row(area, 1))
    } else {
        Layout::horizontal([
            Constraint::Length(6),
            Constraint::Fill(4),
            Constraint::Fill(3),
            Constraint::Fill(3),
            Constraint::Length(6),
        ])
        .split(row(area, 1))
    };
    let secondary = Layout::horizontal([
        Constraint::Length(6),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ])
    .split(row(area, 2));
    text(f, columns[0], "   #", theme.text());
    text(f, columns[1], "TITLE", theme.text());
    if compact {
        text(f, columns[2], "LENGTH", theme.text());
        text(f, secondary[1], "ARTIST", theme.text());
        text(f, secondary[2], "ALBUM", theme.text());
    } else {
        text(f, columns[2], "ARTIST", theme.text());
        text(f, columns[3], "ALBUM", theme.text());
        text(f, columns[4], "LENGTH", theme.text());
    }
    let header_rows = if compact { 3 } else { 2 };
    let item_height = if compact { 2 } else { 1 };
    let height = area.height.saturating_sub(header_rows) / item_height * item_height;
    let body = Rect::new(area.x, area.y + header_rows, area.width, height);
    app.queue_page_size = usize::from(body.height / item_height).max(1);
    if count == 0 {
        text(
            f,
            row(body, 0),
            if app.queue_request.is_some() {
                "Loading queue…"
            } else if app.queue_error.is_some() {
                "Queue unavailable. Press r to retry; 3 for Settings."
            } else if app.last_queue_received.is_none() {
                "Queue not loaded. Press r to refresh."
            } else {
                "Queue empty. Press a to add a song name or URL."
            },
            theme.text(),
        );
        return;
    }
    // Widths include the two-cell selection marker; all metadata clips by cells.
    let items: Vec<_> = app
        .queue
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let length = track
                .duration_ms
                .map(time_label)
                .unwrap_or_else(|| "—".into());
            let mut first = vec![
                Span::styled(
                    format!(
                        "{:>2}  ",
                        app.queue_offset.saturating_add(i).saturating_add(1)
                    ),
                    Style::default().fg(theme.text_secondary),
                ),
                queue_column(&track.title, columns[1].width, theme.text()),
            ];
            let style = Style::default().bg(if i % 2 == 0 {
                theme.tracklist()
            } else {
                theme.alternate_row()
            });
            if compact {
                first.push(queue_column(&length, columns[2].width, theme.length()));
                ListItem::new(vec![
                    Line::from(first),
                    Line::from(vec![
                        Span::raw("    "),
                        queue_column(&track.artist, secondary[1].width, theme.artist()),
                        queue_column(
                            track.album.as_deref().unwrap_or("Unknown album"),
                            secondary[2].width,
                            theme.album(),
                        ),
                    ]),
                ])
                .style(style)
            } else {
                first.extend([
                    queue_column(&track.artist, columns[2].width, theme.artist()),
                    queue_column(
                        track.album.as_deref().unwrap_or("Unknown album"),
                        columns[3].width,
                        theme.album(),
                    ),
                    queue_column(&length, columns[4].width, theme.length()),
                ]);
                ListItem::new(Line::from(first)).style(style)
            }
        })
        .collect();
    f.render_stateful_widget(
        List::new(items).highlight_symbol("> ").highlight_style(
            Style::default()
                .fg(theme.highlight)
                .bg(theme.selection())
                .add_modifier(Modifier::BOLD),
        ),
        body,
        &mut app.queue_state,
    );
}

fn queue_column(value: &str, width: u16, color: Color) -> Span<'static> {
    let value = fit_text(value, width.saturating_sub(2));
    let padding = usize::from(width).saturating_sub(Line::from(value.as_str()).width());
    Span::styled(
        format!("{value}{}", " ".repeat(padding)),
        Style::default().fg(color),
    )
}
