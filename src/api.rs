use anyhow::{Context, Result};
use dirs::config_dir;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct PlayPayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    pub query: String,
    pub user_id: Option<String>,
    pub requested_by: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Serialize)]
pub struct SimplePayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Serialize)]
pub struct QueuePayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Serialize)]
pub struct LoopPayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    pub loop_mode: String,
}

#[derive(Serialize)]
pub struct TwentyFourSevenPayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Serialize)]
pub struct FilterPayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    pub filters: AudioFilters,
}

#[derive(Serialize)]
pub struct LyricsPayload {
    pub action: String,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Serialize, Default, Clone)]
pub struct AudioFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub equalizer: Option<Vec<EqualizerBand>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub karaoke: Option<KaraokeOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timescale: Option<TimescaleOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tremolo: Option<TremoloOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vibrato: Option<VibratoOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotation: Option<RotationOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distortion: Option<DistortionOptions>,
    #[serde(rename = "channelMix", skip_serializing_if = "Option::is_none")]
    pub channel_mix: Option<ChannelMixOptions>,
    #[serde(rename = "lowPass", skip_serializing_if = "Option::is_none")]
    pub low_pass: Option<LowPassOptions>,
}

#[derive(Serialize, Clone)]
pub struct EqualizerBand {
    pub band: i32,
    pub gain: f32,
}

#[derive(Serialize, Clone)]
pub struct KaraokeOptions {
    pub level: Option<f32>,
    #[serde(rename = "monoLevel")]
    pub mono_level: Option<f32>,
    #[serde(rename = "filterBand")]
    pub filter_band: Option<f32>,
    #[serde(rename = "filterWidth")]
    pub filter_width: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct TimescaleOptions {
    pub speed: Option<f32>,
    pub pitch: Option<f32>,
    pub rate: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct TremoloOptions {
    pub frequency: Option<f32>,
    pub depth: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct VibratoOptions {
    pub frequency: Option<f32>,
    pub depth: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct RotationOptions {
    #[serde(rename = "rotationHz")]
    pub rotation_hz: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct DistortionOptions {
    #[serde(rename = "sinOffset")]
    pub sin_offset: Option<f32>,
    #[serde(rename = "sinScale")]
    pub sin_scale: Option<f32>,
    #[serde(rename = "cosOffset")]
    pub cos_offset: Option<f32>,
    #[serde(rename = "cosScale")]
    pub cos_scale: Option<f32>,
    #[serde(rename = "tanOffset")]
    pub tan_offset: Option<f32>,
    #[serde(rename = "tanScale")]
    pub tan_scale: Option<f32>,
    pub offset: Option<f32>,
    pub scale: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct ChannelMixOptions {
    #[serde(rename = "leftToLeft")]
    pub left_to_left: Option<f32>,
    #[serde(rename = "leftToRight")]
    pub left_to_right: Option<f32>,
    #[serde(rename = "rightToLeft")]
    pub right_to_left: Option<f32>,
    #[serde(rename = "rightToRight")]
    pub right_to_right: Option<f32>,
}

#[derive(Serialize, Clone)]
pub struct LowPassOptions {
    pub smoothing: Option<f32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Auth {
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct WsEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "guildId")]
    pub guild_id: Option<String>,
    pub data: Option<Value>,
    pub playback: Option<PlaybackState>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PlaybackState {
    #[serde(rename = "elapsedMs")]
    pub elapsed_ms: u64,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    pub paused: bool,
    pub spectrogram: Option<Vec<Vec<u8>>>,
}

#[derive(Serialize)]
pub struct WsSubscribe {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    #[serde(rename = "guildId")]
    pub guild_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub base_url: String,
    #[serde(default = "default_offset")]
    pub visualizer_offset: i64,
}

fn default_offset() -> i64 { 200 }

pub fn config_file_path() -> Option<PathBuf> {
    config_dir().map(|p| p.join("jorik-cli").join("auth.json"))
}

pub fn settings_file_path() -> Option<PathBuf> {
    config_dir().map(|p| p.join("jorik-cli").join("settings.json"))
}

pub fn load_settings() -> Settings {
    if let Some(path) = settings_file_path() {
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<Settings>(&contents) {
                return settings;
            }
        }
    }
    Settings {
        base_url: "https://jorik.xserv.pp.ua".to_string(),
        visualizer_offset: 200,
    }
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    let path = settings_file_path().context("cannot determine settings path")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating config directory")?;
    }
    let json = serde_json::to_string_pretty(settings).context("serializing settings")?;
    fs::write(&path, json).context("writing settings file")?;
    Ok(())
}

pub fn save_token(token: &str, avatar_url: Option<&str>, username: Option<&str>) -> Result<()> {
    let path = config_file_path().context("cannot determine config path")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating config directory")?;
    }

    let auth = Auth {
        token: token.trim().to_string(),
        avatar_url: avatar_url.map(|s| s.to_string()),
        username: username.map(|s| s.to_string()),
    };

    let json = serde_json::to_string_pretty(&auth).context("serializing auth")?;
    fs::write(&path, json).context("writing auth file")?;
    Ok(())
}

pub fn load_auth() -> Option<Auth> {
    // Try to load the canonical auth.json first.
    if let Some(path) = config_file_path() {
        if let Ok(contents) = fs::read_to_string(&path) {
            if let Ok(auth) = serde_json::from_str::<Auth>(&contents) {
                return Some(auth);
            }
        }
    }
    // Note: Legacy token fallback removed from shared logic to keep it simple,
    // or we can add it back if strictly necessary, but main.rs had specific printing logic.
    // For now, let's include it but without the printing side-effects if possible,
    // or just rely on auth.json.
    // The original code printed a warning.
    None
}

pub fn load_token() -> Option<String> {
    load_auth().map(|a| a.token)
}

pub fn build_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

pub fn clean_query(input: &str) -> String {
    if let Ok(mut url) = Url::parse(input) {
        if url.cannot_be_a_base() || url.query().is_none() {
            return input.to_string();
        }

        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(k, _)| k != "si")
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        if pairs.is_empty() {
            url.set_query(None);
        } else {
            let mut serializer = url.query_pairs_mut();
            serializer.clear();
            for (k, v) in pairs {
                serializer.append_pair(&k, &v);
            }
        }
        return url.to_string();
    }
    input.to_string()
}
