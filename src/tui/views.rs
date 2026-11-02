//! Secondary screens and dialogs share the same spacing, focus, and keyboard language.
use super::design::{fit_text, key_hint, panel, prepare_modal, single_line};
use super::*;

pub(super) fn welcome(f: &mut Frame, app: &App, theme: &Theme) {
    let area = popup_rect(76, 17, f.area());
    let block = panel(" JORIK ", theme, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 11 {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner);
        f.render_widget(
            Paragraph::new(fit_text(&single_line(&app.base_url, app), inner.width))
                .style(Style::default().fg(theme.text())),
            rows[0],
        );
        let status = app
            .auth_info_text
            .as_deref()
            .unwrap_or(if app.legacy_credentials {
                "Older login found. Sign in again, or press ? for migration help."
            } else {
                "Sign in with Discord, then join a voice channel to play music."
            });
        f.render_widget(
            Paragraph::new(crate::output::safe_text(status, app.token.as_deref()))
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.text())),
            rows[1],
        );
        f.render_widget(
            Paragraph::new(vec![
                super::design::primary_action(
                    if app.is_loading { "Esc" } else { "Enter" },
                    if app.is_loading {
                        "cancel login"
                    } else {
                        "sign in"
                    },
                    theme,
                ),
                Line::from(Span::styled(
                    "\\ server · q quit",
                    Style::default().fg(theme.text()),
                )),
            ]),
            rows[2],
        );
        return;
    }
    let show_legacy = app.legacy_credentials && !app.is_loading;
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(if show_legacy { 2 } else { 0 }),
        Constraint::Length(3),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Sign in to Jorik",
                Style::default().fg(theme.text()).bold(),
            )),
            Line::from(Span::styled(
                "Control music in your Discord voice channel.",
                Style::default().fg(theme.text()),
            )),
        ]),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Server",
                Style::default().fg(theme.text_secondary),
            )),
            Line::from(Span::styled(
                fit_text(&single_line(&app.base_url, app), rows[1].width),
                Style::default().fg(theme.text()),
            )),
        ]),
        rows[1],
    );
    if let Some(status) = &app.auth_info_text {
        f.render_widget(
            Paragraph::new(crate::output::safe_text(status, app.token.as_deref()))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(theme.text())),
            rows[2],
        );
    } else {
        let lines = [
            "1. Sign in with Discord",
            "",
            "2. Join a voice channel",
            "",
            "3. Add a song or paste a URL",
        ]
        .into_iter()
        .map(|line| Line::from(Span::styled(line, Style::default().fg(theme.text()).bold())))
        .collect::<Vec<_>>();
        f.render_widget(Paragraph::new(lines), rows[2]);
    }
    if show_legacy {
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "Older login files found.",
                    Style::default().fg(theme.warning()).bold(),
                )),
                Line::from(Span::styled(
                    "Sign in again, or press ? for migration options.",
                    Style::default().fg(theme.text()),
                )),
            ]),
            rows[3],
        );
    }
    let secondary = if app.is_loading {
        vec![("q", "quit")]
    } else {
        vec![("\\", "change server"), ("?", "help"), ("q", "quit")]
    };
    f.render_widget(
        Paragraph::new(vec![
            super::design::primary_action(
                if app.is_loading { "Esc" } else { "Enter" },
                if app.is_loading {
                    "Cancel login"
                } else {
                    "Sign in with Discord"
                },
                theme,
            ),
            Line::from(
                secondary
                    .iter()
                    .flat_map(|(key, label)| key_hint(key, label, theme))
                    .collect::<Vec<_>>(),
            ),
            Line::from(Span::styled(
                if app.is_loading {
                    "Waiting for your browser…"
                } else {
                    "Credentials are stored securely for this server."
                },
                Style::default().fg(theme.text_secondary),
            )),
        ]),
        rows[4],
    );
}

pub(super) fn sign_in_help(f: &mut Frame, app: &mut App, theme: &Theme) {
    let area = popup_rect(76, 20, f.area());
    prepare_modal(f, area, theme);
    let block = panel(
        if app.legacy_credentials {
            " Older login options "
        } else {
            " Sign-in help "
        },
        theme,
        true,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).split(inner);
    let text = if app.legacy_credentials {
        "Sign in again (recommended)\nPress Esc, then Enter for a fresh Discord login.\nYour old files are left untouched.\n\nKeep an existing login\nRun in a separate terminal:\njorik auth migrate --confirm-server SERVER_URL\nReplace SERVER_URL with the original issuing server.\nDo not infer the issuer from the currently selected server.\n\nRemove old login files\njorik auth clear-legacy --confirm\nPermanent deletion; this does not revoke the token remotely."
    } else {
        "Press Enter to open Discord authorization in your browser.\nAfter signing in, join a voice channel that Jorik can access.\n\nYour credentials are saved in the OS credential store and\nare used only for the selected server.\n\nThe backend must support the CLI 0.5 login protocol.\nIf your credential store is locked, unlock it and retry.\n\nFor headless use, provide JORIK_TOKEN through your\nenvironment or secret manager. It is not saved by Jorik."
    };
    let lines = text
        .lines()
        .map(|line| {
            Line::from(Span::styled(
                line,
                Style::default().fg(theme.text()).add_modifier(
                    if line.starts_with("jorik ")
                        || [
                            "Sign in again (recommended)",
                            "Keep an existing login",
                            "Remove old login files",
                        ]
                        .contains(&line)
                    {
                        ratatui::style::Modifier::BOLD
                    } else {
                        ratatui::style::Modifier::empty()
                    },
                ),
            ))
        })
        .collect::<Vec<_>>();
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    app.help_scroll = app.help_scroll.min(
        paragraph
            .line_count(rows[0].width)
            .saturating_sub(usize::from(rows[0].height))
            .min(usize::from(u16::MAX)) as u16,
    );
    f.render_widget(paragraph.scroll((app.help_scroll, 0)), rows[0]);
    f.render_widget(
        Paragraph::new(Line::from(
            [
                key_hint("↑↓", "scroll", theme),
                key_hint("Esc", "back", theme),
            ]
            .concat(),
        )),
        rows[1],
    );
}

pub(super) fn settings(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let columns = if area.width >= 110 {
        Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)]).split(area)
    } else {
        Layout::horizontal([Constraint::Percentage(100), Constraint::Length(0)]).split(area)
    };
    let block = panel(
        if app.is_settings_editing {
            " Settings · editing "
        } else {
            " Settings "
        },
        theme,
        true,
    );
    let inner = block.inner(columns[0]);
    f.render_widget(block, columns[0]);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new("Make this space yours.").style(Style::default().fg(theme.text()).bold()),
        rows[0],
    );
    let fields = [
        (
            SettingsField::Host,
            "Server",
            app.settings_input.clone(),
            "HTTPS required outside loopback development.",
        ),
        (
            SettingsField::Offset,
            "Spectrum offset",
            format!("{} ms", app.offset_input),
            "Synchronize the visualizer with what you hear.",
        ),
        (
            SettingsField::Theme,
            "Color theme",
            format!("‹ {} ›", app.theme),
            "Preview colors with ← →. Saved immediately.",
        ),
        (
            SettingsField::VizStyle,
            "Visualizer",
            format!("‹ {} ›", app.viz_style),
            "Choose the spectrum's visual style.",
        ),
        (
            SettingsField::Layout,
            "Layout",
            format!("‹ {} ›", app.layout),
            "Extra panels adapt to available terminal space.",
        ),
    ];
    let selected = fields
        .iter()
        .position(|(field, ..)| *field == app.settings_field)
        .unwrap_or(0);
    let label_width = if rows[1].width >= 60 {
        20
    } else {
        16.min(rows[1].width.saturating_sub(4))
    };
    let value_width = rows[1].width.saturating_sub(label_width + 2);
    let raw_edit = match app.settings_field {
        SettingsField::Host => app.settings_input.as_str(),
        SettingsField::Offset => app.offset_input.as_str(),
        _ => "",
    };
    let edit_text = single_line(raw_edit, app);
    let raw_cursor = cursor_boundary(raw_edit, app.settings_cursor);
    let displayed_cursor = single_line(&raw_edit[..raw_cursor], app)
        .len()
        .min(edit_text.len());
    let (window, cursor_column) = input_window(
        &edit_text,
        cursor_boundary(&edit_text, displayed_cursor),
        value_width,
    );
    let items = fields
        .iter()
        .enumerate()
        .map(|(i, (_, label, value, description))| {
            let editing = i == selected && app.is_settings_editing;
            let text = if editing {
                window.to_string()
            } else {
                fit_text(&single_line(value, app), value_width)
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        format!("{label:<width$}", width = usize::from(label_width)),
                        Style::default()
                            .fg(if i == selected {
                                theme.highlight
                            } else {
                                theme.text()
                            })
                            .bold(),
                    ),
                    Span::styled(
                        text,
                        Style::default().fg(if editing {
                            theme.warning()
                        } else {
                            theme.text()
                        }),
                    ),
                ]),
                Line::from(Span::styled(
                    format!(
                        "{}{}",
                        " ".repeat(usize::from(label_width)),
                        fit_text(description, value_width)
                    ),
                    Style::default().fg(theme.text_secondary),
                )),
            ])
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(selected));
    f.render_stateful_widget(
        List::new(items)
            .highlight_symbol("▎ ")
            .highlight_style(Style::default().bg(theme.selection())),
        rows[1],
        &mut state,
    );
    let hint = if app.is_settings_editing {
        "Editing draft · Enter saves · Esc discards"
    } else {
        "Enter edits text fields. Appearance changes save immediately."
    };
    f.render_widget(
        Paragraph::new(hint)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(if app.is_settings_editing {
                theme.warning()
            } else {
                theme.text_secondary
            })),
        rows[2],
    );
    if app.is_settings_editing && value_width > 0 {
        let y = rows[1]
            .y
            .saturating_add((selected.saturating_sub(state.offset()) as u16).saturating_mul(2));
        let x = rows[1].x.saturating_add(2 + label_width + cursor_column);
        if y < rows[1].bottom() && x < rows[1].right() {
            f.set_cursor_position((x, y));
        }
    }
    if columns[1].width > 0 {
        let descriptions = match app.settings_field {
            SettingsField::Host => (
                "A separate identity",
                "Credentials belong to the full server URL, including its API path. Changing servers clears cached playback and loads only that server's credentials.",
            ),
            SettingsField::Offset => (
                "Make the beat line up",
                "Use positive or negative milliseconds to shift spectrum playback. This affects visualization only, never the audio.",
            ),
            SettingsField::Theme => (
                "A little atmosphere",
                "Every theme uses the same focus rings, quiet surfaces, and high-contrast text. Choose a palette with the left and right arrows.",
            ),
            SettingsField::VizStyle => (
                "See what you hear",
                "The visualizer needs spectrum data from the server. It yields to your queue in smaller terminals. Toggle it with v on the player.",
            ),
            SettingsField::Layout => (
                "Room to breathe",
                "Standard balances queue and spectrum. Sidebar emphasizes spectrum. Studio stacks panels. Zen expands spectrum; Tab brings the queue back.",
            ),
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    descriptions.0,
                    Style::default().fg(theme.highlight).bold(),
                )),
                Line::from(""),
                Line::from(descriptions.1),
            ])
            .block(panel(" About this setting ", theme, false))
            .wrap(Wrap { trim: true }),
            columns[1],
        );
    }
}

pub(super) fn lyrics(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let block = panel(
        if app.lyrics_loading {
            " Lyrics · fetching… "
        } else {
            " Lyrics "
        },
        theme,
        true,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);
    let track = app.current_track.as_deref().unwrap_or("Nothing playing");
    f.render_widget(
        Paragraph::new(fit_text(&single_line(track, app), inner.width))
            .style(Style::default().fg(theme.highlight).bold()),
        rows[0],
    );
    let text = app.lyrics_text.as_deref().unwrap_or(if app.lyrics_loading {
        "Finding the words…"
    } else {
        "No lyrics loaded. Press r to try again."
    });
    let paragraph = Paragraph::new(crate::output::safe_text(text, app.token.as_deref()))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(theme.text()));
    app.lyrics_page_size = rows[1].height.max(1);
    app.lyrics_max_scroll = paragraph
        .line_count(rows[1].width)
        .saturating_sub(usize::from(rows[1].height))
        .min(usize::from(u16::MAX)) as u16;
    app.lyrics_scroll = app.lyrics_scroll.min(app.lyrics_max_scroll);
    f.render_widget(paragraph.scroll((app.lyrics_scroll, 0)), rows[1]);
    f.render_widget(
        Paragraph::new("↑↓ scroll · r refresh · Esc player")
            .style(Style::default().fg(theme.text_secondary)),
        rows[2],
    );
}

pub(super) fn track_input(f: &mut Frame, app: &App, theme: &Theme) {
    let area = popup_rect(74, 9, f.area());
    prepare_modal(f, area, theme);
    let block = panel(" Add music ", theme, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(0),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new(
            app.input_error
                .as_deref()
                .unwrap_or("Search for a song, or paste a track / playlist URL."),
        )
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(if app.input_error.is_some() {
            theme.danger()
        } else {
            theme.text_secondary
        })),
        rows[0],
    );
    f.render_widget(Block::default().bg(theme.raised()), rows[1]);
    let text_area = Rect::new(
        rows[1].x.saturating_add(2),
        rows[1].y,
        rows[1].width.saturating_sub(3),
        rows[1].height.min(1),
    );
    let (text, cursor) = input_window(&app.input, app.input_cursor, text_area.width);
    f.render_widget(
        Paragraph::new(if app.input.is_empty() {
            "Artist, song, or https://…"
        } else {
            text
        })
        .style(Style::default().fg(if app.input.is_empty() {
            theme.text_secondary
        } else {
            theme.text()
        })),
        text_area,
    );
    f.render_widget(
        Paragraph::new(Line::from(
            [
                key_hint("Enter", "add to queue", theme),
                key_hint("Esc", "discard", theme),
            ]
            .concat(),
        )),
        rows[2],
    );
    if !text_area.is_empty() {
        f.set_cursor_position((text_area.x + cursor, text_area.y));
    }
}

pub(super) fn diagnostics(f: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let block = panel(" Diagnostics · newest first ", theme, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);
    let status = format!(
        "{}  ·  {} pending commands  ·  {} events",
        if app.ws_connected {
            "Live connection"
        } else {
            "HTTP mode"
        },
        app.pending_commands.len(),
        app.debug_logs.len()
    );
    f.render_widget(
        Paragraph::new(status).style(Style::default().fg(theme.highlight)),
        rows[0],
    );
    let lines = app
        .debug_logs
        .iter()
        .rev()
        .map(|line| Line::from(fit_text(&single_line(line, app), rows[1].width)))
        .collect::<Vec<_>>();
    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text_secondary)),
        rows[1],
    );
}

pub(super) fn menu(f: &mut Frame, app: &mut App, theme: &Theme, account: bool) {
    let area = popup_rect(60, if account { 12 } else { 17 }, f.area());
    prepare_modal(f, area, theme);
    let block = panel(
        if account {
            " Account "
        } else {
            " Audio presets "
        },
        theme,
        true,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(inner);
    let subtitle = if account {
        "Credentials are saved separately for each server."
    } else {
        "Choose a sound. Clear removes active filters."
    };
    f.render_widget(
        Paragraph::new(subtitle)
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(theme.text_secondary)),
        rows[0],
    );
    let items = if account {
        &app.auth_menu_items
    } else {
        &app.filter_items
    };
    let items = items
        .iter()
        .map(|item| ListItem::new(format!(" {item}")))
        .collect::<Vec<_>>();
    let state = if account {
        &mut app.auth_menu_state
    } else {
        &mut app.filter_state
    };
    f.render_stateful_widget(
        List::new(items).highlight_symbol("▎ ").highlight_style(
            Style::default()
                .bg(theme.selection())
                .fg(theme.highlight)
                .bold(),
        ),
        rows[1],
        state,
    );
    f.render_widget(
        Paragraph::new(Line::from(
            [
                key_hint("Enter", "choose", theme),
                key_hint("Esc", "back", theme),
            ]
            .concat(),
        )),
        rows[2],
    );
}

pub(super) fn account_info(f: &mut Frame, app: &App, theme: &Theme) {
    let area = popup_rect(76, 16, f.area());
    prepare_modal(f, area, theme);
    let block = panel(
        if app.is_loading {
            " Account · working… "
        } else {
            " Account "
        },
        theme,
        true,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).split(inner);
    f.render_widget(
        Paragraph::new(crate::output::safe_text(
            app.auth_info_text
                .as_deref()
                .unwrap_or("No account information loaded."),
            app.token.as_deref(),
        ))
        .wrap(Wrap { trim: false }),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(key_hint(
            "Esc",
            if app.login_cancel.is_some() {
                "cancel login"
            } else {
                "back"
            },
            theme,
        ))),
        rows[1],
    );
}

pub(super) fn update(f: &mut Frame, app: &App, theme: &Theme) {
    let area = popup_rect(70, 14, f.area());
    prepare_modal(f, area, theme);
    let version = app
        .update_info
        .as_ref()
        .map_or("Unknown", |(version, _)| version.as_str());
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("A fresh Jorik: {}", single_line(version, app)),
            Style::default().fg(theme.highlight).bold(),
        )),
        Line::from(format!("Installed: {}", env!("CARGO_PKG_VERSION"))),
        Line::from(""),
        Line::from(
            "Downloads are checked for SHA-256 integrity and signed build provenance. A current authenticated GitHub CLI (gh) is required.",
        ),
        Line::from(""),
        Line::from("Installing exits Jorik. Nothing changes until you confirm."),
        Line::from(""),
        Line::from(
            [
                key_hint("Esc", "not now", theme),
                key_hint("y", "verify & install", theme),
            ]
            .concat(),
        ),
    ];
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .block(panel(" Review update ", theme, true)),
        area,
    );
}

pub(super) fn about(f: &mut Frame, theme: &Theme) {
    let area = popup_rect(70, 14, f.area());
    prepare_modal(f, area, theme);
    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("JORIK  {}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(theme.highlight).bold(),
        )),
        Line::from("Music control, without leaving your terminal."),
        Line::from(""),
        Line::from(
            "Browser login requires a backend supporting the CLI 0.5 state / PKCE protocol. Legacy bearer-query login is not accepted.",
        ),
        Line::from(""),
        Line::from(
            "Live state and spectrum depend on server capabilities. HTTP playback controls remain available without WebSocket.",
        ),
        Line::from(""),
        Line::from(key_hint("Esc", "back to player", theme)),
    ];
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .block(panel(" About Jorik ", theme, true)),
        area,
    );
}

pub(super) fn connection_error(f: &mut Frame, app: &App, theme: &Theme, message: &str) {
    let area = popup_rect(74, 11, f.area());
    prepare_modal(f, area, theme);
    let block = panel(" Connection needs attention ", theme, true)
        .border_style(Style::default().fg(theme.danger()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(inner);
    f.render_widget(
        Paragraph::new("Your session is still here.")
            .style(Style::default().fg(theme.text()).bold()),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(crate::output::safe_text(message, app.token.as_deref()))
            .wrap(Wrap { trim: true })
            .style(Style::default().fg(theme.text_secondary)),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(
            [key_hint("r", "retry", theme), key_hint("q", "quit", theme)].concat(),
        )),
        rows[2],
    );
}
