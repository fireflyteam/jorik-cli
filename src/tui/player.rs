use super::design::{fit_text, key_hint, panel, single_line, time_label};
use super::*;

pub(super) fn render_player_ui(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    if matches!(app.layout.as_str(), "Desk" | "Queue")
        && app.visualizer_enabled
        && area.height >= 12
    {
        super::desk::monitor(f, app, theme, area);
        return;
    }
    if matches!(app.layout.as_str(), "Desk" | "Queue") && !app.visualizer_enabled {
        super::desk::render(f, app, theme, area);
        return;
    }
    let zen = app.layout == "Zen"
        && app.focus == PlayerFocus::Playback
        && app.visualizer_enabled
        && area.width >= 76
        && area.height >= 16;
    let now_height = if area.height >= 15 {
        8
    } else if area.height >= 9 {
        5
    } else {
        3.min(area.height)
    };
    let rows = Layout::vertical([Constraint::Length(now_height), Constraint::Min(0)]).split(area);
    render_now_playing(f, app, theme, rows[0]);
    let content = rows[1];
    if zen {
        render_visualizer(f, app, theme, content);
    } else if app.visualizer_enabled && area.width >= 110 && area.height >= 22 {
        if app.layout == "Studio" {
            let rows = Layout::vertical([Constraint::Min(6), Constraint::Length(7)]).split(content);
            render_queue(f, app, theme, rows[0]);
            render_visualizer(f, app, theme, rows[1]);
        } else {
            let columns =
                Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
                    .split(content);
            if app.layout == "Sidebar" {
                render_visualizer(f, app, theme, columns[0]);
                render_queue(f, app, theme, columns[1]);
            } else {
                render_queue(f, app, theme, columns[0]);
                render_visualizer(f, app, theme, columns[1]);
            }
        }
    } else {
        render_queue(f, app, theme, content);
    }
}

fn render_now_playing(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let focused = app.focus == PlayerFocus::Playback;
    let block = panel(" Now playing ", theme, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    if let Some(track) = &app.current_track {
        let (title, artist) = (track.title.as_str(), track.artist.as_str());
        let (state, color) = if app.playback_is_stale() {
            ("! Stale · r refresh", theme.warning())
        } else if !app.has_playback_state {
            ("○ State unavailable", theme.text_secondary)
        } else if app.paused {
            ("Ⅱ Paused", theme.warning())
        } else {
            ("● Playing", theme.success())
        };
        let compact = inner.height < 5;
        let mut lines = Vec::new();
        if !compact {
            lines.push(Line::from(vec![
                Span::styled(state, Style::default().fg(color).bold()),
                Span::styled(
                    format!(
                        "   ·   Loop {}{}",
                        app.repeat_label(),
                        app.pending_repeat()
                            .map(|mode| format!(" → {mode}…"))
                            .unwrap_or_default()
                    ),
                    Style::default().fg(theme.text_secondary),
                ),
            ]));
        }
        lines.push(Line::from(Span::styled(
            fit_text(&single_line(title, app), inner.width),
            Style::default().fg(theme.text()).bold(),
        )));
        if inner.height > 1 {
            lines.push(Line::from(Span::styled(
                fit_text(&single_line(artist, app), inner.width),
                Style::default().fg(theme.text_secondary),
            )));
        }
        f.render_widget(Paragraph::new(lines), inner);
        if inner.height >= 3 {
            let progress_y = if compact {
                inner.bottom() - 1
            } else {
                inner.bottom() - 2
            };
            let elapsed = if app.has_playback_state {
                time_label(app.elapsed_ms)
            } else {
                "--:--".to_string()
            };
            let duration = if app.has_playback_state && app.duration_ms > 0 {
                time_label(app.duration_ms)
            } else {
                "--:--".to_string()
            };
            let columns = Layout::horizontal([
                Constraint::Length(7),
                Constraint::Min(0),
                Constraint::Length(8),
            ])
            .split(Rect::new(inner.x, progress_y, inner.width, 1));
            f.render_widget(
                Paragraph::new(elapsed).style(Style::default().fg(theme.text_secondary)),
                columns[0],
            );
            let ratio = if app.has_playback_state && app.duration_ms > 0 {
                (app.elapsed_ms as f64 / app.duration_ms as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            f.render_widget(
                LineGauge::default()
                    .ratio(ratio)
                    .label("")
                    .filled_style(Style::default().fg(theme.primary))
                    .unfilled_style(Style::default().fg(theme.border))
                    .filled_symbol("━")
                    .unfilled_symbol("─"),
                columns[1],
            );
            f.render_widget(
                Paragraph::new(duration)
                    .alignment(Alignment::Right)
                    .style(Style::default().fg(theme.text_secondary)),
                columns[2],
            );
        }
        if !compact {
            let label = if !app.has_playback_state {
                "pause/resume"
            } else if app.paused {
                "resume"
            } else {
                "pause"
            };
            let mut controls = key_hint("Space", label, theme);
            controls.extend(key_hint("s", "skip", theme));
            controls.extend(key_hint("w", "stop", theme));
            f.render_widget(
                Paragraph::new(Line::from(controls)),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
    } else {
        let mut lines = vec![
            Line::from(Span::styled(
                "Make room for your next favorite.",
                Style::default().fg(theme.text()).bold(),
            )),
            Line::from(Span::styled(
                "Join a voice channel, then add a track or paste a URL.",
                Style::default().fg(theme.text_secondary),
            )),
        ];
        if inner.height >= 5 {
            lines.insert(
                0,
                Line::from(Span::styled(
                    "○ Ready to play",
                    Style::default().fg(theme.highlight),
                )),
            );
            lines.push(Line::from(""));
            lines.push(Line::from(key_hint("/", "add music", theme)));
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

fn render_queue(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    app.clamp_queue_selection();
    let focused = app.focus == PlayerFocus::Queue;
    let title = format!(
        " Queue · {} tracks{} ",
        app.queue.len(),
        if app.queue_error.is_some() || app.queue_is_stale() {
            " · cached"
        } else {
            ""
        }
    );
    let block = panel(title, theme, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let has_labels = inner.height >= 5 && inner.width >= 40;
    let rows = Layout::vertical([
        Constraint::Length(u16::from(has_labels)),
        Constraint::Min(0),
        Constraint::Length(u16::from(inner.height >= 4)),
    ])
    .split(inner);
    app.queue_page_size = usize::from(rows[1].height).max(1);
    if app.queue.is_empty() {
        let (title, hint) = if app.queue_request.is_some() {
            ("Fetching your queue…", "Your music will appear here.")
        } else if app.queue_error.is_some() {
            (
                "Queue could not be loaded.",
                "Press r to retry. Existing playback is not interrupted.",
            )
        } else {
            ("Nothing up next. Yet.", "Press / to add a track or URL.")
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(title, Style::default().fg(theme.text()))),
                Line::from(Span::styled(
                    hint,
                    Style::default().fg(theme.text_secondary),
                )),
            ])
            .wrap(Wrap { trim: true }),
            rows[1],
        );
        return;
    }
    let artist_width = if rows[1].width >= 55 {
        (rows[1].width / 3).min(28)
    } else {
        0
    };
    let title_width = rows[1].width.saturating_sub(7 + artist_width);
    if has_labels {
        let header = if artist_width > 0 {
            format!(
                "   #   {:width$} ARTIST",
                "TITLE",
                width = usize::from(title_width)
            )
        } else {
            "   #   TITLE / ARTIST".to_string()
        };
        f.render_widget(
            Paragraph::new(header).style(Style::default().fg(theme.text_secondary)),
            rows[0],
        );
    }
    let items = app
        .queue
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let clean = single_line(&track.label(), app);
            let (title, artist) = (track.title.as_str(), track.artist.as_str());
            let title = fit_text(if artist_width == 0 { &clean } else { title }, title_width);
            let padding =
                usize::from(title_width).saturating_sub(Line::from(title.as_str()).width());
            let mut spans = vec![
                Span::styled(
                    format!(
                        "{:>3}  ",
                        app.queue_offset.saturating_add(i).saturating_add(1)
                    ),
                    Style::default().fg(theme.text_secondary),
                ),
                Span::styled(title, Style::default().fg(theme.text())),
            ];
            if artist_width > 0 {
                spans.push(Span::raw(" ".repeat(padding + 1)));
                spans.push(Span::styled(
                    fit_text(artist, artist_width.saturating_sub(1)),
                    Style::default().fg(theme.text_secondary),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect::<Vec<_>>();
    let list = List::new(items).highlight_symbol("▎ ").highlight_style(
        Style::default()
            .bg(if focused {
                theme.selection()
            } else {
                theme.raised()
            })
            .fg(theme.highlight)
            .bold(),
    );
    f.render_stateful_widget(list, rows[1], &mut app.queue_state);
    let first = app
        .queue_offset
        .saturating_add(app.queue_state.offset())
        .saturating_add(1);
    let last = app.queue_offset.saturating_add(
        app.queue_state
            .offset()
            .saturating_add(app.queue_page_size)
            .min(app.queue.len()),
    );
    let label = format!(
        "{first}–{last} of {} loaded  ·  {}",
        app.queue.len(),
        if focused {
            "↑↓ browse · Enter details"
        } else {
            "Tab to focus"
        }
    );
    f.render_widget(
        Paragraph::new(label).style(Style::default().fg(theme.text_secondary)),
        rows[2],
    );
}

pub(super) fn render_visualizer(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    app.visualizer_visible = area.width >= 4 && area.height >= 4;
    let spec_block = panel(" Visualizer · spectrum ", theme, false);

    if app.current_track.is_some() && app.spectrogram.is_some() {
        let (b_w, b_g) = match app.viz_style.as_str() {
            "Blocky" => ((spec_block.inner(area).width / 64).max(1), 0),
            "Line" => (1, 0),
            "Wave" => (1, 0),
            "Dots" => (1, 1),
            _ => (2, 1),
        };

        let num_bars = if app.viz_style == "Wave" || app.viz_style == "Dots" {
            (spec_block.inner(area).width as usize).min(128)
        } else {
            ((spec_block.inner(area).width / (b_w + b_g)) as usize).min(64)
        };

        let mut bar_items = Vec::with_capacity(num_bars);

        if num_bars > 0 {
            let start_bin = 3.0;
            let end_bin = 61.0;
            let bins_to_show = end_bin - start_bin;
            let bins_per_bar = bins_to_show / num_bars as f32;

            for j in 0..num_bars {
                let start_f = start_bin + j as f32 * bins_per_bar;
                let end_f = start_bin + (j + 1) as f32 * bins_per_bar;
                let mut sum = 0.0;
                let mut weight = 0.0;
                for (i, &amplitude) in app.smoothed_bars.iter().enumerate().take(64) {
                    let overlap = ((i + 1) as f32).min(end_f) - (i as f32).max(start_f);
                    if overlap > 0.0 {
                        sum += amplitude * overlap;
                        weight += overlap;
                    }
                }
                bar_items.push((if weight > 0.0 { sum / weight } else { 0.0 }) as u64);
            }
        }

        let bars: Vec<Bar> = bar_items
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let color = match app.viz_style.as_str() {
                    "Blocky" | "Wave" => {
                        if i < num_bars / 3 {
                            theme.primary
                        } else if i < 2 * num_bars / 3 {
                            theme.highlight
                        } else {
                            Color::Rgb(200, 200, 255)
                        }
                    }
                    "Line" => theme.highlight,
                    _ => {
                        // Bars (Gradient)
                        if i < num_bars / 4 {
                            theme.primary
                        } else if i < num_bars / 2 {
                            theme.highlight
                        } else {
                            Color::Rgb(200, 200, 255)
                        }
                    }
                };

                Bar::default()
                    .value(v)
                    .label(Span::raw(""))
                    .style(Style::default().fg(color))
                    .text_value(String::new())
            })
            .collect();

        let bar_group = BarGroup::default().bars(&bars);
        let spec_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(spec_block.inner(area));

        let barchart = BarChart::default()
            .data(bar_group)
            .bar_width(b_w.max(1))
            .bar_gap(b_g)
            .max(100)
            .label_style(Style::default().fg(theme.text_secondary));

        f.render_widget(spec_block, area);
        f.render_widget(barchart, spec_chunks[0]);

        if app.viz_style != "Wave" && app.viz_style != "Dots" {
            let labels = ["40", "100", "500", "1k", "5k", "10k", "16k"];
            let mut label_spans = Vec::new();
            let total_w = spec_chunks[1].width as usize;
            if total_w > 10 {
                for (i, &l) in labels.iter().enumerate() {
                    let pos = (i as f32 / (labels.len() - 1) as f32 * (total_w - l.len()) as f32)
                        as usize;
                    let current_len: usize =
                        label_spans.iter().map(|s: &Span| s.content.len()).sum();
                    if pos > current_len {
                        label_spans.push(Span::raw(" ".repeat(pos - current_len)));
                    }
                    label_spans.push(Span::styled(l, Style::default().fg(theme.text_secondary)));
                }
                f.render_widget(Paragraph::new(Line::from(label_spans)), spec_chunks[1]);
            }
        }
    } else {
        f.render_widget(
            Paragraph::new("No spectrum data\nPlayback works without visualization.")
                .style(Style::default().fg(theme.text_secondary))
                .block(spec_block)
                .alignment(Alignment::Center),
            area,
        );
    }
}
