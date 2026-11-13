use crate::api::{
    self, AudioFilters, EqualizerBand, FilterPayload, KaraokeOptions, LoopPayload, LowPassOptions,
    LyricsPayload, PlayPayload, PlaybackState, QueuePayload, RotationOptions, SimplePayload,
    TimescaleOptions, TremoloOptions, TwentyFourSevenPayload, VibratoOptions, WsEvent, WsSubscribe,
};
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::style::Stylize;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Bar, BarChart, BarGroup, Block, BorderType, Clear, LineGauge, List, ListItem, ListState,
        Paragraph, Wrap,
    },
};
use reqwest::Client;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(test)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(test)]
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio::time::{interval, timeout};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::protocol::WebSocketConfig,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, protocol::Message},
};
use tokio_util::sync::CancellationToken;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use url::Url;

mod actions;
mod artwork;
mod authentication;
mod chrome;
mod design;
mod desk;
mod player;
mod state;
mod track;
mod views;

use state::{
    CommandKind, MAX_HOST_BYTES, MAX_INPUT_BYTES, MAX_PALETTE_BYTES, PendingCommand, QUEUE_LIMIT,
    QueueRequest, Session,
};
use track::Track;

use actions::{Action, execute as execute_action, handle_clear_keys, handle_palette_keys};
use authentication::{show_auth_info, start_login, start_signout, switch_server};
use chrome::{
    popup_rect, render_clear_confirmation, render_footer, render_header, render_help,
    render_palette,
};
use player::render_player_ui;

// Theme Colors
struct Theme {
    desk_palette: bool,
    bg: Color,
    border: Color,
    primary: Color,
    highlight: Color,
    text_secondary: Color,
}

fn get_theme(name: &str) -> Theme {
    match name {
        "Midnight" => Theme {
            desk_palette: false,
            bg: Color::Rgb(5, 5, 15),
            border: Color::Rgb(40, 40, 60),
            primary: Color::Rgb(100, 100, 255),
            highlight: Color::Rgb(150, 150, 255),
            text_secondary: Color::Rgb(190, 197, 214),
        },
        "Emerald" => Theme {
            desk_palette: false,
            bg: Color::Rgb(10, 18, 17),
            border: Color::Rgb(60, 84, 79),
            primary: Color::Rgb(100, 196, 154),
            highlight: Color::Rgb(147, 224, 179),
            text_secondary: Color::Rgb(192, 207, 201),
        },
        "Ruby" => Theme {
            desk_palette: false,
            bg: Color::Rgb(15, 5, 5),
            border: Color::Rgb(60, 40, 40),
            primary: Color::Rgb(200, 50, 50),
            highlight: Color::Rgb(255, 100, 100),
            text_secondary: Color::Rgb(212, 194, 201),
        },
        "Ocean" => Theme {
            desk_palette: false,
            bg: Color::Rgb(5, 10, 20),
            border: Color::Rgb(40, 60, 100),
            primary: Color::Rgb(50, 150, 255),
            highlight: Color::Rgb(100, 200, 255),
            text_secondary: Color::Rgb(190, 205, 222),
        },
        "Synthwave" => Theme {
            desk_palette: false,
            bg: Color::Rgb(20, 10, 30),
            border: Color::Rgb(100, 40, 100),
            primary: Color::Rgb(255, 50, 255),
            highlight: Color::Rgb(255, 150, 50), // Orange highlight
            text_secondary: Color::Rgb(209, 193, 226),
        },
        "Sepia" => Theme {
            desk_palette: false,
            bg: Color::Rgb(30, 25, 20),
            border: Color::Rgb(80, 70, 60),
            primary: Color::Rgb(180, 140, 100),
            highlight: Color::Rgb(220, 180, 140),
            text_secondary: Color::Rgb(211, 199, 185),
        },
        _ => Theme {
            desk_palette: true,
            // Default Jorik Purple
            bg: Color::Rgb(44, 50, 72),
            border: Color::Rgb(80, 94, 125),
            primary: JORIK_PURPLE,
            highlight: JORIK_HIGHLIGHT,
            text_secondary: Color::Rgb(183, 193, 212),
        },
    }
}

// Approx color from the logo
const JORIK_PURPLE: Color = Color::Rgb(184, 166, 255);
const JORIK_HIGHLIGHT: Color = Color::Rgb(200, 181, 255);

#[derive(PartialEq)]
enum InputMode {
    Normal,
    Editing,
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Main,
    Menu,
    Help,
    ConfirmClear,
    Lyrics,
    FilterMenu,
    AuthMenu,
    AuthResult,
    LoginRequired,
    Settings,
    Debug,
    AppInfo,
    UpdateFound,
    TrackDetails,
}

#[derive(PartialEq, Clone, Copy)]
enum PlayerFocus {
    Playback,
    Queue,
}

#[derive(PartialEq, Clone, Copy)]
enum SettingsField {
    Host,
    Offset,
    Theme,
    VizStyle,
    Layout,
}

struct App {
    client: Client,
    base_url: String,
    token: Option<String>,
    guild_id: Option<String>,
    user_id: Option<String>,

    queue: Vec<Track>,
    queue_total: Option<usize>,
    queue_offset: usize,
    queue_state: ListState,
    queue_page_size: usize,
    focus: PlayerFocus,
    visualizer_enabled: bool,
    visualizer_visible: bool,
    current_track: Option<Track>,
    artwork: artwork::Artwork,
    error_message: Option<String>,
    queue_error: Option<String>,
    loop_mode: String, // "off", "track", "queue"; only displayed when confirmed
    loop_mode_known: bool,
    auth_loading: bool,
    queue_request: Option<QueueRequest>,
    queue_target_offset: usize,
    queue_revision: u64,
    playback_revision: u64,
    next_request_id: u64,
    rest_commands: HashMap<u64, CommandKind>,
    session_generation: u64,
    shutdown: CancellationToken,
    session_cancel: CancellationToken,
    redraw: Arc<Notify>,
    monochrome: bool,

    input: String,
    input_cursor: usize,
    input_error: Option<String>,
    input_truncated: bool,
    add_pending: Option<String>,
    retry_query: Option<String>,
    input_mode: InputMode,
    view: View,

    menu_state: ListState,
    palette_query: String,
    overlay_return: View,
    help_scroll: u16,

    filter_state: ListState,
    filter_items: Vec<&'static str>,

    auth_menu_state: ListState,
    auth_menu_items: Vec<&'static str>,

    lyrics_text: Option<String>,
    lyrics_loading: bool,
    lyrics_scroll: u16,
    lyrics_max_scroll: u16,
    lyrics_page_size: u16,

    auth_info_text: Option<String>,
    legacy_credentials: bool,
    login_cancel: Option<CancellationToken>,
    login_generation: u64,
    signing_out: bool,
    signout_task: Option<tokio::task::JoinHandle<()>>,

    // Real-time data
    spectrogram: Option<Vec<Vec<u8>>>,
    elapsed_ms: u64,
    duration_ms: u64,
    paused: bool,
    has_playback_state: bool,
    last_state_update: Instant,
    last_playback_received: Option<Instant>,
    last_queue_received: Option<Instant>,

    settings_input: String,
    offset_input: String,
    theme: String,
    viz_style: String,
    layout: String,
    settings_field: SettingsField,
    is_settings_editing: bool,
    settings_cursor: usize,
    settings_truncated: bool,
    needs_reconnect: bool,
    visualizer_offset: i64,

    update_info: Option<(String, Vec<api::GiteaAsset>)>,
    update_notice: bool,
    notice: Option<(String, Instant)>,

    debug_logs: Vec<String>,
    ws_connected: bool,
    ws_connecting: bool,
    ws_sender: Option<tokio::sync::mpsc::Sender<Message>>,
    pending_commands: HashMap<String, PendingCommand>,
    next_command_id: u64,

    smoothed_bars: Vec<f32>,
}

impl App {
    fn new(
        client: Client,
        settings: api::Settings,
        token: Option<String>,
        guild_id: Option<String>,
        user_id: Option<String>,
    ) -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));

        let mut filter_state = ListState::default();
        filter_state.select(Some(0));

        let mut auth_menu_state = ListState::default();
        auth_menu_state.select(Some(0));

        let view = if token.is_some() {
            View::Main
        } else {
            View::LoginRequired
        };

        let layout = if settings.layout == "Standard" {
            "Desk".to_string()
        } else {
            settings.layout.clone()
        };
        Self {
            client,
            base_url: settings.base_url.clone(),
            token,
            guild_id,
            user_id,
            queue: Vec::new(),
            queue_total: None,
            queue_offset: 0,
            queue_state: ListState::default(),
            queue_page_size: 10,
            focus: PlayerFocus::Queue,
            visualizer_enabled: !matches!(layout.as_str(), "Desk" | "Queue"),
            visualizer_visible: false,
            current_track: None,
            artwork: artwork::Artwork::new(),
            error_message: None,
            queue_error: None,
            loop_mode: "off".to_string(),
            loop_mode_known: false,
            auth_loading: false,
            queue_request: None,
            queue_target_offset: 0,
            queue_revision: 0,
            playback_revision: 0,
            next_request_id: 0,
            rest_commands: HashMap::new(),
            session_generation: 0,
            shutdown: CancellationToken::new(),
            session_cancel: CancellationToken::new(),
            redraw: Arc::new(Notify::new()),
            monochrome: std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()),
            input: String::new(),
            input_cursor: 0,
            input_error: None,
            input_truncated: false,
            add_pending: None,
            retry_query: None,
            input_mode: InputMode::Normal,
            view,
            menu_state,
            palette_query: String::new(),
            overlay_return: View::Main,
            help_scroll: 0,
            filter_state,
            filter_items: vec![
                "Clear",
                "Bassboost",
                "Nightcore",
                "Vaporwave",
                "8D",
                "Soft",
                "Tremolo",
                "Vibrato",
                "Karaoke",
            ],
            auth_menu_state,
            auth_menu_items: vec!["Login", "Signout", "Info"],
            lyrics_text: None,
            lyrics_loading: false,
            lyrics_scroll: 0,
            lyrics_max_scroll: 0,
            lyrics_page_size: 10,
            auth_info_text: None,
            legacy_credentials: false,
            login_cancel: None,
            login_generation: 0,
            signing_out: false,
            signout_task: None,
            spectrogram: None,
            elapsed_ms: 0,
            duration_ms: 0,
            paused: true,
            has_playback_state: false,
            last_state_update: Instant::now(),
            last_playback_received: None,
            last_queue_received: None,
            settings_input: settings.base_url.clone(),
            offset_input: settings.visualizer_offset.to_string(),
            theme: settings.theme,
            viz_style: settings.visualizer_style,
            layout,
            settings_field: SettingsField::Host,
            is_settings_editing: false,
            settings_cursor: 0,
            settings_truncated: false,
            needs_reconnect: false,
            visualizer_offset: settings.visualizer_offset,
            update_info: None,
            update_notice: false,
            notice: None,
            debug_logs: Vec::new(),
            ws_connected: false,
            ws_connecting: false,
            ws_sender: None,
            pending_commands: HashMap::new(),
            next_command_id: 0,
            smoothed_bars: vec![0.0; 64],
        }
    }

    fn cancel_login(&mut self) {
        if let Some(cancel) = self.login_cancel.take() {
            cancel.cancel();
            self.login_generation += 1;
            self.auth_loading = false;
            self.wake();
            self.auth_info_text = Some("Login cancelled.".to_string());
        }
    }

    fn clear_session(&mut self) {
        self.session_cancel.cancel();
        self.session_cancel = CancellationToken::new();
        self.session_generation = self.session_generation.wrapping_add(1);
        self.queue_request = None;
        self.queue_target_offset = 0;
        self.rest_commands.clear();
        self.add_pending = None;
        self.retry_query = None;
        self.input.clear();
        self.input_cursor = 0;
        self.input_error = None;
        self.input_truncated = false;
        self.input_mode = InputMode::Normal;
        self.loop_mode = "off".into();
        self.loop_mode_known = false;
        self.token = None;
        self.guild_id = None;
        self.user_id = None;
        self.queue.clear();
        self.queue_state.select(None);
        self.current_track = None;
        self.queue_total = None;
        self.queue_offset = 0;
        self.artwork.clear();
        self.last_playback_received = None;
        self.last_queue_received = None;
        self.lyrics_text = None;
        self.lyrics_loading = false;
        self.lyrics_scroll = 0;
        self.spectrogram = None;
        self.smoothed_bars.fill(0.0);
        self.elapsed_ms = 0;
        self.duration_ms = 0;
        self.has_playback_state = false;
        self.paused = true;
        self.queue_error = None;
        self.auth_info_text = None;
        self.auth_loading = false;
        self.debug_logs.clear();
        self.disconnect_ws();
        self.error_message = None;
        self.needs_reconnect = true;
        self.wake();
    }

    fn clamp_queue_selection(&mut self) {
        let selected = if self.queue.is_empty() {
            None
        } else {
            Some(
                self.queue_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.queue.len() - 1),
            )
        };
        self.queue_state.select(selected);
    }

    fn move_queue(&mut self, delta: isize) {
        self.focus = PlayerFocus::Queue;
        self.clamp_queue_selection();
        if let Some(selected) = self.queue_state.selected() {
            self.queue_state.select(Some(
                selected
                    .saturating_add_signed(delta)
                    .min(self.queue.len() - 1),
            ));
        }
    }

    fn open_overlay(&mut self, view: View) {
        self.overlay_return = self.view;
        self.view = view;
        self.palette_query.clear();
        self.menu_state.select(Some(0));
        self.help_scroll = 0;
    }

    fn content_view(&self) -> View {
        if matches!(
            self.view,
            View::Menu
                | View::Help
                | View::ConfirmClear
                | View::UpdateFound
                | View::FilterMenu
                | View::AuthMenu
                | View::AuthResult
                | View::AppInfo
                | View::TrackDetails
        ) {
            self.overlay_return
        } else {
            self.view
        }
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
        self.wake();
    }

    fn offer_update(&mut self, update: (String, Vec<api::GiteaAsset>)) {
        self.update_info = Some(update);
        self.update_notice = true;
        self.wake();
    }

    fn begin_search(&mut self, initial: &str) {
        self.input = if initial.is_empty() {
            self.retry_query
                .take()
                .or_else(|| self.add_pending.clone())
                .unwrap_or_default()
        } else {
            initial.to_string()
        };
        self.input
            .truncate(cursor_boundary(&self.input, MAX_INPUT_BYTES));
        self.input_cursor = self.input.len();
        self.input_error = None;
        self.input_truncated = false;
        self.input_mode = InputMode::Editing;
    }

    #[cfg(test)]
    fn send_ws_action<T: serde::Serialize>(&mut self, payload: &T) -> bool {
        self.send_ws_command(payload, CommandKind::Other)
    }

    fn send_ws_command<T: serde::Serialize>(&mut self, payload: &T, kind: CommandKind) -> bool {
        if !self.ws_connected {
            return false;
        }
        let Some(sender) = &self.ws_sender else {
            return false;
        };
        self.next_command_id += 1;
        let id = format!("cmd-{}", self.next_command_id);
        let action = api::WsAction {
            event_type: "action",
            id: id.clone(),
            payload,
        };
        let Ok(json) = serde_json::to_string(&action) else {
            return false;
        };
        if sender.try_send(Message::Text(json.into())).is_err() {
            return false;
        }
        self.pending_commands.insert(
            id,
            PendingCommand {
                started: Instant::now(),
                kind,
            },
        );
        self.wake();
        true
    }

    fn finish_ws_action(&mut self, event: &WsEvent) -> bool {
        let Some(id) = event.id.as_deref() else {
            return false;
        };
        let Some(pending) = self.pending_commands.remove(id) else {
            return false;
        };
        if event.success != Some(true) {
            let detail = event
                .message
                .clone()
                .or_else(|| {
                    event
                        .error
                        .as_ref()
                        .map(|error| crate::output::safe_json_text(error, self.token.as_deref()))
                })
                .or_else(|| {
                    event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "The server rejected the command.".to_string());
            let detail = crate::output::safe_text(&detail, self.token.as_deref())
                .chars()
                .take(512)
                .collect::<String>();
            self.finish_command(pending.kind, Err(format!("Command failed: {detail}")));
            false
        } else {
            self.finish_command(pending.kind, Ok(()));
            true
        }
    }

    fn disconnect_ws(&mut self) {
        // An unsuccessful upgrade must not erase state just fetched over HTTP.
        if self.ws_connected {
            self.has_playback_state = false;
        }
        self.ws_connected = false;
        self.ws_connecting = false;
        let pending = std::mem::take(&mut self.pending_commands);
        for (_, pending) in pending {
            self.finish_command(
                pending.kind,
                Err("Connection lost before command confirmation. Refresh before retrying.".into()),
            );
        }
        self.wake();
    }

    fn is_busy(&self) -> bool {
        self.auth_loading
            || self.queue_request.is_some()
            || self.lyrics_loading
            || !self.rest_commands.is_empty()
            || !self.pending_commands.is_empty()
    }

    fn log(&mut self, msg: impl Into<String>) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        let message = crate::output::safe_text(&msg.into(), self.token.as_deref())
            .replace(['\n', '\r'], " ")
            .chars()
            .take(2048)
            .collect::<String>();
        self.debug_logs.push(format!("[{}] {}", timestamp, message));
        if self.debug_logs.len() > 100 {
            self.debug_logs.remove(0);
        }
        self.wake();
    }

    fn save_spectrogram(&mut self) {
        let spec = match &self.spectrogram {
            Some(s) => s,
            None => {
                self.log("Save failed: No spectrogram data available.");
                return;
            }
        };

        let desktop = match dirs::desktop_dir() {
            Some(d) => d,
            None => {
                self.log("Save failed: Could not find Desktop directory.");
                return;
            }
        };

        let filename = format!(
            "spectrogram_{}.json",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        );
        let path = desktop.join(filename);

        match serde_json::to_string_pretty(spec) {
            Ok(json) => {
                if std::fs::write(&path, json).is_ok() {
                    self.log(format!("Spectrogram saved to: {:?}", path));
                } else {
                    self.log("Save failed: Could not write to file.");
                }
            }
            Err(_) => {
                self.log("Save failed: Could not serialize spectrogram.");
            }
        }
    }

    fn apply_playback(&mut self, playback: PlaybackState) {
        self.playback_revision = self.playback_revision.wrapping_add(1);
        self.wake();
        self.elapsed_ms = playback.elapsed_ms;
        self.duration_ms = playback.duration_ms;
        self.paused = playback.paused;
        self.has_playback_state = true;
        self.last_state_update = Instant::now();
        self.last_playback_received = Some(Instant::now());
        if let Some(spec) = playback.spectrogram {
            self.spectrogram = Some(spec);
        }
    }

    fn parse_queue_response(&mut self, json: &Value) {
        let response_guild = json
            .get("guild_id")
            .or_else(|| json.get("guildId"))
            .and_then(Value::as_str);
        if let (Some(response_guild), Some(guild)) = (response_guild, &self.guild_id)
            && response_guild != guild
        {
            return;
        }
        self.wake();
        let previous_track = self.current_track.clone();
        // Handle nested queue object if present
        let target = if let Some(queue) = json.get("queue") {
            queue
        } else {
            json
        };

        if target.get("upcoming").is_some()
            || target.get("current").is_some()
            || json.get("settings").is_some()
        {
            self.queue_revision = self.queue_revision.wrapping_add(1);
        }
        if let Some(total) = json
            .get("total_upcoming")
            .or_else(|| target.get("total"))
            .and_then(Value::as_u64)
        {
            self.queue_total = usize::try_from(total).ok();
        }

        // Capture guild_id if provided by server
        if let Some(gid) = json.get("guild_id").and_then(|v| v.as_str()) {
            if self.guild_id.is_none() {
                self.log(format!("Discovered Guild ID: {}", gid));
            }
            self.guild_id = Some(gid.to_string());
        } else if let Some(gid) = json.get("guildId").and_then(|v| v.as_str()) {
            if self.guild_id.is_none() {
                self.log(format!("Discovered Guild ID: {}", gid));
            }
            self.guild_id = Some(gid.to_string());
        }

        if let Some(current) = target.get("current").filter(|value| value.is_object()) {
            self.current_track = Track::parse(current, self.token.as_deref());
        } else if target.get("current").is_some_and(Value::is_null) {
            // Partial updates must not erase a known recording.
            self.current_track = None;
        }

        if let Some(upcoming) = target.get("upcoming").and_then(|v| v.as_array()) {
            self.queue = upcoming
                .iter()
                .filter_map(|item| Track::parse(item, self.token.as_deref()))
                .collect();
            self.queue_total = json
                .get("total_upcoming")
                .or_else(|| target.get("total"))
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok());
            let offset = json
                .get("offset")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0);
            if offset != self.queue_offset {
                self.queue_state = ListState::default();
            }
            self.queue_offset = offset;
            self.last_queue_received = Some(Instant::now());
            self.queue_error = None;
        }
        self.clamp_queue_selection();
        let changed = match (&self.current_track, &previous_track) {
            (Some(current), Some(previous)) => !current.same_recording(previous),
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.lyrics_text = None;
            self.lyrics_scroll = 0;
            self.lyrics_max_scroll = 0;
            self.has_playback_state = false;
            self.last_playback_received = None;
            self.elapsed_ms = 0;
            self.duration_ms = 0;
            self.paused = true;
            self.spectrogram = None;
            self.smoothed_bars.fill(0.0);
            self.last_state_update = Instant::now();
        }
        self.artwork.set_url(
            self.current_track
                .as_ref()
                .and_then(|track| track.thumbnail.as_deref()),
        );
        if let Some(mode) = json.pointer("/settings/loopMode").and_then(Value::as_str)
            && matches!(mode, "off" | "track" | "queue")
        {
            self.loop_mode = mode.into();
            self.loop_mode_known = true;
        }
        if let Some(playback) = json
            .get("playback")
            .or_else(|| target.get("playback"))
            .and_then(|value| serde_json::from_value::<PlaybackState>(value.clone()).ok())
        {
            self.apply_playback(playback);
        }
    }

    fn update_realtime(&mut self) {
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, started)| started.elapsed() >= Duration::from_secs(5))
        {
            self.notice = None;
        }
        let expired = self
            .pending_commands
            .iter()
            .filter(|(_, pending)| pending.started.elapsed() >= Duration::from_secs(10))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in expired {
            if let Some(pending) = self.pending_commands.remove(&id) {
                self.finish_command(
                    pending.kind,
                    Err("Command confirmation timed out. Refresh before retrying.".into()),
                );
            }
        }
        if self.playback_is_stale() {
            self.has_playback_state = false;
            self.smoothed_bars.fill(0.0);
        }
        if self.current_track.is_some() && self.has_playback_state && !self.paused {
            let now = Instant::now();
            let delta = now.duration_since(self.last_state_update).as_millis() as u64;
            self.elapsed_ms = self.elapsed_ms.saturating_add(delta);
            self.last_state_update = now;

            if self.duration_ms > 0 && self.elapsed_ms > self.duration_ms {
                self.elapsed_ms = self.duration_ms;
            }

            // Smoothing logic
            if let Some(spec) = &self.spectrogram {
                let adjusted_ms = self
                    .elapsed_ms
                    .saturating_add_signed(self.visualizer_offset);
                let frame_index = (adjusted_ms as f64 / 42.66).floor() as usize;
                if frame_index < spec.len() {
                    let target_bars = &spec[frame_index];
                    for (i, &target) in target_bars.iter().take(64).enumerate() {
                        let target = target as f32;
                        let current = self.smoothed_bars[i];

                        // Variable noise floor: higher for sub-bass to ignore rumble
                        let floor = if i < 3 { 60.0 } else { 30.0 };
                        let raw_signal = (target - floor).max(0.0);

                        // Simple direct scaling
                        let gain = if i == 0 { 0.1 } else { 0.6 };
                        let scaled_target = (raw_signal * gain).min(100.0);

                        // Preserve smoothing speed at any redraw cadence.
                        let frames = delta as f32 / (1000.0 / 60.0);
                        let retention = if scaled_target > current {
                            0.6_f32
                        } else {
                            0.85_f32
                        };
                        self.smoothed_bars[i] =
                            scaled_target + (current - scaled_target) * retention.powf(frames);
                    }
                }
            }
        } else {
            let frames = self.last_state_update.elapsed().as_secs_f32() * 60.0;
            self.last_state_update = Instant::now();
            for value in &mut self.smoothed_bars {
                *value *= 0.95_f32.powf(frames);
            }
        }
    }
}

// Spawning helpers
async fn checked_request(
    req: reqwest::RequestBuilder,
) -> std::result::Result<reqwest::Response, String> {
    let response = req
        .send()
        .await
        .map_err(|error| format!("Network error: {}", error.without_url()))?;
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = crate::api::response_text(response)
        .await
        .map_err(|error| error.to_string())?;
    let display = match serde_json::from_str::<Value>(&body) {
        Ok(value) => crate::output::safe_json_text(&value, None),
        Err(_) => crate::output::safe_text(&body, None),
    };
    Err(format!(
        "HTTP {status}: {}",
        display.chars().take(512).collect::<String>()
    ))
}

async fn request_json(req: reqwest::RequestBuilder) -> std::result::Result<Value, String> {
    let response = checked_request(req).await?;
    let body = crate::api::response_text(response)
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&body).map_err(|_| "Invalid server response: malformed JSON".to_string())
}

async fn async_fetch_queue(app_arc: Arc<Mutex<App>>) {
    loop {
        let (client, base_url, token, payload, request) = {
            let mut app = app_arc.lock().await;
            let Some(request) = app.begin_queue_request() else {
                return;
            };
            let payload = QueuePayload {
                action: "queue",
                guild_id: app.guild_id.clone(),
                user_id: app.user_id.clone(),
                limit: QUEUE_LIMIT,
                offset: request.offset,
            };
            (
                app.client.clone(),
                app.base_url.clone(),
                app.token.clone(),
                payload,
                request,
            )
        };
        let mut req = client
            .post(api::build_url(&base_url, "/webhook/audio"))
            .json(&payload);
        if let Some(bearer) = &token {
            req = req.bearer_auth(bearer);
        }
        // Network reads and decoding never hold the UI lock.
        let result = request_json(req).await;
        if !app_arc.lock().await.finish_queue_request(&request, result) {
            break;
        }
    }
}

async fn async_play_track(app_arc: Arc<Mutex<App>>, query: String) {
    if query.trim().is_empty() {
        return;
    }
    let payload = {
        let app = app_arc.lock().await;
        PlayPayload {
            action: "play",
            guild_id: app.guild_id.clone(),
            channel_id: None,
            query: api::clean_query(query.trim()),
            user_id: app.user_id.clone(),
            requested_by: None,
            avatar_url: None,
        }
    };
    async_command(
        app_arc,
        "/webhook/audio".to_string(),
        payload,
        CommandKind::Add(query),
    )
    .await;
}

async fn async_fetch_lyrics(app_arc: Arc<Mutex<App>>) {
    let (client, base_url, token, payload, track, session) = {
        let mut app = app_arc.lock().await;
        if app.lyrics_loading {
            return;
        }
        if app.token.is_none() {
            app.lyrics_text = Some("Log in to this server first.".to_string());
            return;
        }
        app.lyrics_loading = true;
        app.wake();
        let payload = LyricsPayload {
            action: "lyrics".to_string(),
            guild_id: app.guild_id.clone(),
            user_id: app.user_id.clone(),
        };
        (
            app.client.clone(),
            app.base_url.clone(),
            app.token.clone(),
            payload,
            app.current_track.clone(),
            Session::capture(&app),
        )
    };

    // Lyrics use REST until there is a correlated WS result handler. Sending
    // through both transports duplicates the action without consuming its result.
    let mut req = client
        .post(api::build_url(&base_url, "/webhook/audio"))
        .json(&payload);
    if let Some(bearer) = &token {
        req = req.bearer_auth(bearer);
    }
    let result = request_json(req).await;

    let mut app = app_arc.lock().await;
    if !session.matches(&app) {
        return;
    }
    app.wake();
    app.lyrics_scroll = 0;
    app.lyrics_loading = false;
    let same_track = match (&app.current_track, &track) {
        (Some(current), Some(requested)) => current.same_recording(requested),
        (None, None) => true,
        _ => false,
    };
    if !same_track {
        app.lyrics_text = Some("The track changed. Press r for the current lyrics.".into());
        return;
    }
    let lyrics = match result {
        Ok(json) => {
            let data = &json["data"];
            let text = if let Some(text) = data["text"].as_str() {
                text.to_string()
            } else if let Some(lines) = data["lines"].as_array() {
                lines
                    .iter()
                    .map(|line| line["line"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                String::new()
            };
            if text.trim().is_empty() {
                "No lyrics found.".to_string()
            } else {
                text
            }
        }
        Err(error) => format!("Failed to fetch lyrics: {error}"),
    };
    app.lyrics_text = Some(crate::output::safe_text(&lyrics, app.token.as_deref()));
}

async fn async_simple_command<T: serde::Serialize + Send + Sync + 'static>(
    app_arc: Arc<Mutex<App>>,
    endpoint: String,
    payload: T,
) {
    async_command(app_arc, endpoint, payload, CommandKind::Other).await;
}

async fn async_command<T: serde::Serialize + Send + Sync + 'static>(
    app_arc: Arc<Mutex<App>>,
    endpoint: String,
    payload: T,
    kind: CommandKind,
) {
    let (client, base_url, token, session, id) = {
        let mut app = app_arc.lock().await;
        if app.token.is_none() {
            app.finish_command(kind, Err("Log in to this server first.".into()));
            return;
        }
        if matches!(kind, CommandKind::Add(_))
            && let Some(query) = app.pending_add_query().map(str::to_owned)
        {
            app.add_pending = Some(query);
            app.notify("A music request is already awaiting confirmation");
            return;
        }
        if matches!(kind, CommandKind::Repeat(_)) && app.pending_repeat().is_some() {
            app.notify("A repeat request is already awaiting confirmation");
            return;
        }
        if app.rest_commands.len() + app.pending_commands.len() >= 32 {
            app.finish_command(
                kind,
                Err("Too many pending commands. Wait for confirmation before retrying.".into()),
            );
            return;
        }
        app.error_message = None;
        if let CommandKind::Add(query) = &kind {
            app.add_pending = Some(query.clone());
        }
        if endpoint == "/webhook/audio" && app.send_ws_command(&payload, kind.clone()) {
            return;
        }
        app.next_request_id = app.next_request_id.wrapping_add(1);
        let id = app.next_request_id;
        app.rest_commands.insert(id, kind.clone());
        app.wake();
        (
            app.client.clone(),
            app.base_url.clone(),
            app.token.clone(),
            Session::capture(&app),
            id,
        )
    };

    let mut req = client
        .post(api::build_url(&base_url, &endpoint))
        .json(&payload);
    if let Some(bearer) = &token {
        req = req.bearer_auth(bearer);
    }
    let result = checked_request(req).await;
    {
        let mut app = app_arc.lock().await;
        if !session.matches(&app) {
            return;
        }
        app.rest_commands.remove(&id);
        if let Err(error) = result {
            let error = if error.starts_with("HTTP ") {
                format!("Command failed: {error}")
            } else {
                format!("Command outcome unconfirmed: {error}. Refresh before retrying.")
            };
            app.finish_command(kind, Err(error));
            return;
        }
        app.finish_command(kind, Ok(()));
    }
    async_fetch_queue(app_arc).await;
}

fn websocket_url(base: &str) -> Result<Url> {
    let server = crate::credentials::server_identity(base)?;
    let mut url = Url::parse(&api::build_url(&server, "/ws"))?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| anyhow::anyhow!("invalid WebSocket URL"))?;
    Ok(url)
}

fn websocket_request(
    base: &str,
    token: &str,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>> {
    crate::credentials::validate_token(token)?;
    let mut request = websocket_url(base)?.as_str().into_client_request()?;
    let headers = request.headers_mut();
    headers.insert("User-Agent", HeaderValue::from_static("jorik-cli"));
    let origin = Url::parse(base)?.origin().ascii_serialization();
    headers.insert("Origin", HeaderValue::from_str(&origin)?);
    let mut bearer = HeaderValue::from_str(&format!("Bearer {token}"))?;
    bearer.set_sensitive(true);
    headers.insert("Authorization", bearer);
    Ok(request)
}

async fn spawn_websocket(
    app_arc: Arc<Mutex<App>>,
    mut ws_rx: tokio::sync::mpsc::Receiver<Message>,
) {
    let mut last_waiting_log = Instant::now();

    loop {
        let (base_url, token, guild_id, session) = {
            let app = app_arc.lock().await;
            (
                app.base_url.clone(),
                app.token.clone(),
                app.guild_id.clone(),
                Session::capture(&app),
            )
        };

        if token.is_none() || guild_id.is_none() {
            if last_waiting_log.elapsed() > Duration::from_secs(10) {
                let mut app = app_arc.lock().await;
                if token.is_none() {
                    app.log("WS waiting for token...");
                } else if guild_id.is_none() {
                    app.log(
                        "WS waiting for Guild ID (join a voice channel or specify --guild-id)...",
                    );
                }
                last_waiting_log = Instant::now();
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        let token = token.unwrap();
        let guild_id = guild_id.unwrap();

        let ws_url = match websocket_url(&base_url) {
            Ok(url) => url,
            Err(_) => {
                let mut app = app_arc.lock().await;
                app.log("Invalid or insecure WebSocket server URL");
                drop(app);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        {
            let mut app = app_arc.lock().await;
            if !session.matches(&app) {
                continue;
            }
            app.needs_reconnect = false;
            app.log(format!("WS Connecting to {}", redacted_url(&ws_url)));
            app.ws_connected = false;
            app.ws_connecting = true;
        }

        let request = match websocket_request(&base_url, &token) {
            Ok(request) => request,
            Err(_) => {
                let mut app = app_arc.lock().await;
                app.log("Could not build authenticated WebSocket request");
                app.ws_connecting = false;
                drop(app);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let websocket_limits = WebSocketConfig::default()
            .max_message_size(Some(4 * 1024 * 1024))
            .max_frame_size(Some(4 * 1024 * 1024));
        match timeout(
            Duration::from_secs(10),
            connect_async_with_config(request, Some(websocket_limits), false),
        )
        .await
        {
            Ok(Ok((mut ws_stream, _))) => {
                {
                    let mut app = app_arc.lock().await;
                    if !session.matches(&app) {
                        continue;
                    }
                    app.log("WS Connected");
                    app.ws_connected = true;
                    app.ws_connecting = false;
                }

                let sub = WsSubscribe {
                    event_type: "subscribe",
                    guild_id: guild_id.clone(),
                };
                if let Ok(json) = serde_json::to_string(&sub)
                    && !matches!(
                        timeout(
                            Duration::from_secs(10),
                            ws_stream.send(Message::Text(json.into()))
                        )
                        .await,
                        Ok(Ok(()))
                    )
                {
                    app_arc.lock().await.disconnect_ws();
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }

                let mut reconnect_tick = interval(Duration::from_millis(500));
                loop {
                    tokio::select! {
                        msg = ws_stream.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(event) = serde_json::from_str::<WsEvent>(&text) {
                                        let mut app = app_arc.lock().await;
                                        if !session.matches(&app) {
                                            break;
                                        }
                                        app.log(format!("WS Event: {}", event.event_type));

                                        match event.event_type.as_str() {
                                            "spectrogram_update" => {
                                                if event.guild_id.as_deref() == app.guild_id.as_deref()
                                                    && let Some(data) = event.data
                                                    && let Ok(spectrogram) = serde_json::from_value::<Vec<Vec<u8>>>(data)
                                                {
                                                    app.log(format!("Received Spectrogram ({} frames)", spectrogram.len()));
                                                    app.spectrogram = Some(spectrogram);
                                                }
                                            }
                                            "state_update" | "initial_state" => {
                                                if event.guild_id.as_deref() == app.guild_id.as_deref() {
                                                    if let Some(data) = &event.data
                                                        && app.parse_ws_state(data) {
                                                        app.spawn(async_fetch_queue(app_arc.clone()));
                                                    }

                                                    // Check both root and data.playback for robustness
                                                    let playback = event.playback.clone().or_else(|| {
                                                        event.data.as_ref()
                                                            .and_then(|d| d.get("playback"))
                                                            .and_then(|p| serde_json::from_value::<PlaybackState>(p.clone()).ok())
                                                    });

                                                    if let Some(playback) = playback {
                                                        if playback.elapsed_ms % 5000 < 500 { // Log every ~5 seconds
                                                            app.log(format!("State Update: elapsed={}ms, paused={}", playback.elapsed_ms, playback.paused));
                                                        }
                                                        if app.elapsed_ms == 0 && playback.elapsed_ms > 0 {
                                                            app.log(format!("Synced playback to {}ms", playback.elapsed_ms));
                                                        }
                                                        app.apply_playback(playback);
                                                    }
                                                }
                                            }
                                            "queue_update" => {
                                                if event.guild_id.as_deref() == app.guild_id.as_deref() {
                                                    app.log("Received Queue Update");
                                                    if let Some(data) = event.data {
                                                        if app.parse_ws_state(&data) {
                                                            app.spawn(async_fetch_queue(app_arc.clone()));
                                                        }
                                                    } else {
                                                        // Fallback to REST if data is missing
                                                        app.spawn(async_fetch_queue(app_arc.clone()));
                                                    }
                                                }
                                            }
                                            "track_start" | "track_end" | "player_update" => {
                                                if event.guild_id.as_deref() == app.guild_id.as_deref() {
                                                    app.log(format!("WS Event: {}, refreshing queue", event.event_type));
                                                    // Trigger a full REST refresh to get the latest queue state
                                                    app.spawn(async_fetch_queue(app_arc.clone()));
                                                }
                                            }
                                            "action_response" => {
                                                let success = event.success.unwrap_or(false);
                                                let id = event.id.as_deref().unwrap_or("unknown");
                                                app.log(format!("WS Action Response [{}]: success={}", id, success));
                                                if app.finish_ws_action(&event) {
                                                    app.spawn(async_fetch_queue(app_arc.clone()));
                                                }
                                            }
                                            _ => {
                                                app.log(format!("WS Unhandled Event: {}", event.event_type));
                                            }
                                        }
                                    } else {
                                        let mut app = app_arc.lock().await;
                                        app.log(format!("WS Unparsed Message: {}", text));
                                    }
                                }
                                Some(Err(e)) => {
                                    let mut app = app_arc.lock().await;
                                    app.log(format!("WS Error: {}", e));
                                    break;
                                }
                                None => {
                                    let mut app = app_arc.lock().await;
                                    app.log("WS Closed");
                                    break;
                                }
                                _ => {}
                            }
                        }
                        Some(out_msg) = ws_rx.recv() => {
                            {
                                let app = app_arc.lock().await;
                                if !session.matches(&app) {
                                    break;
                                }
                                if let Message::Text(text) = &out_msg {
                                    let id = serde_json::from_str::<Value>(text).ok()
                                        .and_then(|json| json.get("id").and_then(Value::as_str).map(str::to_owned));
                                    if !id.as_ref().is_some_and(|id| app.pending_commands.get(id)
                                        .is_some_and(|pending| pending.started.elapsed() < Duration::from_secs(10))) {
                                        continue;
                                    }
                                }
                            }
                            if !matches!(timeout(Duration::from_secs(10), ws_stream.send(out_msg)).await, Ok(Ok(()))) {
                                app_arc.lock().await.log("WS send failed or timed out; outcome unconfirmed");
                                break;
                            }
                        }
                        _ = reconnect_tick.tick() => {
                            let mut app = app_arc.lock().await;
                            if app.needs_reconnect || !session.matches(&app)
                            {
                                app.log("WS Forcing reconnect due to settings change");
                                app.needs_reconnect = false;
                                break;
                            }
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                let mut app = app_arc.lock().await;
                app.log(format!("WS Connection Failed: {}", e));
                app.ws_connecting = false;
            }
            Err(_) => {
                let mut app = app_arc.lock().await;
                app.log("WS connection timed out; using HTTP mode");
                app.ws_connecting = false;
            }
        }

        {
            let mut app = app_arc.lock().await;
            app.disconnect_ws();
        }
        // Never replay unacknowledged actions after a reconnect: playback
        // actions such as skip are not safe to execute twice.
        while ws_rx.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

pub async fn run(
    settings: api::Settings,
    token: Option<String>,
    guild_id: Option<String>,
    user_id: Option<String>,
) -> Result<Option<(String, Vec<api::GiteaAsset>)>> {
    let client = Client::builder()
        .user_agent("jorik-cli-tui")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?;

    crate::credentials::server_identity(&settings.base_url)?;
    let (ws_tx, ws_rx) = tokio::sync::mpsc::channel::<Message>(32);

    let mut app_struct = App::new(client.clone(), settings, token, guild_id, user_id);
    app_struct.legacy_credentials = crate::credentials::legacy_present();
    app_struct.ws_sender = Some(ws_tx);

    let app = Arc::new(Mutex::new(app_struct));

    let mut terminal = ratatui::try_init()?;
    let mut terminal_session = TerminalSession {
        paste_enabled: false,
        shutdown: app.lock().await.shutdown.clone(),
    };
    if !app.lock().await.monochrome {
        app.lock()
            .await
            .artwork
            .set_picker(artwork::terminal_picker());
    }
    terminal_session.paste_enabled =
        ratatui::crossterm::execute!(std::io::stdout(), event::EnableBracketedPaste).is_ok();

    // All non-cleanup tasks are owned by this TUI session.
    app.lock().await.spawn(async_fetch_queue(app.clone()));
    app.lock()
        .await
        .spawn_background(spawn_websocket(app.clone(), ws_rx));

    let app_update = app.clone();
    app.lock().await.spawn_background(async move {
        if let Some(update) = crate::updater::check_for_updates().await {
            let mut app = app_update.lock().await;
            app.offer_update(update);
        }
    });

    let app_clone = app.clone();
    app.lock().await.spawn_background(async move {
        // Poll every 20 seconds for safety if WS misses an update
        let mut interval = interval(Duration::from_secs(20));
        interval.tick().await; // Initial fetch is already spawned above.
        loop {
            interval.tick().await;
            async_fetch_queue(app_clone.clone()).await;
        }
    });

    let res = run_loop(&mut terminal, app.clone()).await;
    let signout = {
        let mut app = app.lock().await;
        app.cancel_login();
        app.shutdown.cancel();
        app.signout_task.take()
    };
    drop(terminal_session);
    if let Some(signout) = signout {
        if !signout.is_finished() {
            eprintln!("Finishing sign-out…");
        }
        signout
            .await
            .map_err(|_| anyhow::anyhow!("sign-out cleanup task failed"))?;
    }
    res
}

struct TerminalSession {
    paste_enabled: bool,
    shutdown: CancellationToken,
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if self.paste_enabled {
            let _ = ratatui::crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste);
        }
        ratatui::restore();
    }
}

fn should_quit(app: &App, key: event::KeyEvent) -> bool {
    (key.code == KeyCode::Char('c')
        && key.modifiers.contains(event::KeyModifiers::CONTROL)
        && !key.modifiers.contains(event::KeyModifiers::ALT))
        || (matches!(key.code, KeyCode::Char('q') | KeyCode::Char('й'))
            && !key
                .modifiers
                .intersects(event::KeyModifiers::CONTROL | event::KeyModifiers::ALT)
            && app.input_mode == InputMode::Normal
            && !app.is_settings_editing
            && !matches!(app.view, View::Settings | View::Menu))
}

enum UiOutcome {
    Continue,
    Quit,
    Update((String, Vec<api::GiteaAsset>)),
}

async fn run_loop(
    terminal: &mut DefaultTerminal,
    app_arc: Arc<Mutex<App>>,
) -> Result<Option<(String, Vec<api::GiteaAsset>)>> {
    let mut events = event::EventStream::new();
    let redraw = app_arc.lock().await.redraw.clone();
    loop {
        let frame_interval = {
            let mut app = app_arc.lock().await;
            app.update_realtime();
            terminal.draw(|f| ui(f, &mut app))?;
            if !app.monochrome
                && let Some(request) = app.artwork.take_request()
            {
                app.spawn(artwork::load(app_arc.clone(), app.client.clone(), request));
            }
            app.frame_interval()
        };
        let now = tokio::time::Instant::now();
        let tick_due = now + frame_interval;
        let paint_due = now + state::FRAME_MIN_INTERVAL;
        let mut dirty = false;
        let event = loop {
            tokio::select! {
                event = events.next() => break Some(match event {
                    Some(event) => event?,
                    None => return Ok(None),
                }),
                _ = redraw.notified() => dirty = true,
                // Coalesce background storms, but never delay keyboard input or resizing.
                _ = tokio::time::sleep_until(paint_due), if dirty => break None,
                _ = tokio::time::sleep_until(tick_due) => break None,
            }
        };
        let Some(event) = event else {
            continue;
        };
        let mut app = app_arc.lock().await;
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match handle_key_event(&mut app, key, app_arc.clone()) {
                    UiOutcome::Continue => {}
                    UiOutcome::Quit => return Ok(None),
                    UiOutcome::Update(update) => return Ok(Some(update)),
                }
            }
            Event::Paste(text) => handle_paste(&mut app, &text),
            _ => {}
        }
    }
}

fn handle_key_event(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) -> UiOutcome {
    // A modified letter must never fall through to an unrelated playback mutation.
    if key.modifiers.contains(event::KeyModifiers::ALT) {
        return UiOutcome::Continue;
    }
    if key.modifiers.contains(event::KeyModifiers::CONTROL) {
        let editing = app.input_mode == InputMode::Editing || app.is_settings_editing;
        let recognized = key.code == KeyCode::Char('c')
            || (editing && key.code == KeyCode::Char('u'))
            || (!editing
                && key.code == KeyCode::Char('p')
                && matches!(
                    app.view,
                    View::Main | View::Lyrics | View::Settings | View::Debug
                ))
            || (!editing && key.code == KeyCode::Char('d') && app.view == View::Main);
        if !recognized {
            return UiOutcome::Continue;
        }
    }
    if should_quit(app, key) {
        app.cancel_login();
        return UiOutcome::Quit;
    }
    if app.login_cancel.is_some() && key.code == KeyCode::Esc {
        app.cancel_login();
        return UiOutcome::Continue;
    }
    if app.input_mode == InputMode::Editing {
        handle_editing_keys(app, key, app_arc);
        return UiOutcome::Continue;
    }
    if app.is_settings_editing {
        handle_settings_keys(app, key, app_arc);
        return UiOutcome::Continue;
    }
    match app.view {
        View::Menu => {
            return if handle_palette_keys(app, key, app_arc) {
                UiOutcome::Quit
            } else {
                UiOutcome::Continue
            };
        }
        View::Help => {
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => app.view = app.overlay_return,
                KeyCode::Down | KeyCode::Char('j') => {
                    app.help_scroll = app.help_scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.help_scroll = app.help_scroll.saturating_sub(1)
                }
                KeyCode::Home => app.help_scroll = 0,
                KeyCode::End => app.help_scroll = u16::MAX,
                _ => {}
            }
            return UiOutcome::Continue;
        }
        View::ConfirmClear => {
            handle_clear_keys(app, key, app_arc);
            return UiOutcome::Continue;
        }
        View::TrackDetails => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Backspace) {
                app.view = app.overlay_return;
            }
            return UiOutcome::Continue;
        }
        View::UpdateFound => {
            return if let Some(update) = handle_update_keys(app, key) {
                UiOutcome::Update(update)
            } else {
                UiOutcome::Continue
            };
        }
        _ => {}
    }
    if matches!(
        app.view,
        View::Main | View::Lyrics | View::Settings | View::Debug
    ) {
        match key.code {
            KeyCode::Char(':') => {
                app.open_overlay(View::Menu);
                return UiOutcome::Continue;
            }
            KeyCode::Char('p') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                app.open_overlay(View::Menu);
                return UiOutcome::Continue;
            }
            KeyCode::Char('?') => {
                app.open_overlay(View::Help);
                return UiOutcome::Continue;
            }
            KeyCode::Char('u') if app.update_info.is_some() => {
                execute_action(Action::Update, app, app_arc);
                return UiOutcome::Continue;
            }
            KeyCode::Esc
                if app.view == View::Main
                    && (app.error_message.is_some()
                        || app.queue_error.is_some()
                        || app.notice.is_some()
                        || app.update_notice) =>
            {
                app.error_message = None;
                app.queue_error = None;
                app.notice = None;
                app.update_notice = false;
                return UiOutcome::Continue;
            }
            _ => {}
        }
    }
    if matches!(key.code, KeyCode::Char('1' | '2' | '3' | '4')) {
        app.cancel_login();
    }
    match key.code {
        KeyCode::Char('1') => {
            app.view = View::Main;
            return UiOutcome::Continue;
        }
        KeyCode::Char('2') => {
            execute_action(Action::Lyrics, app, app_arc);
            return UiOutcome::Continue;
        }
        KeyCode::Char('3') => {
            execute_action(Action::Settings, app, app_arc);
            return UiOutcome::Continue;
        }
        KeyCode::Char('4') => {
            app.view = View::Debug;
            return UiOutcome::Continue;
        }
        _ => {}
    }
    match app.view {
        View::Main => handle_player_keys(app, key, app_arc),
        View::Lyrics => handle_lyrics_keys(app, key, app_arc),
        View::Settings => handle_settings_keys(app, key, app_arc),
        View::Debug => handle_debug_keys(app, key),
        View::FilterMenu => handle_filter_menu_keys(app, key, app_arc),
        View::AuthMenu => handle_auth_menu_keys(app, key, app_arc),
        View::AuthResult => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Backspace) {
                app.view = View::AuthMenu;
            }
        }
        View::AppInfo => {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Enter | KeyCode::Backspace | KeyCode::Char('i' | 'ш')
            ) {
                app.view = app.overlay_return;
            }
        }
        View::LoginRequired => match key.code {
            KeyCode::Char('?') if !app.auth_loading => app.open_overlay(View::Help),
            KeyCode::Enter => {
                start_login(app, app_arc);
            }
            KeyCode::Char('\\') => {
                app.cancel_login();
                app.settings_input = app.base_url.clone();
                app.view = View::Settings;
            }
            _ => {}
        },
        View::Menu | View::Help | View::ConfirmClear | View::UpdateFound | View::TrackDetails => {}
    }
    UiOutcome::Continue
}

fn input_char(c: char) -> bool {
    !c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Both allocation and scanning are bounded, including control-only pastes.
fn insert_paste(text: &mut String, cursor: &mut usize, paste: &str, limit: usize) -> bool {
    *cursor = cursor_boundary(text, *cursor);
    let end = cursor_boundary(paste, limit.saturating_mul(4).saturating_add(64));
    let mut remaining = limit.saturating_sub(text.len());
    let mut clean = String::new();
    let mut truncated = end < paste.len();
    for c in paste[..end].chars().filter(|c| input_char(*c)) {
        if c.len_utf8() > remaining {
            truncated = true;
            break;
        }
        remaining -= c.len_utf8();
        clean.push(c);
    }
    text.insert_str(*cursor, &clean);
    *cursor += clean.len();
    truncated
}

fn handle_paste(app: &mut App, text: &str) {
    if app.input_mode == InputMode::Editing {
        let before = app.input.len();
        let truncated = insert_paste(&mut app.input, &mut app.input_cursor, text, MAX_INPUT_BYTES);
        if truncated || app.input.len() != before {
            app.input_truncated = truncated;
            app.input_error = truncated
                .then(|| "Input limited to 8192 bytes. Shorten the request before adding.".into());
        }
    } else if app.view == View::Menu {
        let mut cursor = app.palette_query.len();
        insert_paste(&mut app.palette_query, &mut cursor, text, MAX_PALETTE_BYTES);
        app.menu_state.select(Some(0));
    } else if app.is_settings_editing {
        match app.settings_field {
            SettingsField::Host => {
                let before = app.settings_input.len();
                let truncated = insert_paste(
                    &mut app.settings_input,
                    &mut app.settings_cursor,
                    text,
                    MAX_HOST_BYTES,
                );
                if truncated || app.settings_input.len() != before {
                    app.settings_truncated = truncated;
                }
                if app.settings_truncated {
                    app.error_message =
                        Some("Server URL limited to 2048 bytes. Edit it before saving.".into());
                }
            }
            SettingsField::Offset => {
                for c in text.chars().take(80).filter(|c| input_char(*c)) {
                    edit_text(
                        &mut app.offset_input,
                        &mut app.settings_cursor,
                        event::KeyEvent::new(KeyCode::Char(c), event::KeyModifiers::NONE),
                        |c, text, cursor| {
                            text.len() < 20
                                && (c.is_ascii_digit()
                                    || (c == '-' && cursor == 0 && !text.contains('-')))
                        },
                    );
                }
            }
            _ => {}
        }
    }
}

fn handle_editing_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    match key.code {
        KeyCode::Enter => {
            if app.input_truncated {
                return;
            }
            if app.input.trim().is_empty() {
                app.input_error = Some("Enter a song name or URL before adding.".into());
                return;
            }
            if app.add_pending.is_some() {
                return;
            }
            let query = app.input.clone();
            app.add_pending = Some(query.clone());
            app.input_error = None;
            app.spawn(async_play_track(app_arc, query));
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.input_cursor = 0;
            app.input_error = None;
            app.input_truncated = false;
        }
        _ => {
            let before = app.input.clone();
            edit_text(&mut app.input, &mut app.input_cursor, key, |_, _, _| true);
            if app.input != before
                || (key.code == KeyCode::Char('u')
                    && key.modifiers.contains(event::KeyModifiers::CONTROL))
            {
                app.input_error = None;
                app.input_truncated = false;
            } else if matches!(key.code, KeyCode::Char(c) if input_char(c) && app.input.len() + c.len_utf8() > MAX_INPUT_BYTES)
            {
                app.input_truncated = true;
                app.input_error =
                    Some("Input limited to 8192 bytes. Shorten the request before adding.".into());
            }
        }
    }
}

fn edit_text(
    text: &mut String,
    cursor: &mut usize,
    key: event::KeyEvent,
    mut accepts: impl FnMut(char, &str, usize) -> bool,
) {
    *cursor = cursor_boundary(text, *cursor);
    match key.code {
        KeyCode::Left => {
            *cursor = text[..*cursor]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(i, _)| i)
        }
        KeyCode::Right => {
            if let Some(grapheme) = text[*cursor..].graphemes(true).next() {
                *cursor += grapheme.len();
            }
        }
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = text.len(),
        KeyCode::Backspace if *cursor > 0 => {
            let start = text[..*cursor]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(i, _)| i);
            text.drain(start..*cursor);
            *cursor = start;
        }
        KeyCode::Delete if *cursor < text.len() => {
            let end = *cursor + text[*cursor..].graphemes(true).next().map_or(0, str::len);
            text.drain(*cursor..end);
        }
        KeyCode::Char('u') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
            text.clear();
            *cursor = 0;
        }
        KeyCode::Char(c)
            if input_char(c)
                && text.len().saturating_add(c.len_utf8()) <= MAX_INPUT_BYTES
                && !key
                    .modifiers
                    .intersects(event::KeyModifiers::CONTROL | event::KeyModifiers::ALT)
                && accepts(c, text, *cursor) =>
        {
            text.insert(*cursor, c);
            *cursor += c.len_utf8();
        }
        _ => {}
    }
}

fn handle_update_keys(
    app: &mut App,
    key: event::KeyEvent,
) -> Option<(String, Vec<api::GiteaAsset>)> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('н') | KeyCode::Char('Н') => {
            app.update_info.clone()
        }
        KeyCode::Char('n')
        | KeyCode::Char('N')
        | KeyCode::Char('т')
        | KeyCode::Char('Т')
        | KeyCode::Esc => {
            app.view = app.overlay_return;
            None
        }
        _ => None,
    }
}

fn handle_player_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    let action = match key.code {
        KeyCode::Tab | KeyCode::BackTab => {
            app.focus = if app.focus == PlayerFocus::Queue {
                PlayerFocus::Playback
            } else {
                PlayerFocus::Queue
            };
            return;
        }
        KeyCode::Down | KeyCode::Char('j' | 'о') => {
            app.move_queue(1);
            return;
        }
        KeyCode::Up | KeyCode::Char('k' | 'л') => {
            app.move_queue(-1);
            return;
        }
        KeyCode::PageDown => {
            app.move_queue(app.queue_page_size as isize);
            return;
        }
        KeyCode::PageUp => {
            app.move_queue(-(app.queue_page_size as isize));
            return;
        }
        KeyCode::Home => {
            app.move_queue(-(app.queue.len() as isize));
            return;
        }
        KeyCode::End => {
            app.move_queue(app.queue.len() as isize);
            return;
        }
        KeyCode::Char('[' | ']') => {
            let next = key.code == KeyCode::Char(']');
            let target = if next {
                app.queue_target_offset.saturating_add(QUEUE_LIMIT)
            } else {
                app.queue_target_offset.saturating_sub(QUEUE_LIMIT)
            };
            if next && app.queue_total.is_some_and(|total| target >= total) {
                app.notify("Last queue page");
                return;
            }
            if next && app.queue_total.is_none() && app.queue.len() < QUEUE_LIMIT {
                app.notify("No more loaded queue pages");
                return;
            }
            if target != app.queue_target_offset {
                app.queue_target_offset = target;
                app.spawn(async_fetch_queue(app_arc));
            }
            return;
        }
        KeyCode::Enter => {
            if app.selected_track().is_some() {
                app.open_overlay(View::TrackDetails);
            }
            return;
        }
        KeyCode::Char('a' | '/') => {
            app.begin_search("");
            return;
        }
        KeyCode::Char(' ' | 'p' | 'з') => Action::Pause,
        KeyCode::Char('r' | 'к') => Action::Refresh,
        KeyCode::Char('s' | 'ы' | 'і') => Action::Skip,
        KeyCode::Char('w' | 'ц') => Action::Stop,
        KeyCode::Char('c' | 'с') => Action::Clear,
        KeyCode::Char('v') => Action::Visualizer,
        KeyCode::Char('i' | 'ш') => Action::Info,
        KeyCode::Char('d') if key.modifiers.contains(event::KeyModifiers::CONTROL) => Action::Debug,
        KeyCode::Char('l' | 'д') => {
            if !app.loop_mode_known {
                app.notify("Repeat state unknown. Refresh or choose a mode in : actions.");
                return;
            }
            match app.loop_mode.as_str() {
                "off" => Action::LoopTrack,
                "track" => Action::LoopQueue,
                _ => Action::LoopOff,
            }
        }
        _ => return,
    };
    execute_action(action, app, app_arc);
}

fn handle_lyrics_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.view = View::Main,
        KeyCode::Down | KeyCode::Char('j' | 'о') => {
            app.lyrics_scroll = app
                .lyrics_scroll
                .saturating_add(1)
                .min(app.lyrics_max_scroll)
        }
        KeyCode::Up | KeyCode::Char('k' | 'л') => {
            app.lyrics_scroll = app.lyrics_scroll.saturating_sub(1)
        }
        KeyCode::PageDown => {
            app.lyrics_scroll = app
                .lyrics_scroll
                .saturating_add(app.lyrics_page_size)
                .min(app.lyrics_max_scroll)
        }
        KeyCode::PageUp => {
            app.lyrics_scroll = app.lyrics_scroll.saturating_sub(app.lyrics_page_size)
        }
        KeyCode::Home => app.lyrics_scroll = 0,
        KeyCode::End => app.lyrics_scroll = app.lyrics_max_scroll,
        KeyCode::Char('r') if !app.lyrics_loading => {
            app.spawn(async_fetch_lyrics(app_arc));
        }
        _ => {}
    }
}

fn handle_settings_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    if app.is_settings_editing {
        let before = if app.settings_field == SettingsField::Host {
            app.settings_input.clone()
        } else {
            app.offset_input.clone()
        };
        match key.code {
            KeyCode::Enter => {
                if !save_app_settings(app) {
                    return;
                }
                app.is_settings_editing = false;
                let canonical = crate::credentials::server_identity(&app.settings_input)
                    .expect("validated server URL");
                let host_changed = app.base_url != canonical;
                app.base_url = canonical.clone();
                app.settings_input = canonical;
                app.visualizer_offset =
                    app.offset_input.parse().expect("validated settings offset");
                if host_changed {
                    switch_server(app, app_arc);
                }
            }
            KeyCode::Esc => {
                app.settings_input = app.base_url.clone();
                app.offset_input = app.visualizer_offset.to_string();
                app.is_settings_editing = false;
                app.settings_truncated = false;
                app.error_message = None;
            }
            _ => match app.settings_field {
                SettingsField::Host => edit_text(
                    &mut app.settings_input,
                    &mut app.settings_cursor,
                    key,
                    |c, text, _| text.len().saturating_add(c.len_utf8()) <= MAX_HOST_BYTES,
                ),
                SettingsField::Offset => edit_text(
                    &mut app.offset_input,
                    &mut app.settings_cursor,
                    key,
                    |c, text, cursor| {
                        text.len() < 20
                            && (c.is_ascii_digit()
                                || (c == '-' && cursor == 0 && !text.contains('-')))
                    },
                ),
                _ => {}
            },
        }
        let after = if app.settings_field == SettingsField::Host {
            &app.settings_input
        } else {
            &app.offset_input
        };
        if after != &before
            || (key.code == KeyCode::Char('u')
                && key.modifiers.contains(event::KeyModifiers::CONTROL))
        {
            app.settings_truncated = false;
        } else if app.settings_field == SettingsField::Host
            && matches!(key.code, KeyCode::Char(c) if input_char(c) && before.len().saturating_add(c.len_utf8()) > MAX_HOST_BYTES)
            && !key
                .modifiers
                .intersects(event::KeyModifiers::CONTROL | event::KeyModifiers::ALT)
        {
            app.settings_truncated = true;
            app.error_message =
                Some("Server URL limited to 2048 bytes. Edit it before saving.".into());
        }
        return;
    }

    match key.code {
        KeyCode::Enter => match app.settings_field {
            SettingsField::Host | SettingsField::Offset => {
                app.is_settings_editing = true;
                app.settings_truncated = false;
                app.settings_cursor = if app.settings_field == SettingsField::Host {
                    app.settings_input.len()
                } else {
                    app.offset_input.len()
                };
            }
            _ => {
                if save_app_settings(app) {
                    app.view = if app.token.is_none() {
                        View::LoginRequired
                    } else {
                        View::Main
                    };
                }
            }
        },
        KeyCode::Esc => {
            app.view = if app.token.is_none() {
                View::LoginRequired
            } else {
                View::Main
            };
        }
        KeyCode::Down | KeyCode::Tab => {
            app.settings_field = match app.settings_field {
                SettingsField::Host => SettingsField::Offset,
                SettingsField::Offset => SettingsField::Theme,
                SettingsField::Theme => SettingsField::VizStyle,
                SettingsField::VizStyle => SettingsField::Layout,
                SettingsField::Layout => SettingsField::Host,
            };
        }
        KeyCode::Up | KeyCode::BackTab => {
            app.settings_field = match app.settings_field {
                SettingsField::Host => SettingsField::Layout,
                SettingsField::Offset => SettingsField::Host,
                SettingsField::Theme => SettingsField::Offset,
                SettingsField::VizStyle => SettingsField::Theme,
                SettingsField::Layout => SettingsField::VizStyle,
            };
        }
        KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('д') => match app.settings_field {
            SettingsField::Theme => {
                app.theme = match app.theme.as_str() {
                    "Default" => "Midnight".to_string(),
                    "Midnight" => "Emerald".to_string(),
                    "Emerald" => "Ruby".to_string(),
                    "Ruby" => "Ocean".to_string(),
                    "Ocean" => "Synthwave".to_string(),
                    "Synthwave" => "Sepia".to_string(),
                    _ => "Default".to_string(),
                };
                save_app_settings(app);
            }
            SettingsField::VizStyle => {
                app.viz_style = match app.viz_style.as_str() {
                    "Bars" => "Blocky".to_string(),
                    "Blocky" => "Line".to_string(),
                    "Line" => "Wave".to_string(),
                    "Wave" => "Dots".to_string(),
                    _ => "Bars".to_string(),
                };
                save_app_settings(app);
            }
            SettingsField::Layout => {
                app.layout = match app.layout.as_str() {
                    "Desk" => "Queue".to_string(),
                    "Queue" => "Sidebar".to_string(),
                    "Sidebar" => "Studio".to_string(),
                    "Studio" => "Zen".to_string(),
                    "Zen" => "Desk".to_string(),
                    _ => "Desk".to_string(),
                };
                save_app_settings(app);
            }
            _ => {}
        },
        KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('р') => match app.settings_field {
            SettingsField::Theme => {
                app.theme = match app.theme.as_str() {
                    "Default" => "Sepia".to_string(),
                    "Midnight" => "Default".to_string(),
                    "Emerald" => "Midnight".to_string(),
                    "Ruby" => "Emerald".to_string(),
                    "Ocean" => "Ruby".to_string(),
                    "Synthwave" => "Ocean".to_string(),
                    "Sepia" => "Synthwave".to_string(),
                    _ => "Default".to_string(),
                };
                save_app_settings(app);
            }
            SettingsField::VizStyle => {
                app.viz_style = match app.viz_style.as_str() {
                    "Bars" => "Dots".to_string(),
                    "Blocky" => "Bars".to_string(),
                    "Line" => "Blocky".to_string(),
                    "Wave" => "Line".to_string(),
                    "Dots" => "Wave".to_string(),
                    _ => "Bars".to_string(),
                };
                save_app_settings(app);
            }
            SettingsField::Layout => {
                app.layout = match app.layout.as_str() {
                    "Desk" => "Zen".to_string(),
                    "Queue" => "Desk".to_string(),
                    "Sidebar" => "Queue".to_string(),
                    "Studio" => "Sidebar".to_string(),
                    "Zen" => "Studio".to_string(),
                    _ => "Desk".to_string(),
                };
                save_app_settings(app);
            }
            _ => {}
        },
        _ => {}
    }
}

fn draft_settings(app: &App) -> Result<api::Settings> {
    anyhow::ensure!(
        !app.settings_truncated,
        "Server URL was truncated. Edit it before saving."
    );
    let base_url = crate::credentials::server_identity(&app.settings_input)?;
    let visualizer_offset = app
        .offset_input
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("visualizer offset must be a valid integer"))?;
    Ok(api::Settings {
        base_url,
        visualizer_offset,
        theme: app.theme.clone(),
        visualizer_style: app.viz_style.clone(),
        layout: app.layout.clone(),
    })
}

fn save_app_settings(app: &mut App) -> bool {
    match draft_settings(app).and_then(|settings| api::save_settings(&settings)) {
        Ok(()) => {
            app.error_message = None;
            true
        }
        Err(error) => {
            app.error_message = Some(format!("Could not save settings: {error:#}"));
            false
        }
    }
}

fn handle_debug_keys(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Char('s') | KeyCode::Char('ы') => app.save_spectrogram(),
        KeyCode::Esc | KeyCode::Backspace => {
            app.view = if app.token.is_none() {
                View::LoginRequired
            } else {
                View::Main
            };
        }
        _ => {}
    }
}

fn handle_filter_menu_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    match key.code {
        KeyCode::Esc => app.view = app.overlay_return,
        KeyCode::Backspace => {
            app.view = app.overlay_return;
            app.open_overlay(View::Menu);
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
            let i = match app.filter_state.selected() {
                Some(i) => {
                    if i >= app.filter_items.len() - 1 {
                        0
                    } else {
                        i + 1
                    }
                }
                None => 0,
            };
            app.filter_state.select(Some(i));
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
            let i = match app.filter_state.selected() {
                Some(i) => {
                    if i == 0 {
                        app.filter_items.len() - 1
                    } else {
                        i - 1
                    }
                }
                None => 0,
            };
            app.filter_state.select(Some(i));
        }
        KeyCode::Enter => {
            if let Some(idx) = app.filter_state.selected() {
                let style = app.filter_items[idx];
                let filters = get_filters_for_style(style);
                let payload = FilterPayload {
                    action: "filter",
                    guild_id: app.guild_id.clone(),
                    user_id: app.user_id.clone(),
                    filters,
                };
                app.spawn(async_simple_command(
                    app_arc,
                    "/webhook/audio".to_string(),
                    payload,
                ));
                app.view = app.overlay_return;
            }
        }
        _ => {}
    }
}

fn handle_auth_menu_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    match key.code {
        KeyCode::Esc | KeyCode::Tab => app.view = app.overlay_return,
        KeyCode::Backspace => {
            app.view = app.overlay_return;
            app.open_overlay(View::Menu);
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
            let i = match app.auth_menu_state.selected() {
                Some(i) => {
                    if i >= app.auth_menu_items.len() - 1 {
                        0
                    } else {
                        i + 1
                    }
                }
                None => 0,
            };
            app.auth_menu_state.select(Some(i));
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
            let i = match app.auth_menu_state.selected() {
                Some(i) => {
                    if i == 0 {
                        app.auth_menu_items.len() - 1
                    } else {
                        i - 1
                    }
                }
                None => 0,
            };
            app.auth_menu_state.select(Some(i));
        }
        KeyCode::Enter => {
            if let Some(idx) = app.auth_menu_state.selected() {
                match app.auth_menu_items[idx] {
                    "Login" => start_login(app, app_arc),
                    "Signout" => start_signout(app, app_arc),
                    "Info" => show_auth_info(app, app_arc),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn get_filters_for_style(style: &str) -> AudioFilters {
    match style.to_lowercase().as_str() {
        "clear" => AudioFilters::default(),
        "bassboost" => AudioFilters {
            equalizer: Some(vec![
                EqualizerBand { band: 0, gain: 0.2 },
                EqualizerBand {
                    band: 1,
                    gain: 0.15,
                },
                EqualizerBand { band: 2, gain: 0.1 },
                EqualizerBand {
                    band: 3,
                    gain: 0.05,
                },
                EqualizerBand { band: 4, gain: 0.0 },
                EqualizerBand {
                    band: 5,
                    gain: -0.05,
                },
            ]),
            ..Default::default()
        },
        "soft" => AudioFilters {
            low_pass: Some(LowPassOptions {
                smoothing: Some(20.0),
            }),
            ..Default::default()
        },
        "nightcore" => AudioFilters {
            timescale: Some(TimescaleOptions {
                speed: Some(1.1),
                pitch: Some(1.1),
                rate: Some(1.0),
            }),
            ..Default::default()
        },
        "vaporwave" => AudioFilters {
            timescale: Some(TimescaleOptions {
                speed: Some(0.85),
                pitch: Some(0.8),
                rate: Some(1.0),
            }),
            ..Default::default()
        },
        "8d" => AudioFilters {
            rotation: Some(RotationOptions {
                rotation_hz: Some(0.2),
            }),
            ..Default::default()
        },
        "tremolo" => AudioFilters {
            tremolo: Some(TremoloOptions {
                frequency: Some(2.0),
                depth: Some(0.5),
            }),
            ..Default::default()
        },
        "vibrato" => AudioFilters {
            vibrato: Some(VibratoOptions {
                frequency: Some(2.0),
                depth: Some(0.5),
            }),
            ..Default::default()
        },
        "karaoke" => AudioFilters {
            karaoke: Some(KaraokeOptions {
                level: Some(1.0),
                mono_level: Some(1.0),
                filter_band: Some(220.0),
                filter_width: Some(100.0),
            }),
            ..Default::default()
        },
        _ => AudioFilters::default(),
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    render_ui(f, app);
    if app.monochrome {
        for cell in &mut f.buffer_mut().content {
            cell.set_fg(Color::Reset).set_bg(Color::Reset);
        }
    }
}

fn render_ui(f: &mut Frame, app: &mut App) {
    app.artwork.last_area = None;
    app.visualizer_visible = false;
    let theme = get_theme(&app.theme);
    f.render_widget(Block::default().bg(theme.bg).fg(theme.text()), f.area());
    if f.area().width < 24 || f.area().height < 8 {
        f.render_widget(
            Paragraph::new("Enlarge terminal\n24×8 minimum\nCtrl+c to quit")
                .style(Style::default().fg(theme.text_secondary)),
            f.area(),
        );
        return;
    }
    if app.content_view() == View::LoginRequired {
        views::welcome(f, app, &theme);
        if app.view == View::Help {
            views::sign_in_help(f, app, &theme);
        }
        return;
    }
    let area = f.area();
    let margin = u16::from(area.width >= 50);
    let area = Rect::new(
        area.x + margin,
        area.y,
        area.width.saturating_sub(margin * 2),
        area.height,
    );
    let spaced = app.content_view() == View::Main
        && app.layout == "Desk"
        && area.width >= 78
        && area.height >= 24;
    let regions = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(u16::from(spaced)),
        Constraint::Min(0),
        Constraint::Length(u16::from(spaced)),
        Constraint::Length(2),
    ])
    .split(area);
    let rows = [regions[0], regions[2], regions[4]];
    let content = app.content_view();
    render_header(f, app, &theme, content, rows[0]);
    match content {
        View::Settings => views::settings(f, app, &theme, rows[1]),
        View::Lyrics => views::lyrics(f, app, &theme, rows[1]),
        View::Debug => views::diagnostics(f, app, &theme, rows[1]),
        _ => render_player_ui(f, app, &theme, rows[1]),
    }
    render_footer(f, app, &theme, rows[2]);
    if app.input_mode == InputMode::Editing {
        views::track_input(f, app, &theme);
    }
    match app.view {
        View::Menu => render_palette(f, app, &theme),
        View::Help => render_help(f, app, &theme),
        View::ConfirmClear => render_clear_confirmation(f, app, &theme),
        View::FilterMenu => views::menu(f, app, &theme, false),
        View::AuthMenu => views::menu(f, app, &theme, true),
        View::AuthResult => views::account_info(f, app, &theme),
        View::AppInfo => views::about(f, &theme),
        View::TrackDetails => views::track_details(f, app, &theme),
        View::UpdateFound => views::update(f, app, &theme),
        _ => {}
    }
}

#[cfg(test)]
mod desk_tests;
#[cfg(test)]
mod hardening_tests;
#[cfg(test)]
mod polish_tests;
#[cfg(test)]
mod refinement_tests;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;

fn redacted_url(url: &Url) -> String {
    let mut safe = url.clone();
    safe.set_query(None);
    safe.set_fragment(None);
    let _ = safe.set_username("");
    let _ = safe.set_password(None);
    safe.to_string()
}

// Keep the insertion cursor visible and measure terminal cells, not UTF-8 bytes.
fn input_tail(input: &str, width: u16) -> &str {
    let mut cells = 0;
    for (start, grapheme) in input.grapheme_indices(true).rev() {
        cells += grapheme.width();
        if cells > usize::from(width) {
            return &input[start + grapheme.len()..];
        }
    }
    input
}

fn cursor_boundary(text: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

fn input_window(input: &str, cursor: usize, width: u16) -> (&str, u16) {
    let mut cursor = cursor.min(input.len());
    while !input.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let prefix = input_tail(&input[..cursor], width.saturating_sub(1));
    let start = cursor - prefix.len();
    let column = prefix.width() as u16;
    let mut cells = usize::from(column);
    let mut end = cursor;
    for grapheme in input[cursor..].graphemes(true) {
        cells += grapheme.width();
        if cells > usize::from(width) {
            break;
        }
        end += grapheme.len();
    }
    (&input[start..end], column)
}
