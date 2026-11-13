use super::design::{fit_text, key_hint, panel, prepare_modal, single_line};
use super::*;

pub(super) fn render_header(f: &mut Frame, app: &App, theme: &Theme, view: View, area: Rect) {
    f.render_widget(Block::default().bg(theme.chrome()), area);
    let area = super::design::padded(area, 2.min(area.width / 4), 0);
    if area.is_empty() {
        return;
    }
    let (connection, color) = if app.token.is_none() {
        ("Sign in", theme.text_secondary)
    } else if app.ws_connected {
        ("Connected", theme.success())
    } else if app.ws_connecting {
        ("Connecting", theme.warning())
    } else {
        ("HTTP mode", theme.text_secondary)
    };
    let columns = Layout::horizontal([
        Constraint::Length(9),
        Constraint::Min(0),
        Constraint::Length(12),
    ])
    .split(Rect::new(area.x, area.y, area.width, 1));
    f.render_widget(Paragraph::new("JORIK").fg(theme.text()).bold(), columns[0]);
    if !(view == View::Main && app.layout == "Desk" && area.width >= 74) {
        let host = Url::parse(&app.base_url)
            .map(|url| url[url::Position::BeforeHost..url::Position::AfterPort].to_owned())
            .unwrap_or_default();
        let context = match &app.guild_id {
            Some(guild) => format!("{host}   Guild {guild}"),
            None => host,
        };
        f.render_widget(
            Paragraph::new(fit_text(&single_line(&context, app), columns[1].width))
                .fg(theme.text_secondary),
            columns[1],
        );
    }
    f.render_widget(
        Paragraph::new(connection)
            .alignment(Alignment::Right)
            .fg(color),
        columns[2],
    );
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
                    theme.highlight
                } else {
                    theme.text_secondary
                })
                .bg(if active {
                    theme.selection()
                } else {
                    theme.chrome()
                })
                .add_modifier(if active && app.monochrome {
                    Modifier::BOLD | Modifier::UNDERLINED
                } else {
                    Modifier::empty()
                }),
        ));
    }
    if area.height > 1 {
        f.render_widget(
            Paragraph::new(Line::from(nav)),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
    }
}

pub(super) fn render_footer(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    f.render_widget(Block::default().bg(theme.chrome()), area);
    let area = super::design::padded(area, 2.min(area.width / 4), 0);
    let error = app.error_message.as_ref().or(app.queue_error.as_ref());
    let (badge, status, color) = if let Some(error) = error {
        (
            " ! ",
            format!(
                "{}Esc dismiss · {error}",
                if app.queue_error.is_some() {
                    "r retry · "
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
    } else if !app.pending_commands.is_empty() || !app.rest_commands.is_empty() {
        (
            " … ",
            format!(
                "Waiting for {} command confirmation(s)…",
                app.pending_commands.len() + app.rest_commands.len()
            ),
            theme.highlight,
        )
    } else if app.is_busy() {
        (
            " … ",
            "Fetching data… you can keep navigating".into(),
            theme.highlight,
        )
    } else if app.view == View::Main && app.playback_is_stale() {
        (
            " ! ",
            "Playback state is stale · r refresh".into(),
            theme.warning(),
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
        let status = if app.queue.is_empty() {
            "Queue empty".into()
        } else {
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
            let total = app
                .queue_total
                .map(|total| format!("   {total} in queue"))
                .unwrap_or_else(|| "   Total unavailable".into());
            if app.queue_offset > 0 {
                format!("Showing {first}–{last}{total}   [ ] queue pages")
            } else {
                format!(
                    "Showing {first}–{last} of {} loaded tracks{total}",
                    app.queue.len()
                )
            }
        };
        (" · ", status, theme.text_secondary)
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
        View::TrackDetails => &[("Esc", "back to queue")],
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
            ("a", "add music"),
            (":", "actions"),
            ("?", "help"),
            ("q", "quit"),
        ],
        _ if area.width < 64 => &[("/", "add"), (":", "actions"), ("?", "help"), ("q", "quit")],
        _ => &[
            ("Space", playback_hint),
            ("a", "add"),
            ("Enter", "details"),
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
        "a or /          Add music: song name or URL",
        "Enter           Inspect the selected track",
        "s / w / l       Skip / stop / cycle loop mode",
        "c               Clear upcoming tracks (asks first)",
        "r               Refresh queue",
        "v               Show / hide visualization",
        "",
        "QUEUE",
        "Tab / Shift+Tab Move focus between panels",
        "↑↓ / j k        Browse loaded tracks",
        "PageUp/PageDown Browse one screen of loaded tracks",
        "Home / End      First / last loaded track",
        "[ / ]           Previous / next server queue page",
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
                app.queue_total
                    .map(|total| format!("Remove all {total} upcoming tracks?"))
                    .unwrap_or_else(|| "Remove all upcoming tracks? Total unavailable.".into()),
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
