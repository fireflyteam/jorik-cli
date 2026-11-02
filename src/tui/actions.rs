use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Pause,
    Skip,
    Stop,
    Shuffle,
    Clear,
    LoopTrack,
    LoopQueue,
    LoopOff,
    Mode247,
    Filters,
    Lyrics,
    Turip,
    Auth,
    Settings,
    Refresh,
    Visualizer,
    Help,
    Debug,
    Info,
    Update,
    Quit,
}

pub(super) struct Command {
    pub action: Action,
    pub label: &'static str,
    pub shortcut: &'static str,
}

impl Command {
    pub(super) fn description(&self) -> &'static str {
        match self.action {
            Action::Pause => "Toggle playback; the server confirms the actual state.",
            Action::Skip => "Move past the current track.",
            Action::Stop => "Stop playback in the current voice channel.",
            Action::Shuffle => "Shuffle upcoming tracks on the server.",
            Action::Clear => "Remove upcoming tracks. Requires explicit confirmation.",
            Action::LoopTrack | Action::LoopQueue | Action::LoopOff => {
                "Change repeat mode for this session."
            }
            Action::Mode247 => "Toggle persistent voice-channel connection.",
            Action::Filters => "Choose an audio preset or clear active filters.",
            Action::Lyrics => "Read lyrics for the current track.",
            Action::Turip => "Request the Turip track.",
            Action::Auth => "Sign in, inspect saved credentials, or sign out.",
            Action::Settings => "Choose a server, theme, layout, and visualizer.",
            Action::Refresh => "Reload the queue without interrupting playback.",
            Action::Visualizer => "Show spectrum data when the terminal has enough room.",
            Action::Help => "All keyboard shortcuts, including text editing.",
            Action::Debug => "Inspect connection events and save spectrum data.",
            Action::Info => "Version, compatibility, and security information.",
            Action::Update => "Review an update; nothing installs without confirmation.",
            Action::Quit => "Close Jorik. This does not stop server playback.",
        }
    }
}

pub(super) const COMMANDS: &[Command] = &[
    Command {
        action: Action::Pause,
        label: "Pause / resume playback",
        shortcut: "Space / p",
    },
    Command {
        action: Action::Skip,
        label: "Skip current track",
        shortcut: "s",
    },
    Command {
        action: Action::Stop,
        label: "Stop playback",
        shortcut: "w",
    },
    Command {
        action: Action::Shuffle,
        label: "Shuffle queue",
        shortcut: "",
    },
    Command {
        action: Action::Clear,
        label: "Clear queue…",
        shortcut: "c",
    },
    Command {
        action: Action::LoopTrack,
        label: "Loop current track",
        shortcut: "",
    },
    Command {
        action: Action::LoopQueue,
        label: "Loop queue",
        shortcut: "",
    },
    Command {
        action: Action::LoopOff,
        label: "Loop off",
        shortcut: "",
    },
    Command {
        action: Action::Mode247,
        label: "Toggle 24/7 mode",
        shortcut: "",
    },
    Command {
        action: Action::Filters,
        label: "Audio filters…",
        shortcut: "",
    },
    Command {
        action: Action::Lyrics,
        label: "Show lyrics",
        shortcut: "2",
    },
    Command {
        action: Action::Turip,
        label: "Play Turip",
        shortcut: "",
    },
    Command {
        action: Action::Auth,
        label: "Account / authentication…",
        shortcut: "",
    },
    Command {
        action: Action::Settings,
        label: "Settings",
        shortcut: "3",
    },
    Command {
        action: Action::Refresh,
        label: "Refresh queue",
        shortcut: "r",
    },
    Command {
        action: Action::Visualizer,
        label: "Toggle visualizer",
        shortcut: "v",
    },
    Command {
        action: Action::Help,
        label: "Keyboard help",
        shortcut: "?",
    },
    Command {
        action: Action::Debug,
        label: "Diagnostics",
        shortcut: "4 / Ctrl+d",
    },
    Command {
        action: Action::Info,
        label: "Build information",
        shortcut: "i",
    },
    Command {
        action: Action::Update,
        label: "Review available update",
        shortcut: "u",
    },
    Command {
        action: Action::Quit,
        label: "Quit Jorik",
        shortcut: "q",
    },
];

pub(super) fn filtered_commands(query: &str) -> Vec<&'static Command> {
    let query = query.to_lowercase();
    COMMANDS
        .iter()
        .filter(|command| command.label.to_lowercase().contains(&query))
        .collect()
}

// Keyboard shortcuts and the palette dispatch the same typed actions.
pub(super) fn execute(action: Action, app: &mut App, app_arc: Arc<Mutex<App>>) -> bool {
    if app.view == View::Menu {
        app.view = app.overlay_return;
    }
    let simple = match action {
        Action::Pause => Some("pause"),
        Action::Skip => Some("skip"),
        Action::Stop => Some("stop"),
        Action::Shuffle => Some("shuffle"),
        _ => None,
    };
    if let Some(action) = simple {
        tokio::spawn(async_simple_command(
            app_arc,
            "/webhook/audio".to_string(),
            SimplePayload {
                action,
                guild_id: app.guild_id.clone(),
                user_id: app.user_id.clone(),
            },
        ));
        return false;
    }
    match action {
        Action::Clear => app.open_overlay(View::ConfirmClear),
        Action::LoopTrack | Action::LoopQueue | Action::LoopOff => {
            let mode = match action {
                Action::LoopTrack => "track",
                Action::LoopQueue => "queue",
                _ => "off",
            };
            app.loop_mode = mode.to_string();
            tokio::spawn(async_simple_command(
                app_arc,
                "/webhook/audio".to_string(),
                LoopPayload {
                    action: "loop",
                    guild_id: app.guild_id.clone(),
                    user_id: app.user_id.clone(),
                    loop_mode: mode.to_string(),
                },
            ));
        }
        Action::Mode247 => {
            tokio::spawn(async_simple_command(
                app_arc,
                "/webhook/audio".to_string(),
                TwentyFourSevenPayload {
                    action: "247",
                    guild_id: app.guild_id.clone(),
                    user_id: app.user_id.clone(),
                    enabled: None,
                },
            ));
        }
        Action::Filters => {
            app.overlay_return = app.view;
            app.view = View::FilterMenu;
        }
        Action::Lyrics => {
            app.view = View::Lyrics;
            tokio::spawn(async_fetch_lyrics(app_arc));
        }
        Action::Turip => {
            tokio::spawn(async_play_track(
                app_arc,
                "https://open.spotify.com/track/2RQWB4Asy1rjZL4IUcJ7kn".to_string(),
            ));
        }
        Action::Auth => {
            app.overlay_return = app.view;
            app.view = View::AuthMenu;
        }
        Action::Settings => {
            app.settings_input = app.base_url.clone();
            app.view = View::Settings;
        }
        Action::Refresh => {
            app.error_message = None;
            tokio::spawn(async_fetch_queue(app_arc));
        }
        Action::Visualizer => {
            app.visualizer_enabled = !app.visualizer_enabled;
            app.notify(if app.visualizer_enabled {
                "Visualizer enabled; appears when space permits"
            } else {
                "Visualizer hidden"
            });
        }
        Action::Help => app.open_overlay(View::Help),
        Action::Debug => app.view = View::Debug,
        Action::Info => {
            app.overlay_return = app.view;
            app.view = View::AppInfo;
        }
        Action::Update => {
            if app.update_info.is_some() {
                app.update_notice = false;
                app.overlay_return = app.view;
                app.view = View::UpdateFound;
            } else {
                app.notify("No update available");
            }
        }
        Action::Quit => return true,
        Action::Pause | Action::Skip | Action::Stop | Action::Shuffle => unreachable!(),
    }
    false
}

pub(super) fn handle_palette_keys(
    app: &mut App,
    key: event::KeyEvent,
    app_arc: Arc<Mutex<App>>,
) -> bool {
    let commands = filtered_commands(&app.palette_query);
    match key.code {
        KeyCode::Esc => app.view = app.overlay_return,
        KeyCode::Down | KeyCode::Tab => {
            let next = if commands.is_empty() {
                None
            } else {
                Some((app.menu_state.selected().unwrap_or(0) + 1) % commands.len())
            };
            app.menu_state.select(next);
        }
        KeyCode::Up | KeyCode::BackTab => {
            let next = if commands.is_empty() {
                None
            } else {
                Some((app.menu_state.selected().unwrap_or(0) + commands.len() - 1) % commands.len())
            };
            app.menu_state.select(next);
        }
        KeyCode::Enter => {
            if let Some(command) = app
                .menu_state
                .selected()
                .and_then(|index| commands.get(index))
            {
                return execute(command.action, app, app_arc);
            }
        }
        KeyCode::Backspace => {
            app.palette_query.pop();
            app.menu_state.select(Some(0));
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(event::KeyModifiers::CONTROL | event::KeyModifiers::ALT) =>
        {
            app.palette_query.push(c);
            app.menu_state.select(Some(0));
        }
        _ => {}
    }
    false
}

pub(super) fn handle_clear_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    match key.code {
        KeyCode::Char('y' | 'Y') => {
            app.view = app.overlay_return;
            tokio::spawn(async_simple_command(
                app_arc,
                "/webhook/audio".to_string(),
                SimplePayload {
                    action: "clear",
                    guild_id: app.guild_id.clone(),
                    user_id: app.user_id.clone(),
                },
            ));
        }
        KeyCode::Esc | KeyCode::Char('n' | 'N') => app.view = app.overlay_return,
        _ => {} // Enter never confirms a destructive action accidentally.
    }
}
