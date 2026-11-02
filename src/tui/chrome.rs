use super::design::{fit_text, key_hint, panel, prepare_modal, single_line};
use super::*;

pub(super) fn render_header(f: &mut Frame, app: &App, theme: &Theme, view: View, area: Rect) {
    let narrow = area.width < 64;
    let (connection, color) = if app.token.is_none() {
        ("○ Sign in", theme.text_secondary)
    } else if app.ws_connected {
        ("● Live", theme.success())
    } else if app.ws_connecting {
        ("◌ Connecting", theme.warning())
    } else {
        ("○ HTTP mode", theme.text_secondary)
    };
    let row = Rect::new(area.x, area.y, area.width, 1.min(area.height));
    let columns = Layout::horizontal([
        Constraint::Length(10),
        Constraint::Min(0),
        Constraint::Length(15),
    ])
    .split(row);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" J", Style::default().fg(theme.highlight).bold()),
            Span::styled("ORIK ", Style::default().fg(theme.text()).bold()),
        ])),
        columns[0],
    );
    f.render_widget(
        Paragraph::new(format!("{connection} "))
            .alignment(Alignment::Right)
            .style(Style::default().fg(color)),
        columns[2],
    );
    let nav_area = if narrow {
        Rect::new(
            area.x,
            area.y.saturating_add(2),
            area.width,
            area.height.saturating_sub(2).min(1),
        )
    } else {
        columns[1]
    };
    let mut nav = Vec::new();
    for (tab, label) in [
        (View::Main, "1 Player"),
        (View::Lyrics, "2 Lyrics"),
        (View::Settings, "3 Settings"),
        (View::Debug, "4 Logs"),
    ] {
        let active = view == tab;
        nav.push(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(if active {
                    theme.text()
                } else {
                    theme.text_secondary
                })
                .bg(if active { theme.selection() } else { theme.bg }),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(nav)), nav_area);
    if area.height > 1 {
        let host = Url::parse(&app.base_url)
            .map(|url| url[url::Position::BeforeHost..url::Position::AfterPort].to_string())
            .unwrap_or_else(|_| "Invalid server".into());
        let context = if let Some(guild) = &app.guild_id {
            format!(" {host}  /  Guild {guild}")
        } else {
            format!(" {host}  /  Join a voice channel to begin")
        };
        f.render_widget(
            Paragraph::new(fit_text(&single_line(&context, app), area.width))
                .style(Style::default().fg(theme.text_secondary)),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
    }
    if !narrow && area.height > 2 {
        f.render_widget(
            Paragraph::new("─".repeat(usize::from(area.width)))
                .style(Style::default().fg(theme.border)),
            Rect::new(area.x, area.y + 2, area.width, 1),
        );
    }
}

pub(super) fn render_footer(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let error = app.error_message.as_ref().or(app.queue_error.as_ref());
    let (badge, status, color) = if let Some(error) = error {
        (
            " ! ",
            format!(
                "{error} · Esc dismiss{}",
                if app.queue_error.is_some() {
                    " · r retry"
                } else {
                    ""
                }
            ),
            theme.danger(),
        )
    } else if let Some((notice, _)) = &app.notice {
        (" ✓ ", notice.clone(), theme.success())
    } else if app.update_notice {
        (
            " ↑ ",
            format!(
                "Update {} available · u review · Esc dismiss",
                app.update_info.as_ref().map_or("", |(v, _)| v.as_str())
            ),
            theme.highlight,
        )
    } else if !app.pending_commands.is_empty() {
        (
            " … ",
            format!(
                "Waiting for {} command confirmation(s)…",
                app.pending_commands.len()
            ),
            theme.highlight,
        )
    } else if app.is_busy() {
        (
            " … ",
            "Fetching data… you can keep navigating".into(),
            theme.highlight,
        )
    } else if app.view == View::Settings {
        (
            " · ",
            if app.is_settings_editing {
                "Draft changes · Enter validates and saves · Esc discards"
            } else {
                "Preferences · appearance saves immediately · server changes stay isolated"
            }
            .into(),
            theme.text_secondary,
        )
    } else if app.view == View::Lyrics {
        (
            " · ",
            format!(
                "Lyrics · scroll position {} of {}",
                app.lyrics_scroll, app.lyrics_max_scroll
            ),
            theme.text_secondary,
        )
    } else if app.view == View::Debug {
        (
            " · ",
            "Diagnostics · credentials are redacted · s saves spectrum data".into(),
            theme.text_secondary,
        )
    } else {
        let focus = if app.focus == PlayerFocus::Queue {
            "Queue"
        } else {
            "Playback"
        };
        (
            " · ",
            format!("{focus} focus · Tab switches panels · Queue selection is browsing only"),
            theme.text_secondary,
        )
    };
    let badge_width = 3.min(area.width);
    let status = fit_text(
        &single_line(&status, app),
        area.width.saturating_sub(badge_width),
    );
    let status_line = Line::from(vec![
        Span::styled(badge, Style::default().fg(color).bg(theme.raised()).bold()),
        Span::styled(status, Style::default().fg(color)),
    ]);
    let playback_hint = if !app.has_playback_state {
        "toggle"
    } else if app.paused {
        "resume"
    } else {
        "pause"
    };
    let hints: &[(&str, &str)] = match app.view {
        View::Menu => &[("↑↓", "select"), ("Enter", "run"), ("Esc", "close")],
        View::Help => &[("↑↓", "scroll"), ("Esc", "close")],
        View::ConfirmClear => &[("y", "clear"), ("Esc", "keep queue")],
        View::Settings if app.is_settings_editing => &[
            ("Enter", "save"),
            ("Esc", "discard"),
            ("←→", "cursor"),
            ("Ctrl+u", "clear"),
        ],
        View::Settings => &[
            ("↑↓", "select"),
            ("←→", "change"),
            ("Enter", "edit/done"),
            ("Esc", "back"),
        ],
        View::Lyrics => &[
            ("↑↓", "scroll"),
            ("r", "refresh"),
            ("Esc", "player"),
            (":", "actions"),
        ],
        View::Debug => &[("s", "save spectrum"), ("Esc", "player"), (":", "actions")],
        View::AuthMenu | View::FilterMenu => {
            &[("↑↓", "select"), ("Enter", "choose"), ("Esc", "back")]
        }
        View::AuthResult | View::AppInfo => &[("Esc", "back"), ("q", "quit")],
        _ if app.input_mode == InputMode::Editing => {
            &[("Enter", "add"), ("Esc", "discard"), ("←→", "cursor")]
        }
        View::Main if app.current_track.is_none() => &[
            ("/", "add music"),
            (":", "actions"),
            ("?", "help"),
            ("q", "quit"),
        ],
        _ if area.width < 64 => &[("/", "add"), (":", "actions"), ("?", "help"), ("q", "quit")],
        _ => &[
            ("Space", playback_hint),
            ("/", "add"),
            ("Tab", "focus"),
            (":", "actions"),
            ("?", "help"),
            ("q", "quit"),
        ],
    };
    let keys = hints
        .iter()
        .flat_map(|(key, label)| key_hint(key, label, theme))
        .collect::<Vec<_>>();
    f.render_widget(Paragraph::new(vec![status_line, Line::from(keys)]), area);
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
    let area = popup_rect(76, 19, f.area());
    prepare_modal(f, area, theme);
    let block = panel(" Command palette ", theme, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(inner);
    let (query, cursor) = input_window(
        &app.palette_query,
        app.palette_query.len(),
        rows[0].width.saturating_sub(3),
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(" / ", Style::default().fg(theme.highlight).bold()),
                Span::styled(
                    if query.is_empty() {
                        "Search actions…"
                    } else {
                        query
                    },
                    Style::default().fg(if query.is_empty() {
                        theme.text_secondary
                    } else {
                        theme.text()
                    }),
                ),
            ]),
            Line::from(Span::styled(
                format!(" {} actions · type to filter", commands.len()),
                Style::default().fg(theme.text_secondary),
            )),
        ]),
        rows[0],
    );
    if commands.is_empty() {
        app.menu_state.select(None);
        f.render_widget(
            Paragraph::new("No matching actions. Try ‘queue’, ‘account’, or ‘help’.")
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.text_secondary)),
            rows[1],
        );
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
                let label = fit_text(command.label, rows[1].width.saturating_sub(18));
                let padding = usize::from(rows[1].width.saturating_sub(3))
                    .saturating_sub(Line::from(label.as_str()).width() + command.shortcut.len());
                ListItem::new(Line::from(vec![
                    Span::styled(
                        label,
                        Style::default().fg(if command.action == Action::Clear {
                            theme.danger()
                        } else {
                            theme.text()
                        }),
                    ),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(command.shortcut, Style::default().fg(theme.text_secondary)),
                ]))
            })
            .collect::<Vec<_>>();
        f.render_stateful_widget(
            List::new(items)
                .highlight_symbol("▎ ")
                .highlight_style(Style::default().bg(theme.selection()).bold()),
            rows[1],
            &mut app.menu_state,
        );
        f.render_widget(
            Paragraph::new(vec![
                Line::from(fit_text(commands[selected].description(), rows[2].width)),
                Line::from("↑↓ select · Enter run · Esc close"),
            ])
            .style(Style::default().fg(theme.text_secondary)),
            rows[2],
        );
    }
    if rows[0].height > 0 && rows[0].width > 3 {
        f.set_cursor_position((rows[0].x + 3 + cursor, rows[0].y));
    }
}

pub(super) fn render_help(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = popup_rect(76, 24, f.area());
    prepare_modal(f, area, theme);
    let block = panel(" Keyboard help · Esc closes ", theme, true);
    let inner = block.inner(area);
    let text = [
        "PLAYBACK",
        "Space / p       Pause or resume",
        "/ / Enter       Add music: search or paste a URL",
        "s / w / l       Skip / stop / cycle loop mode",
        "c               Clear upcoming tracks (asks first)",
        "r               Refresh queue",
        "v               Show / hide visualization",
        "",
        "QUEUE",
        "Tab / Shift+Tab Move focus between panels",
        "↑↓ / j k        Browse loaded tracks",
        "PageUp/PageDown Browse one page",
        "Home / End      First / last loaded track",
        "Selection does not play, remove, or reorder a track.",
        "",
        "NAVIGATION",
        "1 / 2 / 3 / 4   Player / lyrics / settings / diagnostics",
        ": / Ctrl+p      Search all actions",
        "u               Review an available update",
        "Esc             Close an overlay or dismiss a notice",
        "q / Ctrl+c      Quit (q is text while editing)",
        "",
        "EDITING",
        "←→ / Home / End Move cursor",
        "Ctrl+u          Clear input",
        "Enter / Esc     Submit / discard",
    ];
    let lines = text
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                *line,
                Style::default().fg(
                    if matches!(*line, "PLAYBACK" | "QUEUE" | "NAVIGATION" | "EDITING") {
                        theme.highlight
                    } else {
                        theme.text()
                    },
                ),
            ))
        })
        .collect::<Vec<_>>();
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    app.help_scroll = app.help_scroll.min(
        paragraph
            .line_count(inner.width)
            .saturating_sub(usize::from(inner.height))
            .min(usize::from(u16::MAX)) as u16,
    );
    f.render_widget(paragraph.block(block).scroll((app.help_scroll, 0)), area);
}

pub(super) fn render_clear_confirmation(f: &mut Frame, app: &App, theme: &Theme) {
    let area = popup_rect(58, 9, f.area());
    prepare_modal(f, area, theme);
    let block = panel(" Clear upcoming queue? ", theme, true)
        .border_style(Style::default().fg(theme.danger()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).split(inner);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                format!("Remove {} loaded upcoming track(s)?", app.queue.len()),
                Style::default().fg(theme.text()).bold(),
            )),
            Line::from(Span::styled(
                "This cannot be undone.",
                Style::default().fg(theme.text_secondary),
            )),
        ])
        .wrap(Wrap { trim: true }),
        rows[0],
    );
    let keys = if inner.width < 42 {
        vec![
            Line::from(key_hint("Esc", "keep queue", theme)),
            Line::from(key_hint("y", "clear queue", theme)),
        ]
    } else {
        vec![Line::from(
            [
                key_hint("Esc", "keep queue", theme),
                key_hint("y", "clear queue", theme),
            ]
            .concat(),
        )]
    };
    f.render_widget(Paragraph::new(keys), rows[1]);
}
