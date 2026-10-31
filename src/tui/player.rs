use super::*;

pub(super) fn render_player_ui(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let zen = app.layout == "Zen"
        && app.focus == PlayerFocus::Playback
        && app.visualizer_enabled
        && area.width >= 80
        && area.height >= 16;
    let now_height = if area.height >= 10 {
        6
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
                Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)])
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
    let state = if app.current_track.is_none() {
        "Idle"
    } else if !app.has_playback_state {
        "State unavailable"
    } else if app.paused {
        "Paused"
    } else {
        "Playing"
    };
    let block = Block::bordered()
        .border_style(Style::default().fg(if focused {
            theme.highlight
        } else {
            theme.border
        }))
        .title(format!(" Now playing · {state} · Loop {} ", app.loop_mode))
        .title_style(Style::default().fg(theme.highlight))
        .padding(ratatui::widgets::Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if let Some(track) = &app.current_track {
        let (title, artist) = track
            .rsplit_once(" - ")
            .unwrap_or((track, "Unknown artist"));
        let info = Paragraph::new(vec![
            Line::from(Span::styled(
                title,
                Style::default().fg(Color::White).bold(),
            )),
            Line::from(Span::styled(
                artist,
                Style::default().fg(theme.text_secondary),
            )),
        ]);
        f.render_widget(info, inner);
        if inner.height >= 3 {
            let label = if app.duration_ms > 0 {
                format!(
                    "{:02}:{:02} / {:02}:{:02}",
                    app.elapsed_ms / 60000,
                    (app.elapsed_ms / 1000) % 60,
                    app.duration_ms / 60000,
                    (app.duration_ms / 1000) % 60
                )
            } else {
                "Live / duration unavailable".to_string()
            };
            let ratio = if app.duration_ms > 0 {
                (app.elapsed_ms as f64 / app.duration_ms as f64).clamp(0.0, 1.0)
            } else {
                0.0
            };
            f.render_widget(
                Gauge::default()
                    .ratio(ratio)
                    .label(label)
                    .gauge_style(Style::default().fg(theme.primary).bg(theme.bg)),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
    } else {
        f.render_widget(
            Paragraph::new(vec![
                Line::from("Nothing is playing"),
                Line::from(Span::styled(
                    "/ Add a track or URL · Join a voice channel to start",
                    Style::default().fg(theme.text_secondary),
                )),
            ]),
            inner,
        );
    }
}

fn render_queue(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    app.clamp_queue_selection();
    let focused = app.focus == PlayerFocus::Queue;
    let selected = app.queue_state.selected().map_or(0, |i| i + 1);
    let block = Block::bordered()
        .border_style(Style::default().fg(if focused {
            theme.highlight
        } else {
            theme.border
        }))
        .title(format!(
            " Queue · {} tracks · {selected}/{} ",
            app.queue.len(),
            app.queue.len()
        ))
        .title_style(Style::default().fg(theme.highlight))
        .padding(ratatui::widgets::Padding::horizontal(1));
    app.queue_page_size = usize::from(block.inner(area).height).max(1);
    if app.queue.is_empty() {
        f.render_widget(
            Paragraph::new("Queue is empty. Press / to add a track.")
                .style(Style::default().fg(theme.text_secondary))
                .block(block),
            area,
        );
        return;
    }
    let items = app
        .queue
        .iter()
        .enumerate()
        .map(|(i, track)| ListItem::new(format!(" {:>2}  {track}", i + 1)))
        .collect::<Vec<_>>();
    let selected_style = if focused {
        Style::default().bg(theme.primary).fg(Color::Black).bold()
    } else {
        Style::default().fg(theme.highlight)
    };
    let list = List::new(items)
        .block(block)
        .style(Style::default().fg(theme.text_secondary))
        .highlight_style(selected_style)
        .highlight_symbol("› ");
    f.render_stateful_widget(list, area, &mut app.queue_state);
}

fn render_visualizer(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let spec_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme.border))
        .title(" Visualizer ")
        .title_style(
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        );

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
                for i in 0..64 {
                    let overlap = ((i + 1) as f32).min(end_f) - (i as f32).max(start_f);
                    if overlap > 0.0 {
                        sum += app.smoothed_bars[i] * overlap;
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

                let label = if app.viz_style == "Line" || app.viz_style == "Wave" {
                    String::new()
                } else {
                    format!("{:2}", v.min(99))
                };

                Bar::default()
                    .value(v)
                    .label(Span::from(label))
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
            Paragraph::new("Visualization unavailable")
                .block(spec_block)
                .alignment(Alignment::Center),
            area,
        );
    }
}
