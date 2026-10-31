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
        Bar, BarChart, BarGroup, Block, BorderType, Borders, Clear, Gauge, List, ListItem,
        ListState, Paragraph, Wrap,
    },
};
use reqwest::Client;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::{interval, timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, protocol::Message},
};
use url::Url;

mod actions;
mod chrome;
mod player;

use actions::{Action, execute as execute_action, handle_clear_keys, handle_palette_keys};
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
            text_secondary: Color::Rgb(120, 120, 140),
        },
        "Emerald" => Theme {
            bg: Color::Rgb(5, 15, 5),
            border: Color::Rgb(40, 60, 40),
            primary: Color::Rgb(50, 200, 50),
            highlight: Color::Rgb(100, 255, 100),
            text_secondary: Color::Rgb(120, 140, 120),
        },
        "Ruby" => Theme {
            bg: Color::Rgb(15, 5, 5),
            border: Color::Rgb(60, 40, 40),
            primary: Color::Rgb(200, 50, 50),
            highlight: Color::Rgb(255, 100, 100),
            text_secondary: Color::Rgb(140, 120, 120),
        },
        "Ocean" => Theme {
            bg: Color::Rgb(5, 10, 20),
            border: Color::Rgb(40, 60, 100),
            primary: Color::Rgb(50, 150, 255),
            highlight: Color::Rgb(100, 200, 255),
            text_secondary: Color::Rgb(120, 130, 160),
        },
        "Synthwave" => Theme {
            bg: Color::Rgb(20, 10, 30),
            border: Color::Rgb(100, 40, 100),
            primary: Color::Rgb(255, 50, 255),
            highlight: Color::Rgb(255, 150, 50), // Orange highlight
            text_secondary: Color::Rgb(160, 120, 180),
        },
        "Sepia" => Theme {
            bg: Color::Rgb(30, 25, 20),
            border: Color::Rgb(80, 70, 60),
            primary: Color::Rgb(180, 140, 100),
            highlight: Color::Rgb(220, 180, 140),
            text_secondary: Color::Rgb(140, 130, 120),
        },
        _ => Theme {
            // Default Jorik Purple
            bg: Color::Rgb(15, 15, 25),
            border: Color::Rgb(60, 60, 80),
            primary: JORIK_PURPLE,
            highlight: JORIK_HIGHLIGHT,
            text_secondary: Color::Rgb(150, 150, 170),
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
    lyrics_scroll: u16,

    auth_info_text: Option<String>,

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
            lyrics_scroll: 0,
            auth_info_text: None,
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
        if matches!(self.view, View::Menu | View::Help | View::ConfirmClear) {
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
                .or_else(|| event.error.as_ref().map(Value::to_string))
                .or_else(|| {
                    event
                        .data
                        .as_ref()
                        .and_then(|data| data.get("message"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "The server rejected the command.".to_string());
            self.error_message = Some(format!("Command failed: {detail}"));
        }
    }

    fn disconnect_ws(&mut self) {
        self.ws_connected = false;
        self.ws_connecting = false;
        self.has_playback_state = false;
        if !self.pending_commands.is_empty() {
            self.pending_commands.clear();
            self.error_message = Some(
                "Connection lost before command confirmation. Refresh before retrying.".to_string(),
            );
        }
    }

    fn is_busy(&self) -> bool {
        self.is_loading || !self.pending_commands.is_empty()
    }

    fn log(&mut self, msg: impl Into<String>) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        let mut message = msg.into();
        if let Some(token) = self.token.as_ref().filter(|token| !token.is_empty()) {
            message = message.replace(token, "[redacted]");
        }
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
            self.current_track = Some(format!("{} - {}", title, author));
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
                self.queue.push(format!("{} - {}", title, author));
            }
        }
        self.clamp_queue_selection();
        if self.current_track != previous_track {
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
            self.elapsed_ms += delta;
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
    let body = response.text().await.unwrap_or_default();
    Err(format!(
        "HTTP {status}: {}",
        body.chars().take(512).collect::<String>()
    ))
}

async fn request_json(req: reqwest::RequestBuilder) -> std::result::Result<Value, String> {
    checked_request(req)
        .await?
        .json::<Value>()
        .await
        .map_err(|error| format!("Invalid server response: {}", error.without_url()))
}

async fn async_fetch_queue(app_arc: Arc<Mutex<App>>) {
    let (client, base_url, token, payload) = {
        let mut app = app_arc.lock().await;
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
    let (client, base_url, token, payload) = {
        let mut app = app_arc.lock().await;
        app.is_loading = true;
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
    app.lyrics_text = Some(match result {
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
    });
}

async fn async_simple_command<T: serde::Serialize + Send + Sync + 'static>(
    app_arc: Arc<Mutex<App>>,
    endpoint: String,
    payload: T,
) {
    let (client, base_url, token) = {
        let mut app = app_arc.lock().await;
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

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn async_auth_login(app_arc: Arc<Mutex<App>>) {
    let (base_url, is_login_required_screen) = {
        let mut app = app_arc.lock().await;
        app.is_loading = true;
        app.auth_info_text = Some("Initializing login...".to_string());

        let is_login_required = app.view == View::LoginRequired;

        // If we are NOT on the LoginRequired screen (meaning we are in the Auth Menu),
        // switch to AuthResult to show the popup.
        // If we ARE on LoginRequired, we do NOTHING to the view, staying on that screen.
        if !is_login_required {
            app.view = View::AuthResult;
        }

        (app.base_url.clone(), is_login_required)
    };

    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(l) => l,
        Err(e) => {
            let mut app = app_arc.lock().await;
            app.is_loading = false;
            app.auth_info_text = Some(format!("Failed to bind listener: {}", e));
            return;
        }
    };

    let local_addr = match listener.local_addr() {
        Ok(a) => a,
        Err(e) => {
            let mut app = app_arc.lock().await;
            app.is_loading = false;
            app.auth_info_text = Some(format!("Failed to get local addr: {}", e));
            return;
        }
    };

    let callback_url = format!("http://{}/oauth-callback", local_addr);

    let mut auth_url = match reqwest::Url::parse(&api::build_url(&base_url, "/authorize")) {
        Ok(u) => u,
        Err(e) => {
            let mut app = app_arc.lock().await;
            app.is_loading = false;
            app.auth_info_text = Some(format!("Invalid base URL: {}", e));
            return;
        }
    };

    auth_url
        .query_pairs_mut()
        .append_pair("callback", &callback_url);

    {
        let mut app = app_arc.lock().await;
        app.auth_info_text = Some(format!(
            "Opening browser...\n\nIf it doesn't open, visit:\n{}",
            auth_url.as_str()
        ));
    }

    let _ = open::that(auth_url.as_str());

    // Wait for callback (120s timeout)
    match timeout(Duration::from_secs(120), listener.accept()).await {
        Ok(Ok((mut stream, _addr))) => {
            let mut buf = vec![0u8; 8192];
            let n = match stream.read(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    let mut app = app_arc.lock().await;
                    app.is_loading = false;
                    app.auth_info_text = Some(format!("Error reading callback: {}", e));
                    return;
                }
            };

            let req = String::from_utf8_lossy(&buf[..n]);
            let first_line = req.lines().next().unwrap_or("");
            let path = first_line.split_whitespace().nth(1).unwrap_or("");

            // Prepend a scheme+host so `Url::parse` can parse query params.
            if let Ok(parsed) = reqwest::Url::parse(&format!("http://localhost{}", path)) {
                let token_pair = parsed.query_pairs().find(|(k, _)| k == "token");
                let avatar_pair = parsed.query_pairs().find(|(k, _)| k == "avatar");
                let username_pair = parsed.query_pairs().find(|(k, _)| k == "username");

                if let Some((_, v)) = token_pair {
                    let token = v.into_owned();
                    let token_trim = token.trim().to_string();
                    if token_trim.is_empty() {
                        let body = "Missing token";
                        let resp = format!(
                            "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;

                        let mut app = app_arc.lock().await;
                        app.is_loading = false;
                        app.auth_info_text = Some("No token provided in callback.".to_string());
                        return;
                    }

                    let avatar_val = avatar_pair.map(|(_, val)| val.into_owned());
                    let username_val = username_pair.map(|(_, val)| val.into_owned());

                    if let Err(e) =
                        api::save_token(&token_trim, avatar_val.as_deref(), username_val.as_deref())
                    {
                        let mut app = app_arc.lock().await;
                        app.is_loading = false;
                        app.auth_info_text = Some(format!("Failed to save token: {}", e));
                        return;
                    }

                    // Build a small, readable success page and kick off confetti animation.
                    let escaped_username = username_val
                        .as_deref()
                        .map(escape_html)
                        .unwrap_or_else(|| "User".to_string());
                    let escaped_avatar = avatar_val.as_deref().map(escape_html);
                    let saved_path_html = if let Some(path) = api::config_file_path() {
                        format!(
                            "<p>Saved to <code>{}</code></p>",
                            escape_html(&path.display().to_string())
                        )
                    } else {
                        "".to_string()
                    };

                    let mut body = String::new();
                    body.push_str(
                        "<!doctype html><html><head><meta charset=\"utf-8\"/><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"/><title>Authorization complete</title><style>",
                    );
                    body.push_str("body{font-family:-apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,\"Helvetica Neue\",Arial, sans-serif;background:#2f3136;color:#dcddde;margin:0;padding:0;display:flex;align-items:center;justify-content:center;height:100vh}");
                    body.push_str(".container{max-width:560px;width:100%;padding:28px;background:#36393f;border-radius:12px;box-shadow:0 6px 20px rgba(0,0,0,0.6)}");
                    body.push_str(
                        ".header{display:flex;align-items:center;gap:16px;margin-bottom:18px}",
                    );
                    body.push_str(".badge{width:56px;height:56px;display:flex;align-items:center;justify-content:center;border-radius:50%;background:#2f3136}");
                    body.push_str(".check{width:34px;height:34px;border-radius:50%;background:#43b581;color:#fff;display:flex;align-items:center;justify-content:center;font-weight:700;font-size:16px}");
                    body.push_str(".avatar{width:56px;height:56px;border-radius:50%;object-fit:cover;border:2px solid rgba(0,0,0,0.4)}");
                    body.push_str(".user{font-size:16px;font-weight:600;margin:0;color:#fff}");
                    body.push_str(".sp{color:#b9bbbe;font-size:13px;margin-top:4px}");
                    body.push_str(".path{display:inline-block;background:#2f3136;padding:6px 8px;border-radius:6px;color:#b9bbbe;font-family:monospace;margin-top:8px}");
                    body.push_str(
                        "</style></head><body><div class=\"container\"><div class=\"header\">",
                    );
                    if let Some(avatar) = &escaped_avatar {
                        body.push_str(&format!(
                            r#"<img class="avatar" src="{}" alt="avatar"/>"#,
                            avatar
                        ));
                    } else {
                        body.push_str(r#"<div class="badge"><div class="check">✓</div></div>"#);
                    }
                    body.push_str(&format!(
                        r#"<div><div class="user">{}</div><div class="sp">Authorization complete</div>{}</div>"#,
                        escaped_username, saved_path_html
                    ));
                    body.push_str(r#"</div><div><p class="sp">Token saved to your config. You may close this window.</p></div>"#);

                    // confetti
                    body.push_str(r#"<script src="https://cdn.jsdelivr.net/npm/canvas-confetti@1.6.0/dist/confetti.browser.min.js"></script>"#);
                    body.push_str(
                        r#"<script>
  const duration = 15 * 1000,
    animationEnd = Date.now() + duration,
    defaults = { startVelocity: 30, spread: 360, ticks: 60, zIndex: 0 };

  function randomInRange(min, max) {
    return Math.random() * (max - min) + min;
  }

  const interval = setInterval(function() {
    const timeLeft = animationEnd - Date.now();

    if (timeLeft <= 0) {
      return clearInterval(interval);
    }

    const particleCount = 50 * (timeLeft / duration);

    confetti(
      Object.assign({}, defaults, {
        particleCount,
        origin: { x: randomInRange(0.1, 0.3), y: Math.random() - 0.2 },
      })
    );
    confetti(
      Object.assign({}, defaults, {
        particleCount,
        origin: { x: randomInRange(0.7, 0.9), y: Math.random() - 0.2 },
      })
    );
  }, 250);
</script>"#,
                    );
                    body.push_str("</div></body></html>");

                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let _ = stream.shutdown().await;

                    {
                        let mut app = app_arc.lock().await;
                        app.is_loading = false;
                        app.token = Some(token_trim.clone());
                        app.auth_info_text = Some(format!(
                            "Login Successful!\n\nUser: {}\nToken saved.",
                            username_val.unwrap_or_default()
                        ));
                    }

                    // Small delay to ensure stability
                    tokio::time::sleep(Duration::from_millis(500)).await;

                    // Refresh data before switching view
                    async_fetch_queue(app_arc.clone()).await;

                    let mut app = app_arc.lock().await;
                    // Only transition to Main if we were on the LoginRequired screen.
                    if is_login_required_screen {
                        app.view = View::Main;
                    }
                } else {
                    let body = "No token in callback";
                    let resp = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;

                    let mut app = app_arc.lock().await;
                    app.is_loading = false;
                    app.auth_info_text =
                        Some("Login failed: Missing token in callback.".to_string());
                }
            }
        }
        _ => {
            let mut app = app_arc.lock().await;
            app.is_loading = false;
            app.auth_info_text = Some("Login timed out.".to_string());
        }
    }
}

async fn async_auth_signout(app_arc: Arc<Mutex<App>>) {
    let (client, base_url, token) = {
        let mut app = app_arc.lock().await;
        app.is_loading = true;
        app.view = View::AuthResult;
        app.auth_info_text = Some("Signing out...".to_string());
        (app.client.clone(), app.base_url.clone(), app.token.clone())
    };

    if let Some(tok) = token {
        let url = api::build_url(&base_url, "/webhook/auth/revoke");
        let _ = client.post(&url).bearer_auth(tok).send().await;
    }

    // Remove local file
    if let Some(path) = api::config_file_path() {
        let _ = std::fs::remove_file(path);
    }

    let mut app = app_arc.lock().await;
    app.is_loading = false;
    app.token = None;
    app.auth_info_text = None;
    app.view = View::LoginRequired;
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

        let ws_url = match Url::parse(&base_url) {
            Ok(u) => {
                let scheme = if u.scheme() == "https" { "wss" } else { "ws" };
                let mut u = u;
                u.set_scheme(scheme).ok();
                u.set_path("/ws");
                u.query_pairs_mut().append_pair("token", &token);
                u
            }
            Err(e) => {
                let mut app = app_arc.lock().await;
                app.log(format!("WS URL Parse Error: {}", e));
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

        let request = match ws_url.as_str().into_client_request() {
            Ok(mut req) => {
                let headers = req.headers_mut();
                headers.insert("User-Agent", HeaderValue::from_static("jorik-cli"));
                headers.insert(
                    "Origin",
                    HeaderValue::from_str(&base_url)
                        .unwrap_or_else(|_| HeaderValue::from_static("jorik-cli")),
                );
                headers.insert(
                    "Authorization",
                    HeaderValue::from_str(&format!("Bearer {}", token))
                        .unwrap_or_else(|_| HeaderValue::from_static("")),
                );
                req
            }
            Err(e) => {
                let mut app = app_arc.lock().await;
                app.log(format!("WS Request Error: {}", e));
                drop(app);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        match connect_async(request).await {
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
        .timeout(Duration::from_secs(10))
        .build()?;

    api::validate_base_url(&settings.base_url)?;
    let (ws_tx, ws_rx) = tokio::sync::mpsc::channel::<Message>(32);

    let mut app_struct = App::new(client.clone(), settings, token, guild_id, user_id);
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
    let res = run_loop(&mut terminal, app).await;
    if paste_enabled {
        let _ = ratatui::crossterm::execute!(std::io::stdout(), event::DisableBracketedPaste);
    }
    ratatui::restore();
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
        return UiOutcome::Quit;
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
                    app.help_scroll = app.help_scroll.saturating_add(1).min(30)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.help_scroll = app.help_scroll.saturating_sub(1)
                }
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
        View::Lyrics => handle_lyrics_keys(app, key),
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
                app.view = View::Main;
            }
        }
        View::LoginRequired => match key.code {
            KeyCode::Enter => {
                tokio::spawn(async_auth_login(app_arc));
            }
            KeyCode::Char('\\') => {
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
    } else if app.view == View::Menu {
        app.palette_query.push_str(&text);
        app.menu_state.select(Some(0));
    } else if app.is_settings_editing {
        match app.settings_field {
            SettingsField::Host => app.settings_input.push_str(&text),
            SettingsField::Offset => {
                for c in text.chars() {
                    if c.is_ascii_digit() || (c == '-' && app.offset_input.is_empty()) {
                        app.offset_input.push(c);
                    }
                }
            }
            _ => {}
        }
    }
}

fn handle_editing_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    match key.code {
        KeyCode::Enter => {
            let query = std::mem::take(&mut app.input);
            app.input_cursor = 0;
            app.input_mode = InputMode::Normal;
            tokio::spawn(async_play_track(app_arc, query));
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.input_cursor = 0;
        }
        KeyCode::Left => {
            app.input_cursor = app.input[..app.input_cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
        }
        KeyCode::Right => {
            if let Some(c) = app.input[app.input_cursor..].chars().next() {
                app.input_cursor += c.len_utf8();
            }
        }
        KeyCode::Home => app.input_cursor = 0,
        KeyCode::End => app.input_cursor = app.input.len(),
        KeyCode::Backspace if app.input_cursor > 0 => {
            let start = app.input[..app.input_cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i);
            app.input.drain(start..app.input_cursor);
            app.input_cursor = start;
        }
        KeyCode::Delete if app.input_cursor < app.input.len() => {
            let end = app.input_cursor
                + app.input[app.input_cursor..]
                    .chars()
                    .next()
                    .unwrap()
                    .len_utf8();
            app.input.drain(app.input_cursor..end);
        }
        KeyCode::Char('u') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
            app.input.clear();
            app.input_cursor = 0;
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(event::KeyModifiers::CONTROL | event::KeyModifiers::ALT) =>
        {
            app.input.insert(app.input_cursor, c);
            app.input_cursor += c.len_utf8();
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

fn handle_lyrics_keys(app: &mut App, key: event::KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.view = View::Main,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
            app.lyrics_scroll = app.lyrics_scroll.saturating_add(1);
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
            app.lyrics_scroll = app.lyrics_scroll.saturating_sub(1);
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
                let host_changed = app.base_url != app.settings_input;
                app.base_url = app.settings_input.clone();
                app.visualizer_offset =
                    app.offset_input.parse().expect("validated settings offset");
                if host_changed {
                    app.needs_reconnect = true;
                    tokio::spawn(async_fetch_queue(app_arc));
                }
            }
            KeyCode::Esc => {
                app.settings_input = app.base_url.clone();
                app.offset_input = app.visualizer_offset.to_string();
                app.is_settings_editing = false;
                app.error_message = None;
            }
            KeyCode::Char(c) => match app.settings_field {
                SettingsField::Host => {
                    app.settings_input.push(c);
                }
                SettingsField::Offset
                    if c.is_ascii_digit() || (c == '-' && app.offset_input.is_empty()) =>
                {
                    app.offset_input.push(c);
                }
                _ => {}
            },
            KeyCode::Backspace => match app.settings_field {
                SettingsField::Host => {
                    app.settings_input.pop();
                }
                SettingsField::Offset => {
                    app.offset_input.pop();
                }
                _ => {}
            },
            _ => {}
        }
        return;
    }

    match key.code {
        KeyCode::Enter => match app.settings_field {
            SettingsField::Host | SettingsField::Offset => {
                app.is_settings_editing = true;
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
        KeyCode::Up => {
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
    api::validate_base_url(&app.settings_input)?;
    let visualizer_offset = app
        .offset_input
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("visualizer offset must be a valid integer"))?;
    Ok(api::Settings {
        base_url: app.settings_input.clone(),
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
        KeyCode::Esc => app.view = View::Main,
        KeyCode::Backspace => {
            app.view = View::Main;
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
                app.view = View::Main;
            }
        }
        _ => {}
    }
}

fn handle_auth_menu_keys(app: &mut App, key: event::KeyEvent, app_arc: Arc<Mutex<App>>) {
    match key.code {
        KeyCode::Esc | KeyCode::Tab => app.view = View::Main,
        KeyCode::Backspace => {
            app.view = View::Main;
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
                    "Login" => {
                        tokio::spawn(async_auth_login(app_arc));
                    }
                    "Signout" => {
                        tokio::spawn(async_auth_signout(app_arc));
                    }
                    "Info" => {
                        if let Some(auth) = api::load_auth() {
                            let mut info = String::new();
                            if let Some(path) = api::config_file_path() {
                                info.push_str(&format!("Auth file: {}\n", path.display()));
                            }
                            info.push_str(&format!(
                                "User: {}\n",
                                auth.username.unwrap_or_else(|| "Unknown".to_string())
                            ));
                            if let Some(avatar) = auth.avatar_url {
                                info.push_str(&format!("Avatar: {}\n", avatar));
                            }
                            let token_masked = "[redacted]";
                            info.push_str(&format!("Token: {}", token_masked));
                            app.auth_info_text = Some(info);
                            app.view = View::AuthResult;
                        } else {
                            app.auth_info_text = Some("Not authenticated. Run Login.".to_string());
                            app.view = View::AuthResult;
                        }
                    }
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

    // Base background color for the entire UI
    f.render_widget(Block::default().bg(theme.bg), f.area());

    if app.view == View::UpdateFound {
        let area = popup_rect(66, 13, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(" 🚀 Update Available ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(Color::Green));

        let version = app
            .update_info
            .as_ref()
            .map(|(v, _)| v.as_str())
            .unwrap_or("Unknown");

        let text = vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("A new version "),
                Span::styled(
                    version,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" is available!"),
            ]),
            Line::from(""),
            Line::from("Do you want to update now?"),
            Line::from(""),
            Line::from(vec![
                Span::raw("Press "),
                Span::styled(
                    " y ",
                    Style::default()
                        .bg(Color::Green)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" to Update and Exit"),
            ]),
            Line::from(vec![
                Span::raw("Press "),
                Span::styled(
                    " n ",
                    Style::default()
                        .bg(Color::Red)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" to Skip for now"),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Nothing is installed unless you confirm.",
                Style::default().fg(theme.text_secondary),
            )),
        ];

        let p = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(block)
            .wrap(Wrap { trim: true });

        f.render_widget(p, area);
        return;
    }

    if app.view == View::LoginRequired {
        let area = popup_rect(72, 14, f.area());
        let status = app
            .auth_info_text
            .as_deref()
            .unwrap_or("Log in with Discord to manage playback in your voice channel.");
        let text = vec![
            Line::from(Span::styled(
                "JORIK",
                Style::default().fg(theme.highlight).bold(),
            )),
            Line::from(format!("Server: {}", app.base_url)),
            Line::from(""),
            Line::from(status),
            Line::from(""),
            Line::from(if app.is_loading {
                "Waiting for browser authorization…"
            } else {
                "Enter log in · \\ change host · q quit"
            }),
        ];
        f.render_widget(
            Paragraph::new(text)
                .block(
                    Block::bordered()
                        .title(" Welcome ")
                        .border_style(Style::default().fg(theme.border)),
                )
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }

    let main_layout = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(f.area());
    let top_section = main_layout[1];
    let status_bar_area = main_layout[2];
    let content_view = app.content_view();
    render_header(f, app, &theme, content_view, main_layout[0]);

    match content_view {
        View::Lyrics => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(format!(
                    " Lyrics {} ",
                    if app.is_loading { " ⏳ " } else { "" }
                ))
                .title_alignment(Alignment::Center)
                .border_style(Style::default().fg(theme.primary));

            let text = app.lyrics_text.as_deref().unwrap_or("Loading...");
            let p = Paragraph::new(text)
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((app.lyrics_scroll, 0));

            f.render_widget(p, top_section);
        }
        View::Settings => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(" Settings ")
                .title_alignment(Alignment::Center)
                .border_style(Style::default().fg(theme.primary));

            let f_field = app.settings_field;
            let is_ed = app.is_settings_editing;

            let h_s = |f| {
                if f_field == f {
                    if is_ed {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    }
                } else {
                    Style::default().fg(theme.text_secondary)
                }
            };

            let h_l = |f, l| {
                if f_field == f {
                    if is_ed {
                        format!(" >> [EDITING] {}", l)
                    } else {
                        format!(" >> {}", l)
                    }
                } else {
                    format!("    {}", l)
                }
            };

            let inner = block.inner(top_section);
            let host_label = h_l(SettingsField::Host, "Webhook Host: ");
            let offset_label = h_l(SettingsField::Offset, "Visualizer Offset (ms): ");
            let host_text = input_tail(
                &app.settings_input,
                inner
                    .width
                    .saturating_sub(Line::from(host_label.as_str()).width() as u16 + 1),
            );
            let offset_text = input_tail(
                &app.offset_input,
                inner
                    .width
                    .saturating_sub(Line::from(offset_label.as_str()).width() as u16 + 1),
            );

            let p = Paragraph::new(vec![
                Line::from("Configure your experience:"),
                Line::from(""),
                Line::from(vec![
                    Span::styled(host_label.as_str(), h_s(SettingsField::Host)),
                    Span::styled(host_text, h_s(SettingsField::Host)),
                ]),
                Line::from(vec![
                    Span::styled(offset_label.as_str(), h_s(SettingsField::Offset)),
                    Span::styled(offset_text, h_s(SettingsField::Offset)),
                ]),
                Line::from(vec![
                    Span::styled(
                        h_l(SettingsField::Theme, "Color Theme: "),
                        h_s(SettingsField::Theme),
                    ),
                    Span::styled(format!("< {} >", app.theme), h_s(SettingsField::Theme)),
                ]),
                Line::from(vec![
                    Span::styled(
                        h_l(SettingsField::VizStyle, "Visualizer Style: "),
                        h_s(SettingsField::VizStyle),
                    ),
                    Span::styled(
                        format!("< {} >", app.viz_style),
                        h_s(SettingsField::VizStyle),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(
                        h_l(SettingsField::Layout, "UI Layout: "),
                        h_s(SettingsField::Layout),
                    ),
                    Span::styled(format!("< {} >", app.layout), h_s(SettingsField::Layout)),
                ]),
                Line::from(""),
                Line::from(if is_ed {
                    Span::styled(
                        "TYPE TO EDIT, ENTER TO SAVE, ESC TO CANCEL",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(
                        "NAVIGATE WITH ARROWS/TAB, ENTER ON TEXT TO EDIT, ESC TO EXIT",
                        Style::default().fg(theme.text_secondary),
                    )
                }),
            ])
            .block(block)
            .alignment(Alignment::Left);

            f.render_widget(p, top_section);

            // Derive positions from the rendered labels and terminal-cell widths.
            if is_ed && inner.width > 0 && inner.height > 0 {
                let (row, label, text) = match f_field {
                    SettingsField::Host => (2, host_label.as_str(), host_text),
                    SettingsField::Offset => (3, offset_label.as_str(), offset_text),
                    _ => (inner.height, "", ""),
                };
                let column = Line::from(label).width() + Line::from(text).width();
                if row < inner.height && column < usize::from(inner.width) {
                    f.set_cursor_position((inner.x + column as u16, inner.y + row));
                }
            }
        }
        View::Debug => {
            let ws_status = if app.ws_connected {
                Span::styled(
                    " CONNECTED ",
                    Style::default()
                        .bg(Color::Green)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
            } else if app.ws_connecting {
                Span::styled(
                    " CONNECTING... ",
                    Style::default()
                        .bg(Color::Yellow)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    " DISCONNECTED ",
                    Style::default()
                        .bg(Color::Red)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(vec![
                    Span::raw(" Debug Console "),
                    ws_status,
                    Span::raw(" (Press 's' to Save Spectrogram) "),
                ])
                .title_alignment(Alignment::Left)
                .border_style(Style::default().fg(Color::Yellow));

            let log_lines: Vec<Line> = app
                .debug_logs
                .iter()
                .rev()
                .map(|l| Line::from(l.as_str()))
                .collect();

            let p = Paragraph::new(log_lines)
                .block(block)
                .wrap(Wrap { trim: false });

            f.render_widget(p, top_section);
        }
        _ => {
            render_player_ui(f, app, &theme, top_section);
        }
    }

    render_footer(f, app, &theme, status_bar_area);

    if app.input_mode == InputMode::Editing {
        let area = popup_rect(70, 5, f.area());
        f.render_widget(Clear, area);

        let loading_text = if app.is_loading { " ⏳ " } else { "" };
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(format!(" Add a track or URL {} ", loading_text))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(theme.highlight));

        let inner = input_block.inner(area);
        let (text, cursor_column) = input_window(&app.input, app.input_cursor, inner.width);
        let p = Paragraph::new(text)
            .block(input_block)
            .style(Style::default().fg(Color::White));
        f.render_widget(p, area);

        if inner.width > 0 && inner.height > 0 {
            f.set_cursor_position((inner.x + cursor_column, inner.y));
        }
    }

    match app.view {
        View::Menu => render_palette(f, app, &theme),
        View::Help => render_help(f, app, &theme),
        View::ConfirmClear => render_clear_confirmation(f, &theme),
        _ => {}
    }

    if app.view == View::FilterMenu {
        let area = popup_rect(52, 13, f.area());

        // Shadow
        let shadow_area = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width,
            height: area.height,
        };
        if shadow_area.right() < f.area().right() && shadow_area.bottom() < f.area().bottom() {
            f.render_widget(Block::default().bg(Color::Rgb(10, 10, 20)), shadow_area);
        }

        f.render_widget(Clear, area);

        let loading_text = if app.is_loading { " ⏳ " } else { "" };
        let menu_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(format!(" Select Filter {} ", loading_text))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(theme.primary));

        let items: Vec<ListItem> = app
            .filter_items
            .iter()
            .map(|i| ListItem::new(format!("  {}  ", *i)))
            .collect();

        let list = List::new(items)
            .block(menu_block)
            .highlight_style(
                Style::default()
                    .bg(theme.primary)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(" >> ");

        f.render_stateful_widget(list, area, &mut app.filter_state);
    }

    if app.view == View::AuthMenu {
        let area = popup_rect(52, 7, f.area());

        // Shadow
        let shadow_area = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width,
            height: area.height,
        };
        if shadow_area.right() < f.area().right() && shadow_area.bottom() < f.area().bottom() {
            f.render_widget(Block::default().bg(Color::Rgb(10, 10, 20)), shadow_area);
        }

        f.render_widget(Clear, area);

        let loading_text = if app.is_loading { " ⏳ " } else { "" };
        let menu_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(format!(" Auth {} ", loading_text))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(theme.primary));

        let items: Vec<ListItem> = app
            .auth_menu_items
            .iter()
            .map(|i| ListItem::new(format!("  {}  ", *i)))
            .collect();

        let list = List::new(items)
            .block(menu_block)
            .highlight_style(
                Style::default()
                    .bg(theme.primary)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(" >> ");

        f.render_stateful_widget(list, area, &mut app.auth_menu_state);
    }

    if app.view == View::AuthResult {
        let area = popup_rect(72, 12, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(" Auth Info ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(theme.primary));

        let text = app.auth_info_text.as_deref().unwrap_or("No data.");
        let p = Paragraph::new(text).block(block).wrap(Wrap { trim: true });

        f.render_widget(p, area);
    }

    if app.view == View::AppInfo {
        let area = popup_rect(72, 18, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(" Build Compatibility Info ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(theme.highlight));

        let text = vec![
            Line::from(Span::styled(
                "BUILD COMPATIBILITY",
                Style::default()
                    .add_modifier(Modifier::BOLD)
                    .fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from("This version of Jorik CLI is intended for use with"),
            Line::from(vec![
                Span::raw("the "),
                Span::styled(
                    "INTERNAL DEV VERSION",
                    Style::default()
                        .fg(theme.highlight)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" of Jorik bot."),
            ]),
            Line::from(""),
            Line::from("The production version will work, but with significantly"),
            Line::from("reduced functionality (limited real-time features)."),
            Line::from(""),
            Line::from(vec![
                Span::raw("Current Version: "),
                Span::styled(env!("CARGO_PKG_VERSION"), Style::default().fg(Color::White)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Press 'i' or Esc to close",
                Style::default().fg(theme.text_secondary),
            )),
        ];

        let p = Paragraph::new(text)
            .alignment(Alignment::Center)
            .block(block)
            .wrap(Wrap { trim: true });

        f.render_widget(p, area);
    }

    if let Some(msg) = &app.fatal_error {
        let area = popup_rect(72, 8, f.area());
        f.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .title(" ⚠ Connection Error ")
            .title_alignment(Alignment::Center)
            .style(Style::default())
            .border_style(Style::default().fg(Color::Red));

        let p = Paragraph::new(msg.as_str())
            .block(block)
            .style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(p, area);
    }
}

#[cfg(test)]
mod refinement_tests;
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
