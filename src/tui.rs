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
    style::{Color, Style},
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
use tokio::sync::Mutex;
use tokio::time::interval;
#[cfg(test)]
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::protocol::WebSocketConfig,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, protocol::Message},
};
use tokio_util::sync::CancellationToken;
use url::Url;

mod actions;
mod authentication;
mod chrome;
mod design;
mod player;
mod views;

use actions::{Action, execute as execute_action, handle_clear_keys, handle_palette_keys};
use authentication::{show_auth_info, start_login, start_signout, switch_server};
use chrome::{
    popup_rect, render_clear_confirmation, render_footer, render_header, render_help,
    render_palette,
};
use player::render_player_ui;

// Theme Colors
struct Theme {
    bg: Color,
    border: Color,
    primary: Color,
    highlight: Color,
    text_secondary: Color,
}

fn get_theme(name: &str) -> Theme {
    match name {
        "Midnight" => Theme {
            bg: Color::Rgb(5, 5, 15),
            border: Color::Rgb(40, 40, 60),
            primary: Color::Rgb(100, 100, 255),
            highlight: Color::Rgb(150, 150, 255),
            text_secondary: Color::Rgb(190, 197, 214),
        },
        "Emerald" => Theme {
            bg: Color::Rgb(10, 18, 17),
            border: Color::Rgb(60, 84, 79),
            primary: Color::Rgb(100, 196, 154),
            highlight: Color::Rgb(147, 224, 179),
            text_secondary: Color::Rgb(192, 207, 201),
        },
        "Ruby" => Theme {
            bg: Color::Rgb(15, 5, 5),
            border: Color::Rgb(60, 40, 40),
            primary: Color::Rgb(200, 50, 50),
            highlight: Color::Rgb(255, 100, 100),
            text_secondary: Color::Rgb(212, 194, 201),
        },
        "Ocean" => Theme {
            bg: Color::Rgb(5, 10, 20),
            border: Color::Rgb(40, 60, 100),
            primary: Color::Rgb(50, 150, 255),
            highlight: Color::Rgb(100, 200, 255),
            text_secondary: Color::Rgb(190, 205, 222),
        },
        "Synthwave" => Theme {
            bg: Color::Rgb(20, 10, 30),
            border: Color::Rgb(100, 40, 100),
            primary: Color::Rgb(255, 50, 255),
            highlight: Color::Rgb(255, 150, 50), // Orange highlight
            text_secondary: Color::Rgb(209, 193, 226),
        },
        "Sepia" => Theme {
            bg: Color::Rgb(30, 25, 20),
            border: Color::Rgb(80, 70, 60),
            primary: Color::Rgb(180, 140, 100),
            highlight: Color::Rgb(220, 180, 140),
            text_secondary: Color::Rgb(211, 199, 185),
        },
        _ => Theme {
            // Default Jorik Purple
            bg: Color::Rgb(15, 15, 25),
            border: Color::Rgb(60, 60, 80),
            primary: JORIK_PURPLE,
            highlight: JORIK_HIGHLIGHT,
            text_secondary: Color::Rgb(193, 198, 214),
        },
    }
}

// Approx color from the logo
const JORIK_PURPLE: Color = Color::Rgb(130, 110, 230); // Soft purple/indigo
const JORIK_HIGHLIGHT: Color = Color::Rgb(160, 140, 250);

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

    queue: Vec<String>,
    queue_state: ListState,
    queue_page_size: usize,
    focus: PlayerFocus,
    visualizer_enabled: bool,
    current_track: Option<String>,
    error_message: Option<String>,
    queue_error: Option<String>,
    fatal_error: Option<String>,
    loop_mode: String, // "off", "track", "queue"
    is_loading: bool,

    input: String,
    input_cursor: usize,
    input_error: Option<String>,
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

    settings_input: String,
    offset_input: String,
    theme: String,
    viz_style: String,
    layout: String,
    settings_field: SettingsField,
    is_settings_editing: bool,
    settings_cursor: usize,
    needs_reconnect: bool,
    visualizer_offset: i64,

    update_info: Option<(String, Vec<api::GiteaAsset>)>,
    update_notice: bool,
    notice: Option<(String, Instant)>,

    debug_logs: Vec<String>,
    ws_connected: bool,
    ws_connecting: bool,
    ws_sender: Option<tokio::sync::mpsc::Sender<Message>>,
    pending_commands: HashMap<String, Instant>,
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

        Self {
            client,
            base_url: settings.base_url.clone(),
            token,
            guild_id,
            user_id,
            queue: Vec::new(),
            queue_state: ListState::default(),
            queue_page_size: 10,
            focus: PlayerFocus::Queue,
            visualizer_enabled: true,
            current_track: None,
            error_message: None,
            queue_error: None,
            fatal_error: None,
            loop_mode: "off".to_string(),
            is_loading: false,
            input: String::new(),
            input_cursor: 0,
            input_error: None,
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
            settings_input: settings.base_url.clone(),
            offset_input: settings.visualizer_offset.to_string(),
            theme: settings.theme,
            viz_style: settings.visualizer_style,
            layout: settings.layout,
            settings_field: SettingsField::Host,
            is_settings_editing: false,
            settings_cursor: 0,
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
            self.is_loading = false;
            self.auth_info_text = Some("Login cancelled.".to_string());
        }
    }

    fn clear_session(&mut self) {
        self.token = None;
        self.guild_id = None;
        self.user_id = None;
        self.queue.clear();
        self.queue_state.select(None);
        self.current_track = None;
        self.lyrics_text = None;
        self.lyrics_loading = false;
        self.lyrics_scroll = 0;
        self.spectrogram = None;
        self.smoothed_bars.fill(0.0);
        self.elapsed_ms = 0;
        self.duration_ms = 0;
        self.has_playback_state = false;
        self.paused = true;
        self.fatal_error = None;
        self.queue_error = None;
        self.auth_info_text = None;
        self.is_loading = false;
        self.debug_logs.clear();
        self.disconnect_ws();
        self.error_message = None;
        self.needs_reconnect = true;
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
        ) {
            self.overlay_return
        } else {
            self.view
        }
    }

    fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
    }

    fn offer_update(&mut self, update: (String, Vec<api::GiteaAsset>)) {
        self.update_info = Some(update);
        self.update_notice = true;
    }

    fn begin_search(&mut self, initial: &str) {
        self.input = initial.to_string();
        self.input_cursor = self.input.len();
        self.input_error = None;
        self.input_mode = InputMode::Editing;
    }

    fn send_ws_action<T: serde::Serialize>(&mut self, payload: &T) -> bool {
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
        self.pending_commands.insert(id, Instant::now());
        true
    }

    fn finish_ws_action(&mut self, event: &WsEvent) {
        let Some(id) = event.id.as_deref() else {
            return;
        };
        if self.pending_commands.remove(id).is_none() {
            return;
        }
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
            self.error_message = Some(format!("Command failed: {detail}"));
        }
    }

    fn disconnect_ws(&mut self) {
        // An unsuccessful upgrade must not erase state just fetched over HTTP.
        if self.ws_connected {
            self.has_playback_state = false;
        }
        self.ws_connected = false;
        self.ws_connecting = false;
        if !self.pending_commands.is_empty() {
            self.pending_commands.clear();
            self.error_message = Some(
                "Connection lost before command confirmation. Refresh before retrying.".to_string(),
            );
        }
    }

    fn is_busy(&self) -> bool {
        self.is_loading || self.lyrics_loading || !self.pending_commands.is_empty()
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
        self.elapsed_ms = playback.elapsed_ms;
        self.duration_ms = playback.duration_ms;
        self.paused = playback.paused;
        self.has_playback_state = true;
        self.last_state_update = Instant::now();
        if let Some(spec) = playback.spectrogram {
            self.spectrogram = Some(spec);
        }
    }

    fn parse_queue_response(&mut self, json: &Value) {
        let previous_track = self.current_track.clone();
        // Handle nested queue object if present
        let target = if let Some(queue) = json.get("queue") {
            queue
        } else {
            json
        };

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

        if let Some(current) = target.get("current").and_then(|v| v.as_object()) {
            let title = current
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let author = current.get("author").and_then(|v| v.as_str()).unwrap_or("");
            self.current_track = Some(crate::output::safe_text(
                &format!("{} - {}", title, author),
                self.token.as_deref(),
            ));
        } else {
            // Only clear current_track if we are sure we are looking at a queue object
            if target.get("current").is_some() || target.get("upcoming").is_some() {
                self.current_track = None;
            }
        }

        if let Some(upcoming) = target.get("upcoming").and_then(|v| v.as_array()) {
            self.queue.clear();
            for item in upcoming {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");
                let author = item.get("author").and_then(|v| v.as_str()).unwrap_or("");
                self.queue.push(crate::output::safe_text(
                    &format!("{} - {}", title, author),
                    self.token.as_deref(),
                ));
            }
        }
        self.clamp_queue_selection();
        if self.current_track != previous_track {
            self.lyrics_text = None;
            self.lyrics_scroll = 0;
            self.lyrics_max_scroll = 0;
            self.has_playback_state = false;
            self.elapsed_ms = 0;
            self.duration_ms = 0;
            self.paused = true;
            self.spectrogram = None;
            self.smoothed_bars.fill(0.0);
            self.last_state_update = Instant::now();
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
        let pending_count = self.pending_commands.len();
        self.pending_commands
            .retain(|_, started| started.elapsed() < Duration::from_secs(10));
        if self.pending_commands.len() < pending_count {
            self.error_message =
                Some("Command confirmation timed out. Refresh before retrying.".to_string());
        }
        if self.current_track.is_some() && !self.paused {
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

                        // Factors adjusted for 60fps
                        if scaled_target > current {
                            self.smoothed_bars[i] = current + (scaled_target - current) * 0.4;
                        } else {
                            self.smoothed_bars[i] = current - (current - scaled_target) * 0.15;
                        }
                    }
                }
            }
        } else {
            self.last_state_update = Instant::now();
            // Fade out bars when idle
            for i in 0..64 {
                self.smoothed_bars[i] *= 0.95;
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
    let (client, base_url, token, payload) = {
        let mut app = app_arc.lock().await;
        if app.token.is_none() {
            return;
        }
        app.is_loading = true;
        let payload = QueuePayload {
            action: "queue",
            guild_id: app.guild_id.clone(),
            user_id: app.user_id.clone(),
            limit: 20,
            offset: 0,
        };
        (
            app.client.clone(),
            app.base_url.clone(),
            app.token.clone(),
            payload,
        )
    };

    let mut req = client
        .post(api::build_url(&base_url, "/webhook/audio"))
        .json(&payload);
    if let Some(bearer) = &token {
        req = req.bearer_auth(bearer);
    }
    // Read and decode the entire response before taking the UI lock.
    let result = request_json(req).await;
    let mut app = app_arc.lock().await;
    if app.base_url != base_url || app.token != token {
        return;
    }
    app.is_loading = false;
    match result {
        Ok(json) => {
            app.parse_queue_response(&json);
            app.queue_error = None;
            app.fatal_error = None;
        }
        Err(error) => {
            if error.contains("user_not_in_voice_channel_or_guild_unknown") {
                app.fatal_error = Some(
                    "User not in voice channel or guild unknown.\n\nPress 'r' to reload or 'q' to quit.".to_string()
                );
            } else {
                app.queue_error = Some(error);
            }
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
    async_simple_command(app_arc, "/webhook/audio".to_string(), payload).await;
}

async fn async_fetch_lyrics(app_arc: Arc<Mutex<App>>) {
    let (client, base_url, token, payload, track) = {
        let mut app = app_arc.lock().await;
        if app.lyrics_loading {
            return;
        }
        if app.token.is_none() {
            app.lyrics_text = Some("Log in to this server first.".to_string());
            return;
        }
        app.is_loading = true;
        app.lyrics_loading = true;
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
    if app.base_url != base_url || app.token != token {
        return;
    }
    app.lyrics_scroll = 0;
    app.is_loading = false;
    app.lyrics_loading = false;
    if app.current_track != track {
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
    let (client, base_url, token) = {
        let mut app = app_arc.lock().await;
        if app.token.is_none() {
            app.error_message = Some("Log in to this server first.".to_string());
            return;
        }
        app.error_message = None;
        if endpoint == "/webhook/audio" && app.send_ws_action(&payload) {
            return;
        }
        app.is_loading = true;
        (app.client.clone(), app.base_url.clone(), app.token.clone())
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
        if app.base_url != base_url || app.token != token {
            return;
        }
        app.is_loading = false;
        if let Err(error) = result {
            app.error_message = Some(error);
            return;
        }
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
        let (base_url, token, guild_id) = {
            let app = app_arc.lock().await;
            (
                app.base_url.clone(),
                app.token.clone(),
                app.guild_id.clone(),
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
            app.log(format!("WS Connecting to {}", redacted_url(&ws_url)));
            app.ws_connected = false;
            app.ws_connecting = true;
        }

        let request = match websocket_request(&base_url, &token) {
            Ok(request) => request,
            Err(_) => {
                let mut app = app_arc.lock().await;
                app.log("Could not build authenticated WebSocket request");
                drop(app);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let websocket_limits = WebSocketConfig::default()
            .max_message_size(Some(4 * 1024 * 1024))
            .max_frame_size(Some(4 * 1024 * 1024));
        match connect_async_with_config(request, Some(websocket_limits), false).await {
            Ok((mut ws_stream, _)) => {
                {
                    let mut app = app_arc.lock().await;
                    app.log("WS Connected");
                    app.ws_connected = true;
                    app.ws_connecting = false;
                }

                let sub = WsSubscribe {
                    event_type: "subscribe",
                    guild_id: guild_id.clone(),
                };
                if let Ok(json) = serde_json::to_string(&sub) {
                    let _ = ws_stream.send(Message::Text(json.into())).await;
                }

                let mut reconnect_tick = interval(Duration::from_millis(500));
                loop {
                    tokio::select! {
                        msg = ws_stream.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(event) = serde_json::from_str::<WsEvent>(&text) {
                                        let mut app = app_arc.lock().await;
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
                                                    if let Some(data) = &event.data {
                                                        app.parse_queue_response(data);
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
                                                        app.parse_queue_response(&data);
                                                    } else {
                                                        // Fallback to REST if data is missing
                                                        tokio::spawn(async_fetch_queue(app_arc.clone()));
                                                    }
                                                }
                                            }
                                            "track_start" | "track_end" | "player_update" => {
                                                if event.guild_id.as_deref() == app.guild_id.as_deref() {
                                                    app.log(format!("WS Event: {}, refreshing queue", event.event_type));
                                                    // Trigger a full REST refresh to get the latest queue state
                                                    tokio::spawn(async_fetch_queue(app_arc.clone()));
                                                }
                                            }
                                            "action_response" => {
                                                let success = event.success.unwrap_or(false);
                                                let id = event.id.as_deref().unwrap_or("unknown");
                                                app.log(format!("WS Action Response [{}]: success={}", id, success));
                                                app.finish_ws_action(&event);
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
                                if app.base_url != base_url || app.token.as_deref() != Some(token.as_str()) {
                                    break;
                                }
                            }
                            if let Err(e) = ws_stream.send(out_msg).await {
                                let mut app = app_arc.lock().await;
                                app.log(format!("WS Send Error: {}", e));
                                break;
                            }
                        }
                        _ = reconnect_tick.tick() => {
                            let mut app = app_arc.lock().await;
                            if app.needs_reconnect
                                || app.base_url != base_url
                                || app.token.as_deref() != Some(token.as_str())
                                || app.guild_id.as_deref() != Some(guild_id.as_str())
                            {
                                app.log("WS Forcing reconnect due to settings change");
                                app.needs_reconnect = false;
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let mut app = app_arc.lock().await;
                app.log(format!("WS Connection Failed: {}", e));
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

    // Initial fetch
    tokio::spawn(async_fetch_queue(app.clone()));
    tokio::spawn(spawn_websocket(app.clone(), ws_rx));

    let app_update = app.clone();
    tokio::spawn(async move {
        if let Some(update) = crate::updater::check_for_updates().await {
            let mut app = app_update.lock().await;
            app.offer_update(update);
        }
    });

    let app_clone = app.clone();
    tokio::spawn(async move {
        // Poll every 20 seconds for safety if WS misses an update
        let mut interval = interval(Duration::from_secs(20));
        interval.tick().await; // Initial fetch is already spawned above.
        loop {
            interval.tick().await;
            async_fetch_queue(app_clone.clone()).await;
        }
    });

    let mut terminal = ratatui::init();
    let paste_enabled =
        ratatui::crossterm::execute!(std::io::stdout(), event::EnableBracketedPaste).is_ok();
    let res = run_loop(&mut terminal, app.clone()).await;
    let signout = {
        let mut app = app.lock().await;
        app.cancel_login();
        app.signout_task.take()
    };
    if paste_enabled {
        let _ = ratatui::crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste);
    }
    ratatui::restore();
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

fn should_quit(app: &App, key: event::KeyEvent) -> bool {
    (key.code == KeyCode::Char('c') && key.modifiers.contains(event::KeyModifiers::CONTROL))
        || (matches!(key.code, KeyCode::Char('q') | KeyCode::Char('й'))
            && (app.fatal_error.is_some()
                || (app.input_mode == InputMode::Normal
                    && !app.is_settings_editing
                    && !matches!(app.view, View::Settings | View::Menu))))
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
    loop {
        {
            let mut app = app_arc.lock().await;
            app.update_realtime();
            terminal.draw(|f| ui(f, &mut app))?;
        }
        if !event::poll(Duration::from_millis(16))? {
            continue;
        }
        let event = event::read()?;
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
    if should_quit(app, key) {
        app.cancel_login();
        return UiOutcome::Quit;
    }
    if app.login_cancel.is_some() && key.code == KeyCode::Esc {
        app.cancel_login();
        return UiOutcome::Continue;
    }
    if app.fatal_error.is_some() {
        if matches!(key.code, KeyCode::Char('r' | 'к')) {
            app.fatal_error = None;
            app.error_message = None;
            tokio::spawn(async_fetch_queue(app_arc));
        }
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
                if app.error_message.is_some()
                    || app.queue_error.is_some()
                    || app.notice.is_some()
                    || app.update_notice =>
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
            KeyCode::Char('?') if !app.is_loading => app.open_overlay(View::Help),
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
        View::Menu | View::Help | View::ConfirmClear | View::UpdateFound => {}
    }
    UiOutcome::Continue
}

fn handle_paste(app: &mut App, text: &str) {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    if app.input_mode == InputMode::Editing {
        app.input.insert_str(app.input_cursor, &text);
        app.input_cursor += text.len();
        app.input_error = None;
    } else if app.view == View::Menu {
        app.palette_query.push_str(&text);
        app.menu_state.select(Some(0));
    } else if app.is_settings_editing {
        match app.settings_field {
            SettingsField::Host => {
                app.settings_cursor = cursor_boundary(&app.settings_input, app.settings_cursor);
                app.settings_input.insert_str(app.settings_cursor, &text);
                app.settings_cursor += text.len();
            }
            SettingsField::Offset => {
                for c in text.chars() {
                    edit_text(
                        &mut app.offset_input,
                        &mut app.settings_cursor,
                        event::KeyEvent::new(KeyCode::Char(c), event::KeyModifiers::NONE),
                        |c, text, cursor| {
                            c.is_ascii_digit() || (c == '-' && cursor == 0 && !text.contains('-'))
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
            if app.input.trim().is_empty() {
                app.input_error = Some("Enter a song name or URL before adding.".into());
                return;
            }
            let query = std::mem::take(&mut app.input);
            app.input_cursor = 0;
            app.input_error = None;
            app.input_mode = InputMode::Normal;
            tokio::spawn(async_play_track(app_arc, query));
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.input_cursor = 0;
            app.input_error = None;
        }
        _ => {
            let before = app.input.len();
            edit_text(&mut app.input, &mut app.input_cursor, key, |_, _, _| true);
            if app.input.len() != before {
                app.input_error = None;
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
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i)
        }
        KeyCode::Right => {
            if let Some(c) = text[*cursor..].chars().next() {
                *cursor += c.len_utf8();
            }
        }
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = text.len(),
        KeyCode::Backspace if *cursor > 0 => {
            let start = text[..*cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
            text.drain(start..*cursor);
            *cursor = start;
        }
        KeyCode::Delete if *cursor < text.len() => {
            let end = *cursor + text[*cursor..].chars().next().unwrap().len_utf8();
            text.drain(*cursor..end);
        }
        KeyCode::Char('u') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
            text.clear();
            *cursor = 0;
        }
        KeyCode::Char(c)
            if !c.is_control()
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
        KeyCode::Enter | KeyCode::Char('/') => {
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
        KeyCode::Char('l' | 'д') => match app.loop_mode.as_str() {
            "off" => Action::LoopTrack,
            "track" => Action::LoopQueue,
            _ => Action::LoopOff,
        },
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(event::KeyModifiers::CONTROL | event::KeyModifiers::ALT) =>
        {
            app.begin_search(&c.to_string());
            return;
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
            tokio::spawn(async_fetch_lyrics(app_arc));
        }
        _ => {}
    }
}

fn handle_settings_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    if app.is_settings_editing {
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
                app.error_message = None;
            }
            _ => match app.settings_field {
                SettingsField::Host => edit_text(
                    &mut app.settings_input,
                    &mut app.settings_cursor,
                    key,
                    |_, _, _| true,
                ),
                SettingsField::Offset => edit_text(
                    &mut app.offset_input,
                    &mut app.settings_cursor,
                    key,
                    |c, text, cursor| {
                        c.is_ascii_digit() || (c == '-' && cursor == 0 && !text.contains('-'))
                    },
                ),
                _ => {}
            },
        }
        return;
    }

    match key.code {
        KeyCode::Enter => match app.settings_field {
            SettingsField::Host | SettingsField::Offset => {
                app.is_settings_editing = true;
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
                    "Standard" => "Sidebar".to_string(),
                    "Sidebar" => "Studio".to_string(),
                    "Studio" => "Zen".to_string(),
                    "Zen" => "Standard".to_string(),
                    _ => "Standard".to_string(),
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
                    "Standard" => "Zen".to_string(),
                    "Sidebar" => "Standard".to_string(),
                    "Studio" => "Sidebar".to_string(),
                    "Zen" => "Studio".to_string(),
                    _ => "Standard".to_string(),
                };
                save_app_settings(app);
            }
            _ => {}
        },
        _ => {}
    }
}

fn draft_settings(app: &App) -> Result<api::Settings> {
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
                tokio::spawn(async_simple_command(
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
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);
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
        View::UpdateFound => views::update(f, app, &theme),
        _ => {}
    }
    if let Some(message) = &app.fatal_error {
        views::connection_error(f, app, &theme, message);
    }
}

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
    for (start, _) in input.char_indices() {
        let tail = &input[start..];
        if Line::from(tail).width() <= usize::from(width) {
            return tail;
        }
    }
    ""
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
    let column = Line::from(prefix).width() as u16;
    let mut end = cursor;
    for c in input[cursor..].chars() {
        let next = end + c.len_utf8();
        if Line::from(&input[start..next]).width() > usize::from(width) {
            break;
        }
        end = next;
    }
    (&input[start..end], column)
}
