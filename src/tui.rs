use crate::api::{self, AudioFilters, EqualizerBand, FilterPayload, KaraokeOptions, LoopPayload, LowPassOptions, LyricsPayload, PlayPayload, QueuePayload, RotationOptions, SimplePayload, TimescaleOptions, TremoloOptions, TwentyFourSevenPayload, VibratoOptions, WsEvent, WsSubscribe, PlaybackState};
use crate::ascii::ASCII_LOGO;
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap, BarChart, Bar, BarGroup},
    DefaultTerminal, Frame,
};
use reqwest::Client;
use serde_json::Value;
use std::{sync::Arc, time::{Duration, Instant}};
use tokio::sync::Mutex;
use tokio::time::{interval, timeout};
use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use futures_util::{StreamExt, SinkExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;



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
    Lyrics,
    FilterMenu,
    AuthMenu,
    AuthResult,
    LoginRequired,
    Settings,
    Debug,
}

#[derive(PartialEq, Clone, Copy)]
enum SettingsField {
    Host,
    Offset,
}

struct App {
    client: Client,
    base_url: String,
    token: Option<String>,
    guild_id: Option<String>,
    user_id: Option<String>,
    
    queue: Vec<String>,
    current_track: Option<String>,
    error_message: Option<String>,
    fatal_error: Option<String>,
    loop_mode: String, // "off", "track", "queue"
    is_loading: bool,
    
    input: String,
    input_mode: InputMode,
    view: View,
    
    menu_state: ListState,
    menu_items: Vec<&'static str>,
    
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
    last_state_update: Instant,

    settings_input: String,
    offset_input: String,
    settings_field: SettingsField,
    needs_reconnect: bool,
    visualizer_offset: i64,

    debug_logs: Vec<String>,
    ws_connected: bool,
    ws_connecting: bool,

    smoothed_bars: Vec<f32>,
}

impl App {
    fn new(
        client: Client,
        base_url: String,
        visualizer_offset: i64,
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
        
        let view = if token.is_some() { View::Main } else { View::LoginRequired };

        Self {
            client,
            base_url: base_url.clone(),
            token,
            guild_id,
            user_id,
            queue: Vec::new(),
            current_track: None,
            error_message: None,
            fatal_error: None,
            loop_mode: "off".to_string(),
            is_loading: false,
            input: String::new(),
            input_mode: InputMode::Normal,
            view,
            menu_state,
            menu_items: vec![
                "Skip", "Pause/Resume", "Stop", "Shuffle", 
                "Clear Queue", "Loop Track", "Loop Queue", "Loop Off",
                "24/7 Mode Toggle", "Filters...", "Lyrics", "Play Turip",
                "Auth", "Settings", "Exit TUI"
            ],
            filter_state,
            filter_items: vec![
                "Clear", "Bassboost", "Nightcore", "Vaporwave", 
                "8D", "Soft", "Tremolo", "Vibrato", "Karaoke"
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
            last_state_update: Instant::now(),
            settings_input: base_url,
            offset_input: visualizer_offset.to_string(),
            settings_field: SettingsField::Host,
            needs_reconnect: false,
            visualizer_offset,
            debug_logs: Vec::new(),
            ws_connected: false,
            ws_connecting: false,
            smoothed_bars: vec![0.0; 64],
        }
    }

    fn log(&mut self, msg: impl Into<String>) {
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        self.debug_logs.push(format!("[{}] {}", timestamp, msg.into()));
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
                if let Ok(_) = std::fs::write(&path, json) {
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

    fn parse_queue_response(&mut self, json: &Value) {
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

        if let Some(current) = json.get("current").and_then(|v| v.as_object()) {
            let title = current.get("title").and_then(|v| v.as_str()).unwrap_or("Unknown");
            let author = current.get("author").and_then(|v| v.as_str()).unwrap_or("");
            self.current_track = Some(format!("{} - {}", title, author));
        } else {
            self.current_track = None;
        }

        self.queue.clear();
        if let Some(upcoming) = json.get("upcoming").and_then(|v| v.as_array()) {
            for item in upcoming {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("Unknown");
                let author = item.get("author").and_then(|v| v.as_str()).unwrap_or("");
                self.queue.push(format!("{} - {}", title, author));
            }
        }
    }

    fn update_realtime(&mut self) {
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
                let adjusted_ms = self.elapsed_ms.saturating_add_signed(self.visualizer_offset);
                let frame_index = (adjusted_ms as f64 / 42.66).floor() as usize;
                if frame_index < spec.len() {
                    let target_bars = &spec[frame_index];
                    for i in 0..64.min(target_bars.len()) {
                        let target = target_bars[i] as f32;
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
async fn async_fetch_queue(app_arc: Arc<Mutex<App>>) {
    let (client, url, token, payload) = {
        let mut app = app_arc.lock().await;
        app.is_loading = true;
        let payload = QueuePayload {
            action: "queue",
            guild_id: app.guild_id.clone(),
            user_id: app.user_id.clone(),
            limit: 20,
            offset: 0,
        };
        let url = api::build_url(&app.base_url, "/webhook/audio");
        (app.client.clone(), url, app.token.clone(), payload)
    };

    let mut req = client.post(&url).json(&payload);
    if let Some(bearer) = &token {
        req = req.bearer_auth(bearer);
    }

    let result = req.send().await;
    
    let mut app = app_arc.lock().await;
    app.is_loading = false;
    match result {
        Ok(resp) => {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<Value>().await {
                    app.parse_queue_response(&json);
                    app.error_message = None;
                }
            } else {
                 let text = resp.text().await.unwrap_or_default();
                 
                 let mut handled = false;
                 if let Ok(json_err) = serde_json::from_str::<Value>(&text) {
                     if json_err.get("error").and_then(|v| v.as_str()) == Some("bad_request") &&
                        json_err.get("message").and_then(|v| v.as_str()) == Some("user_not_in_voice_channel_or_guild_unknown") {
                            app.fatal_error = Some("User not in voice channel or guild unknown.\n\nPress 'r' to reload.".to_string());
                            handled = true;
                     }
                 }

                 if !handled {
                     if text.contains("guild_id is required") {
                         app.error_message = Some("Not connected to a voice channel or Guild ID missing.".to_string());
                     } else {
                         app.error_message = Some(format!("Error: {}", text));
                     }
                 }
            }
        }
        Err(e) => {
            app.error_message = Some(format!("Network error: {}", e));
        }
    }
}

async fn async_play_track(app_arc: Arc<Mutex<App>>, query: String) {
    let (client, url, token, payload) = {
        let mut app = app_arc.lock().await;
        app.is_loading = true;
        let payload = PlayPayload {
            action: "play",
            guild_id: app.guild_id.clone(),
            channel_id: None,
            query: api::clean_query(&query),
            user_id: app.user_id.clone(),
            requested_by: None,
            avatar_url: None,
        };
        let url = api::build_url(&app.base_url, "/webhook/audio");
        (app.client.clone(), url, app.token.clone(), payload)
    };

    let mut req = client.post(&url).json(&payload);
    if let Some(bearer) = &token {
        req = req.bearer_auth(bearer);
    }

    let _ = req.send().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    async_fetch_queue(app_arc).await;
}

async fn async_fetch_lyrics(app_arc: Arc<Mutex<App>>) {
    let (client, url, token, payload) = {
        let mut app = app_arc.lock().await;
        app.is_loading = true;
        let payload = LyricsPayload {
            action: "lyrics".to_string(),
            guild_id: app.guild_id.clone(),
            user_id: app.user_id.clone(),
        };
        let url = api::build_url(&app.base_url, "/webhook/audio");
        (app.client.clone(), url, app.token.clone(), payload)
    };

    let mut req = client.post(&url).json(&payload);
    if let Some(bearer) = &token {
        req = req.bearer_auth(bearer);
    }

    let result = req.send().await;
    
    let mut app = app_arc.lock().await;
    app.view = View::Lyrics;
    app.lyrics_scroll = 0;
    app.is_loading = false;
    
    match result {
        Ok(resp) => {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(data) = json.get("data").and_then(|v| v.as_object()) {
                    let mut output = String::new();
                    if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
                        output.push_str(text);
                    } else if let Some(lines) = data.get("lines").and_then(|v| v.as_array()) {
                        for line in lines {
                            let text = line.get("line").and_then(|v| v.as_str()).unwrap_or("");
                            output.push_str(&format!("{}\n", text));
                        }
                    }
                    if output.trim().is_empty() {
                         app.lyrics_text = Some("No lyrics found.".to_string());
                    } else {
                         app.lyrics_text = Some(output);
                    }
                } else {
                    app.lyrics_text = Some("No lyrics found.".to_string());
                }
            } else {
                app.lyrics_text = Some("Failed to parse lyrics.".to_string());
            }
        }
        Err(e) => {
            app.lyrics_text = Some(format!("Failed to fetch lyrics: {}", e));
        }
    }
}

async fn async_simple_command<T: serde::Serialize + Send + Sync + 'static>(app_arc: Arc<Mutex<App>>, endpoint: String, payload: T) {
    let (client, url, token) = {
        let mut app = app_arc.lock().await;
        app.is_loading = true;
        let url = api::build_url(&app.base_url, &endpoint);
        (app.client.clone(), url, app.token.clone())
    };

    let mut req = client.post(&url).json(&payload);
    if let Some(bearer) = &token {
        req = req.bearer_auth(bearer);
    }

    let _ = req.send().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
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
    
    auth_url.query_pairs_mut().append_pair("callback", &callback_url);

    {
        let mut app = app_arc.lock().await;
        app.auth_info_text = Some(format!("Opening browser...\n\nIf it doesn't open, visit:\n{}", auth_url.as_str()));
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

                    if let Err(e) = api::save_token(&token_trim, avatar_val.as_deref(), username_val.as_deref()) {
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
                        app.auth_info_text = Some(format!("Login Successful!\n\nUser: {}\nToken saved.", username_val.unwrap_or_default()));
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
                } else {                    let body = "No token in callback";
                    let resp = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    
                    let mut app = app_arc.lock().await;
                    app.is_loading = false;
                    app.auth_info_text = Some("Login failed: Missing token in callback.".to_string());
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
        if path.exists() {
             let _ = std::fs::remove_file(path);
        }
    }

    let mut app = app_arc.lock().await;
    app.is_loading = false;
    app.token = None;
    app.auth_info_text = None;
    app.view = View::LoginRequired;
}

async fn spawn_websocket(app_arc: Arc<Mutex<App>>) {
    let mut last_waiting_log = Instant::now();
    
    loop {
        let (base_url, token, guild_id) = {
            let app = app_arc.lock().await;
            (app.base_url.clone(), app.token.clone(), app.guild_id.clone())
        };

        if token.is_none() || guild_id.is_none() {
            if last_waiting_log.elapsed() > Duration::from_secs(10) {
                let mut app = app_arc.lock().await;
                if token.is_none() {
                    app.log("WS waiting for token...");
                } else if guild_id.is_none() {
                    app.log("WS waiting for Guild ID (join a voice channel or specify --guild-id)...");
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
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        {
            let mut app = app_arc.lock().await;
            app.log(format!("WS Connecting to {}", ws_url));
            app.ws_connected = false;
            app.ws_connecting = true;
        }

        match connect_async(ws_url.as_str()).await {
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
                                                if event.guild_id.as_deref() == app.guild_id.as_deref() {
                                                    if let Some(data) = event.data {
                                                        if let Ok(spectrogram) = serde_json::from_value::<Vec<Vec<u8>>>(data) {
                                                            app.log(format!("Received Spectrogram ({} frames)", spectrogram.len()));
                                                            app.spectrogram = Some(spectrogram);
                                                        }
                                                    }
                                                }
                                            }
                                            "state_update" | "initial_state" => {
                                                if event.guild_id.as_deref() == app.guild_id.as_deref() {
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
                                                        app.elapsed_ms = playback.elapsed_ms;
                                                        app.duration_ms = playback.duration_ms;
                                                        app.paused = playback.paused;
                                                        app.last_state_update = Instant::now();
                                                        if let Some(spec) = playback.spectrogram {
                                                            app.log(format!("Received Spectrogram in state ({} frames)", spec.len()));
                                                            app.spectrogram = Some(spec);
                                                        }
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
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {
                            let mut app = app_arc.lock().await;
                            if app.needs_reconnect {
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
            app.ws_connected = false;
            app.ws_connecting = false;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

pub async fn run(
    settings: api::Settings,
    token: Option<String>,
    guild_id: Option<String>,
    user_id: Option<String>,
) -> Result<()> {
    let client = Client::builder()
        .user_agent("jorik-cli-tui")
        .timeout(Duration::from_secs(10))
        .build()?;

    let app = Arc::new(Mutex::new(App::new(client, settings.base_url, settings.visualizer_offset, token, guild_id, user_id)));
    
    // Initial fetch
    tokio::spawn(async_fetch_queue(app.clone()));
    tokio::spawn(spawn_websocket(app.clone()));

    let app_clone = app.clone();
    tokio::spawn(async move {
        // Poll every 20 seconds for safety if WS misses an update
        let mut interval = interval(Duration::from_secs(20));
        loop {
            interval.tick().await;
            async_fetch_queue(app_clone.clone()).await;
        }
    });

    let mut terminal = ratatui::init();
    let res = run_loop(&mut terminal, app).await;
    ratatui::restore();
    res
}

async fn run_loop(terminal: &mut DefaultTerminal, app_arc: Arc<Mutex<App>>) -> Result<()> {
    loop {
        {
            let mut app = app_arc.lock().await;
            app.update_realtime();
            terminal.draw(|f| ui(f, &mut *app))?;
        }

        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    let mut app = app_arc.lock().await;

                    if app.fatal_error.is_some() {
                        if let KeyCode::Char('r') | KeyCode::Char('к') = key.code {
                            app.fatal_error = None;
                            app.error_message = None;
                            drop(app);
                            tokio::spawn(async_fetch_queue(app_arc.clone()));
                        }
                        continue;
                    }
                    
                    if app.input_mode == InputMode::Editing {
                        match key.code {
                            KeyCode::Enter => {
                                let query = app.input.clone();
                                app.input.clear();
                                app.input_mode = InputMode::Normal;
                                tokio::spawn(async_play_track(app_arc.clone(), query));
                            }
                            KeyCode::Esc => {
                                app.input_mode = InputMode::Normal;
                                app.input.clear();
                            }
                            KeyCode::Char(c) => {
                                app.input.push(c);
                            }
                            KeyCode::Backspace => {
                                app.input.pop();
                            }
                            _ => {}
                        }
                    } else {
                        match app.view {
                            View::Menu => {
                                match key.code {
                                    KeyCode::Esc | KeyCode::Tab => app.view = View::Main,
                                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
                                        let i = match app.menu_state.selected() {
                                            Some(i) => if i >= app.menu_items.len() - 1 { 0 } else { i + 1 },
                                            None => 0,
                                        };
                                        app.menu_state.select(Some(i));
                                    }
                                    KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
                                        let i = match app.menu_state.selected() {
                                            Some(i) => if i == 0 { app.menu_items.len() - 1 } else { i - 1 },
                                            None => 0,
                                        };
                                        app.menu_state.select(Some(i));
                                    }
                                    KeyCode::Enter => {
                                        if let Some(idx) = app.menu_state.selected() {
                                            let item = app.menu_items[idx];
                                            match item {
                                                "Skip" => { tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), SimplePayload { action: "skip", guild_id: app.guild_id.clone(), user_id: app.user_id.clone() })); }
                                                "Pause/Resume" => { tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), SimplePayload { action: "pause", guild_id: app.guild_id.clone(), user_id: app.user_id.clone() })); }
                                                "Stop" => { tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), SimplePayload { action: "stop", guild_id: app.guild_id.clone(), user_id: app.user_id.clone() })); }
                                                "Shuffle" => { tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), SimplePayload { action: "shuffle", guild_id: app.guild_id.clone(), user_id: app.user_id.clone() })); }
                                                "Clear Queue" => { tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), SimplePayload { action: "clear", guild_id: app.guild_id.clone(), user_id: app.user_id.clone() })); }
                                                "Loop Track" => { app.loop_mode = "track".to_string(); tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), LoopPayload { action: "loop", guild_id: app.guild_id.clone(), user_id: app.user_id.clone(), loop_mode: "track".to_string() })); }
                                                "Loop Queue" => { app.loop_mode = "queue".to_string(); tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), LoopPayload { action: "loop", guild_id: app.guild_id.clone(), user_id: app.user_id.clone(), loop_mode: "queue".to_string() })); }
                                                "Loop Off" => { app.loop_mode = "off".to_string(); tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), LoopPayload { action: "loop", guild_id: app.guild_id.clone(), user_id: app.user_id.clone(), loop_mode: "off".to_string() })); }
                                                "24/7 Mode Toggle" => { tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), TwentyFourSevenPayload { action: "247", guild_id: app.guild_id.clone(), user_id: app.user_id.clone(), enabled: None })); }
                                                "Filters..." => { app.view = View::FilterMenu; }
                                                "Lyrics" => { tokio::spawn(async_fetch_lyrics(app_arc.clone())); }
                                                "Play Turip" => { tokio::spawn(async_play_track(app_arc.clone(), "https://open.spotify.com/track/2RQWB4Asy1rjZL4IUcJ7kn".to_string())); }
                                                "Auth" => { app.view = View::AuthMenu; }
                                                "Settings" => { 
                                                    app.settings_input = app.base_url.clone();
                                                    app.view = View::Settings; 
                                                }
                                                "Exit TUI" => return Ok(()),
                                                _ => {}
                                            }
                                            if item != "Filters..." && item != "Lyrics" && item != "Auth" && item != "Settings" {
                                                app.view = View::Main;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            },
                            View::Settings => {
                                match key.code {
                                    KeyCode::Enter => {
                                        let was_login_required = app.token.is_none();
                                        let old_host = app.base_url.clone();
                                        app.base_url = app.settings_input.clone();
                                        
                                        if let Ok(offset) = app.offset_input.parse::<i64>() {
                                            app.visualizer_offset = offset;
                                        }

                                        if old_host != app.base_url {
                                            app.needs_reconnect = true;
                                        }

                                        let settings = api::Settings { 
                                            base_url: app.base_url.clone(),
                                            visualizer_offset: app.visualizer_offset,
                                        };
                                        let _ = api::save_settings(&settings);
                                        
                                        if was_login_required {
                                            app.view = View::LoginRequired;
                                        } else {
                                            app.view = View::Main;
                                        }
                                        
                                        // Refresh data with new host
                                        tokio::spawn(async_fetch_queue(app_arc.clone()));
                                    }
                                    KeyCode::Esc => {
                                        if app.token.is_none() {
                                            app.view = View::LoginRequired;
                                        } else {
                                            app.view = View::Main;
                                        }
                                    }
                                    KeyCode::Down | KeyCode::Up | KeyCode::Tab => {
                                        app.settings_field = match app.settings_field {
                                            SettingsField::Host => SettingsField::Offset,
                                            SettingsField::Offset => SettingsField::Host,
                                        };
                                    }
                                    KeyCode::Backspace => {
                                        match app.settings_field {
                                            SettingsField::Host => { app.settings_input.pop(); }
                                            SettingsField::Offset => { app.offset_input.pop(); }
                                        }
                                    }
                                    KeyCode::Char(c) => {
                                        match app.settings_field {
                                            SettingsField::Host => { app.settings_input.push(c); }
                                            SettingsField::Offset => { 
                                                if c.is_ascii_digit() || (c == '-' && app.offset_input.is_empty()) { 
                                                    app.offset_input.push(c); 
                                                } 
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            },
                            View::AuthMenu => {
                                match key.code {
                                    KeyCode::Esc | KeyCode::Tab => app.view = View::Main,
                                    KeyCode::Backspace => app.view = View::Menu,
                                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
                                        let i = match app.auth_menu_state.selected() {
                                            Some(i) => if i >= app.auth_menu_items.len() - 1 { 0 } else { i + 1 },
                                            None => 0,
                                        };
                                        app.auth_menu_state.select(Some(i));
                                    }
                                    KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
                                        let i = match app.auth_menu_state.selected() {
                                            Some(i) => if i == 0 { app.auth_menu_items.len() - 1 } else { i - 1 },
                                            None => 0,
                                        };
                                        app.auth_menu_state.select(Some(i));
                                    }
                                    KeyCode::Enter => {
                                        if let Some(idx) = app.auth_menu_state.selected() {
                                            match app.auth_menu_items[idx] {
                                                "Login" => {
                                                    tokio::spawn(async_auth_login(app_arc.clone()));
                                                }
                                                "Signout" => {
                                                    tokio::spawn(async_auth_signout(app_arc.clone()));
                                                }
                                                "Info" => {
                                                    if let Some(auth) = api::load_auth() {
                                                        let mut info = String::new();
                                                        if let Some(path) = api::config_file_path() {
                                                            info.push_str(&format!("Auth file: {}\n", path.display()));
                                                        }
                                                        info.push_str(&format!("User: {}\n", auth.username.unwrap_or_else(|| "Unknown".to_string())));
                                                        if let Some(avatar) = auth.avatar_url {
                                                            info.push_str(&format!("Avatar: {}\n", avatar));
                                                        } else {
                                                            info.push_str("Avatar: (none)\n");
                                                        }
                                                        let token_masked = if auth.token.len() > 8 {
                                                            format!("{}...{}", &auth.token[0..4], &auth.token[auth.token.len() - 4..])
                                                        } else {
                                                            auth.token
                                                        };
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
                            },
                            View::AuthResult => {
                                match key.code {
                                    KeyCode::Esc | KeyCode::Enter | KeyCode::Backspace => app.view = View::AuthMenu,
                                    _ => {}
                                }
                            },
                            View::LoginRequired => {
                                match key.code {
                                    KeyCode::Enter => {
                                        tokio::spawn(async_auth_login(app_arc.clone()));
                                    }
                                    KeyCode::Char('\\') => {
                                        app.settings_input = app.base_url.clone();
                                        app.view = View::Settings;
                                    }
                                    KeyCode::Char('q') | KeyCode::Char('й') => return Ok(()),
                                    _ => {}
                                }
                            },
                            View::FilterMenu => {
                                match key.code {
                                    KeyCode::Esc => app.view = View::Main,
                                    KeyCode::Backspace => app.view = View::Menu,
                                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
                                        let i = match app.filter_state.selected() {
                                            Some(i) => if i >= app.filter_items.len() - 1 { 0 } else { i + 1 },
                                            None => 0,
                                        };
                                        app.filter_state.select(Some(i));
                                    }
                                    KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
                                        let i = match app.filter_state.selected() {
                                            Some(i) => if i == 0 { app.filter_items.len() - 1 } else { i - 1 },
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
                                            tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), payload));
                                            app.view = View::Main;
                                        }
                                    }
                                    _ => {}
                                }
                            },
                            View::Lyrics => {
                                match key.code {
                                    KeyCode::Esc => app.view = View::Main,
                                    KeyCode::Backspace => app.view = View::Menu,
                                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
                                        app.lyrics_scroll = app.lyrics_scroll.saturating_add(1);
                                    },
                                    KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
                                        app.lyrics_scroll = app.lyrics_scroll.saturating_sub(1);
                                    },
                                    _ => {}
                                }
                            },
                            View::Main => {
                                match key.code {
                                    KeyCode::Char('q') | KeyCode::Char('й') => return Ok(()),
                                    KeyCode::Char('r') | KeyCode::Char('к') => {
                                        tokio::spawn(async_fetch_queue(app_arc.clone()));
                                    }
                                    KeyCode::Tab => {
                                        app.view = View::Menu;
                                    }
                                    KeyCode::Enter => {
                                        app.input_mode = InputMode::Editing;
                                    }
                                    KeyCode::Char('l') | KeyCode::Char('д') => {
                                        let new_mode = match app.loop_mode.as_str() {
                                            "off" => "track",
                                            "track" => "queue",
                                            "queue" => "off",
                                            _ => "off",
                                        };
                                        app.loop_mode = new_mode.to_string();
                                        tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), LoopPayload { action: "loop", guild_id: app.guild_id.clone(), user_id: app.user_id.clone(), loop_mode: new_mode.to_string() }));
                                    }
                                    KeyCode::Char('s') | KeyCode::Char('ы') | KeyCode::Char('і') => {
                                        tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), SimplePayload { action: "skip", guild_id: app.guild_id.clone(), user_id: app.user_id.clone() }));
                                    }
                                    KeyCode::Char('w') | KeyCode::Char('ц') => {
                                        tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), SimplePayload { action: "stop", guild_id: app.guild_id.clone(), user_id: app.user_id.clone() }));
                                    }
                                    KeyCode::Char('c') | KeyCode::Char('с') => {
                                        tokio::spawn(async_simple_command(app_arc.clone(), "/webhook/audio".to_string(), SimplePayload { action: "clear", guild_id: app.guild_id.clone(), user_id: app.user_id.clone() }));
                                    }
                                    KeyCode::Char('d') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                                        app.view = View::Debug;
                                    }
                                    KeyCode::Char(c) => {
                                        app.input_mode = InputMode::Editing;
                                        app.input.push(c);
                                    }
                                    _ => {}
                                }
                            }
                            View::Debug => {
                                match key.code {
                                    KeyCode::Char('s') | KeyCode::Char('ы') => {
                                        app.save_spectrogram();
                                    }
                                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('й') => {
                                        if app.token.is_none() {
                                            app.view = View::LoginRequired;
                                        } else {
                                            app.view = View::Main;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn get_filters_for_style(style: &str) -> AudioFilters {
    match style.to_lowercase().as_str() {
        "clear" => AudioFilters::default(),
        "bassboost" => AudioFilters {
            equalizer: Some(vec![
                EqualizerBand { band: 0, gain: 0.2 },
                EqualizerBand { band: 1, gain: 0.15 },
                EqualizerBand { band: 2, gain: 0.1 },
                EqualizerBand { band: 3, gain: 0.05 },
                EqualizerBand { band: 4, gain: 0.0 },
                EqualizerBand { band: 5, gain: -0.05 },
            ]),
            ..Default::default()
        },
        "soft" => AudioFilters {
            low_pass: Some(LowPassOptions { smoothing: Some(20.0) }),
            ..Default::default()
        },
        "nightcore" => AudioFilters {
            timescale: Some(TimescaleOptions { speed: Some(1.1), pitch: Some(1.1), rate: Some(1.0) }),
            ..Default::default()
        },
        "vaporwave" => AudioFilters {
            timescale: Some(TimescaleOptions { speed: Some(0.85), pitch: Some(0.8), rate: Some(1.0) }),
            ..Default::default()
        },
        "8d" => AudioFilters {
            rotation: Some(RotationOptions { rotation_hz: Some(0.2) }),
            ..Default::default()
        },
        "tremolo" => AudioFilters {
            tremolo: Some(TremoloOptions { frequency: Some(2.0), depth: Some(0.5) }),
            ..Default::default()
        },
        "vibrato" => AudioFilters {
            vibrato: Some(VibratoOptions { frequency: Some(2.0), depth: Some(0.5) }),
            ..Default::default()
        },
        "karaoke" => AudioFilters {
            karaoke: Some(KaraokeOptions { level: Some(1.0), mono_level: Some(1.0), filter_band: Some(220.0), filter_width: Some(100.0) }),
            ..Default::default()
        },
        _ => AudioFilters::default(),
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    if app.view == View::LoginRequired {
        let area = f.area();
        f.render_widget(Clear, area);
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(12), // Logo
                Constraint::Length(2),  // Spacer
                Constraint::Length(8),  // Text
                Constraint::Min(1),
            ])
            .split(area);

        // Logo
        let art_text: Vec<Line> = ASCII_LOGO.iter().map(|s| Line::from(Span::styled(*s, Style::default().fg(JORIK_PURPLE)))).collect();
        let art_paragraph = Paragraph::new(art_text)
            .alignment(Alignment::Center);
        f.render_widget(art_paragraph, chunks[1]);

        // Text
        let text = if app.is_loading || (app.auth_info_text.is_some() && app.auth_info_text.as_deref() != Some("Initializing login...")) {
             let status = app.auth_info_text.clone().unwrap_or_else(|| "Authenticating...".to_string());
             vec![
                Line::from(Span::styled("Authenticating...", Style::default().add_modifier(Modifier::BOLD).fg(Color::Yellow))),
                Line::from(""),
                Line::from(status),
             ]
        } else {
             vec![
                Line::from(Span::styled("Authentication Required", Style::default().add_modifier(Modifier::BOLD).fg(Color::Red))),
                Line::from(""),
                Line::from("To use Jorik CLI, you must log in with your Discord account."),
                Line::from("This allows us to access your voice channels and manage playback."),
                Line::from(""),
                Line::from(vec![
                    Span::raw("Press "),
                    Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD).fg(JORIK_PURPLE)),
                    Span::raw(" to Login"),
                ]),
                Line::from(vec![
                    Span::raw("Press "),
                    Span::styled("\\", Style::default().add_modifier(Modifier::BOLD).fg(JORIK_PURPLE)),
                    Span::raw(" to Change Host"),
                ]),
                Line::from(vec![
                    Span::raw("Press "),
                    Span::styled("q", Style::default().add_modifier(Modifier::BOLD).fg(JORIK_PURPLE)),
                    Span::raw(" to Quit"),
                ]),
            ]
        };
        
        let p = Paragraph::new(text)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
        f.render_widget(p, chunks[3]);
        return;
    }

    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    let top_section = main_layout[0];
    let status_bar_area = main_layout[1];

    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(65),
            Constraint::Percentage(35),
        ])
        .split(top_section);

    let left_side = content_chunks[0];
    let spectrogram_area = content_chunks[1];

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12),
            Constraint::Min(0),
        ])
        .split(left_side);

    let logo_area = left_chunks[0];
    let queue_area = left_chunks[1];

    // 1. ASCII Art
    let art_text: Vec<Line> = ASCII_LOGO.iter().map(|s| Line::from(Span::styled(*s, Style::default().fg(JORIK_PURPLE)))).collect();
    let art_paragraph = Paragraph::new(art_text)
        .alignment(Alignment::Center)
        .block(Block::default());
    f.render_widget(art_paragraph, logo_area);

    // 2. Main Content (Queue or Error)
    let loop_status = app.loop_mode.to_uppercase();
    let loading_indicator = if app.is_loading { " ⏳ Loading... " } else { " " };
    let title = format!(" Queue (Loop: {}){} ", loop_status, loading_indicator);
    
    let content_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(JORIK_PURPLE))
        .title_style(Style::default().fg(JORIK_PURPLE).add_modifier(Modifier::BOLD))
        .title(title)
        .style(Style::default());

    if let Some(err) = &app.error_message {
        let p = Paragraph::new(format!("⚠ {}", err))
            .style(Style::default().fg(Color::Red))
            .block(content_block)
            .wrap(Wrap { trim: true });
        f.render_widget(p, queue_area);
    } else {
        let mut items = Vec::new();
        
        if let Some(current) = &app.current_track {
             items.push(ListItem::new(Line::from(vec![
                Span::styled(" NOW PLAYING ", Style::default().bg(JORIK_PURPLE).fg(Color::Black).add_modifier(Modifier::BOLD)),
             ])));
             items.push(ListItem::new(Line::from(vec![
                Span::styled("   ▶ ", Style::default().fg(JORIK_PURPLE)),
                Span::styled(current, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ])));

            // Progress Bar
            if app.duration_ms > 0 {
                let ratio = (app.elapsed_ms as f64 / app.duration_ms as f64).min(1.0);
                let pct_val = (ratio * 100.0) as usize;
                let pct = (ratio * 30.0).round() as usize;
                let bar = "━".repeat(pct.min(30)) + "⚪" + &"━".repeat(30usize.saturating_sub(pct));
                let time_str = format!(
                    " {:02}:{:02} / {:02}:{:02} ({:3}%) ",
                    app.elapsed_ms / 60000,
                    (app.elapsed_ms % 60000) / 1000,
                    app.duration_ms / 60000,
                    (app.duration_ms % 60000) / 1000,
                    pct_val
                );
                items.push(ListItem::new(Line::from(vec![
                    Span::styled("   ", Style::default()),
                    Span::styled(bar, Style::default().fg(JORIK_PURPLE)),
                    Span::styled(time_str, Style::default().fg(Color::Gray)),
                ])));
            }
            items.push(ListItem::new(Span::raw("")));
        } else {
            items.push(ListItem::new(Span::styled("Nothing playing", Style::default().fg(Color::DarkGray))));
            items.push(ListItem::new(Span::raw("")));
        }
        
        if !app.queue.is_empty() {
             items.push(ListItem::new(Line::from(vec![
                Span::styled(" UP NEXT ", Style::default().fg(JORIK_PURPLE).add_modifier(Modifier::BOLD)),
             ])));
             for (i, track) in app.queue.iter().enumerate() {
                items.push(ListItem::new(format!("   {}. {}", i + 1, track)).style(Style::default().fg(Color::Gray)));
            }
        } else {
             items.push(ListItem::new(Span::styled("   Queue is empty", Style::default().fg(Color::DarkGray))));
        }

        let list = List::new(items)
            .block(content_block);
        f.render_widget(list, queue_area);
    }

    // Spectrogram
    let spec_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(JORIK_PURPLE))
        .title(" Visualizer ")
        .title_style(Style::default().fg(JORIK_PURPLE).add_modifier(Modifier::BOLD));

    if app.current_track.is_some() {
        // Bar width 2 + Gap 1 = 3 cells per bar
        let num_bars = (spectrogram_area.width / 3).min(64) as usize;
        let mut bar_items = Vec::with_capacity(num_bars);

        if num_bars > 0 {
            let bins_per_bar = 64.0 / num_bars as f32;
            for j in 0..num_bars {
                let start_f = j as f32 * bins_per_bar;
                let end_f = (j + 1) as f32 * bins_per_bar;
                
                let mut sum = 0.0;
                let mut weight = 0.0;
                
                for i in 0..64 {
                    let overlap = ((i + 1) as f32).min(end_f) - (i as f32).max(start_f);
                    if overlap > 0.0 {
                        sum += app.smoothed_bars[i] * overlap;
                        weight += overlap;
                    }
                }
                let val = if weight > 0.0 { sum / weight } else { 0.0 };
                bar_items.push(val as u64);
            }
        }

        let bar_labels: Vec<String> = bar_items.iter()
            .map(|&v| format!("{:2}", v.min(99)))
            .collect();

        let bars: Vec<Bar> = bar_items.iter().enumerate()
            .map(|(i, &v)| {
                Bar::default()
                    .value(v)
                    .label(Span::from(bar_labels[i].as_str()))
                    .text_value(String::new())
            })
            .collect();
        
        let bar_group = BarGroup::default().bars(&bars);
        
        // Split area into chart and labels
        let spec_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(spec_block.inner(spectrogram_area));

        let barchart = BarChart::default()
            .data(bar_group)
            .bar_width(2)
            .bar_gap(1)
            .max(100) 
            .bar_style(Style::default().fg(JORIK_PURPLE))
            .label_style(Style::default().fg(Color::White));
        
        f.render_widget(spec_block, spectrogram_area);
        f.render_widget(barchart, spec_chunks[0]);

        // Custom label rendering for frequency
        let labels = ["30", "100", "500", "1k", "5k", "10k", "20k"];
        let mut label_spans = Vec::new();
        let total_w = spec_chunks[1].width as usize;
        
        if total_w > 10 {
            for (i, &l) in labels.iter().enumerate() {
                let pos = (i as f32 / (labels.len() - 1) as f32 * (total_w - l.len()) as f32) as usize;
                let current_len: usize = label_spans.iter().map(|s: &Span| s.content.len()).sum();
                if pos > current_len {
                    label_spans.push(Span::raw(" ".repeat(pos - current_len)));
                }
                label_spans.push(Span::styled(l, Style::default().fg(Color::DarkGray)));
            }
            f.render_widget(Paragraph::new(Line::from(label_spans)), spec_chunks[1]);
        }
    } else {
        f.render_widget(Paragraph::new("Idle (No Track)").block(spec_block).alignment(Alignment::Center), spectrogram_area);
    }

    // 3. Status Bar / Hint
    if app.input_mode == InputMode::Normal && app.view == View::Main {
        let keys = vec![
            ("Type", "Search"),
            ("Enter", "Play"),
            ("Tab", "Menu"),
            ("s", "Skip"),
            ("w", "Stop"),
            ("c", "Clear"),
            ("l", "Loop"),
            ("r", "Refresh"),
            ("q", "Quit"),
        ];
        
        let mut spans = Vec::new();
        for (key, desc) in keys {
            spans.push(Span::styled(format!(" {} ", key), Style::default().bg(JORIK_PURPLE).fg(Color::Black).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(format!(" {} ", desc), Style::default().fg(Color::Gray)));
            spans.push(Span::raw(" "));
        }

        let p = Paragraph::new(Line::from(spans))
            .style(Style::default())
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::TOP).border_type(BorderType::Double).border_style(Style::default().fg(Color::DarkGray)));
            
        f.render_widget(p, status_bar_area);
    }

    // Overlays

    // Input Box
    if app.input_mode == InputMode::Editing {
        let area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, area);
        
        let loading_text = if app.is_loading { " ⏳ " } else { "" };
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(format!(" Play / Search {} ", loading_text))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(JORIK_HIGHLIGHT));
        
        let p = Paragraph::new(app.input.as_str())
            .block(input_block)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: true });
        f.render_widget(p, area);
    }

    // Menu Box
    if app.view == View::Menu {
        let area = centered_rect(40, 50, f.area());
        f.render_widget(Clear, area);
        
        let loading_text = if app.is_loading { " ⏳ " } else { "" };
        let menu_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(format!(" Menu {} ", loading_text))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(JORIK_PURPLE));
        
        let items: Vec<ListItem> = app.menu_items
            .iter()
            .map(|i| ListItem::new(format!("  {}  ", *i)))
            .collect();
            
        let list = List::new(items)
            .block(menu_block)
            .highlight_style(Style::default().bg(JORIK_PURPLE).fg(Color::White).add_modifier(Modifier::BOLD))
            .highlight_symbol(" ➤ ");
            
        f.render_stateful_widget(list, area, &mut app.menu_state);
    }

    // Filter Menu Box
    if app.view == View::FilterMenu {
        let area = centered_rect(40, 50, f.area());
        f.render_widget(Clear, area);
        
        let loading_text = if app.is_loading { " ⏳ " } else { "" };
        let menu_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(format!(" Select Filter {} ", loading_text))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(JORIK_PURPLE));
        
        let items: Vec<ListItem> = app.filter_items
            .iter()
            .map(|i| ListItem::new(format!("  {}  ", *i)))
            .collect();
            
        let list = List::new(items)
            .block(menu_block)
            .highlight_style(Style::default().bg(JORIK_PURPLE).fg(Color::White).add_modifier(Modifier::BOLD))
            .highlight_symbol(" ➤ ");
            
        f.render_stateful_widget(list, area, &mut app.filter_state);
    }

    // Auth Menu Box
    if app.view == View::AuthMenu {
        let area = centered_rect(40, 40, f.area());
        f.render_widget(Clear, area);
        
        let loading_text = if app.is_loading { " ⏳ " } else { "" };
        let menu_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(format!(" Auth {} ", loading_text))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(JORIK_PURPLE));
        
        let items: Vec<ListItem> = app.auth_menu_items
            .iter()
            .map(|i| ListItem::new(format!("  {}  ", *i)))
            .collect();
            
        let list = List::new(items)
            .block(menu_block)
            .highlight_style(Style::default().bg(JORIK_PURPLE).fg(Color::White).add_modifier(Modifier::BOLD))
            .highlight_symbol(" ➤ ");
            
        f.render_stateful_widget(list, area, &mut app.auth_menu_state);
    }

    // Auth Result/Info Box
    if app.view == View::AuthResult {
        let area = centered_rect(60, 40, f.area());
        f.render_widget(Clear, area);
        
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Auth Info ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(JORIK_PURPLE));
        
        let text = app.auth_info_text.as_deref().unwrap_or("No data.");
        let p = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: true });
            
        f.render_widget(p, area);
    }

    // Lyrics Box
    if app.view == View::Lyrics {
        let area = centered_rect(70, 70, f.area());
        f.render_widget(Clear, area);
        
        let loading_text = if app.is_loading { " ⏳ " } else { "" };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(format!(" Lyrics {} ", loading_text))
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(JORIK_PURPLE));
        
        let text = app.lyrics_text.as_deref().unwrap_or("Loading...");
        let p = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((app.lyrics_scroll, 0));
            
        f.render_widget(p, area);
    }

    // Settings Box
    if app.view == View::Settings {
        let area = centered_rect(60, 30, f.area());
        f.render_widget(Clear, area);
        
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" Settings ")
            .title_alignment(Alignment::Center)
            .border_style(Style::default().fg(JORIK_PURPLE));
        
        let is_editing_host = app.settings_field == SettingsField::Host;
        
        let host_style = if is_editing_host { Style::default().fg(Color::White).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::DarkGray) };
        let offset_style = if !is_editing_host { Style::default().fg(Color::White).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::DarkGray) };

        let host_label = if is_editing_host { "▶ Webhook Host: " } else { "  Webhook Host: " };
        let offset_label = if !is_editing_host { "▶ Visualizer Offset (ms): " } else { "  Visualizer Offset (ms): " };

        let p = Paragraph::new(vec![
            Line::from("Configure your connection and visualizer sync:"),
            Line::from(""),
            Line::from(vec![
                Span::styled(host_label, host_style),
                Span::styled(&app.settings_input, host_style),
            ]),
            Line::from(vec![
                Span::styled(offset_label, offset_style),
                Span::styled(&app.offset_input, offset_style),
            ]),
            Line::from(""),
            Line::from(Span::styled("Use Arrows/Tab to switch, Enter to Save", Style::default().fg(Color::Gray))),
        ])
        .block(block)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });
            
        f.render_widget(p, area);
    }

    // Debug Box
    if app.view == View::Debug {
        let area = centered_rect(80, 80, f.area());
        f.render_widget(Clear, area);
        
        let ws_status = if app.ws_connected {
            Span::styled(" CONNECTED ", Style::default().bg(Color::Green).fg(Color::Black).add_modifier(Modifier::BOLD))
        } else if app.ws_connecting {
            Span::styled(" CONNECTING... ", Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(" DISCONNECTED ", Style::default().bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD))
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(vec![
                Span::raw(" Debug Console "), 
                ws_status,
                Span::raw(" (Press 's' to Save Spectrogram) ")
            ])
            .title_alignment(Alignment::Left)
            .border_style(Style::default().fg(Color::Yellow));
        
        let log_lines: Vec<Line> = app.debug_logs.iter()
            .rev()
            .map(|l| Line::from(l.as_str()))
            .collect();
            
        let p = Paragraph::new(log_lines)
            .block(block)
            .wrap(Wrap { trim: false });
            
        f.render_widget(p, area);
    }

    // Fatal Error Overlay
    if let Some(msg) = &app.fatal_error {
        let area = centered_rect(60, 25, f.area());
        f.render_widget(Clear, area);
        
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .title(" ⚠ Connection Error ")
            .title_alignment(Alignment::Center)
            .style(Style::default())
            .border_style(Style::default().fg(Color::Red));
        
        let p = Paragraph::new(msg.as_str())
            .block(block)
            .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });
            
        f.render_widget(p, area);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    let horiz_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1]);

    horiz_layout[1]
}