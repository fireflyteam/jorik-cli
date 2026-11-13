pub use crate::GiteaAsset;
use anyhow::{Context, Result};
use dirs::config_dir;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Clone)]
pub struct PlayPayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub channel_id: Option<String>,
    pub query: String,
    pub user_id: Option<String>,
    pub requested_by: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct SimplePayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct QueuePayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Serialize, Clone)]
pub struct LoopPayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    pub loop_mode: String,
}

#[derive(Serialize, Clone)]
pub struct TwentyFourSevenPayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Serialize, Clone)]
pub struct FilterPayload {
    pub action: &'static str,
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    pub filters: AudioFilters,
}

#[derive(Serialize, Clone)]
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

#[derive(Deserialize, Debug, Clone)]
pub struct WsEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "guildId")]
    pub guild_id: Option<String>,
    pub data: Option<Value>,
    pub playback: Option<PlaybackState>,
    pub success: Option<bool>,
    pub id: Option<String>,
    pub message: Option<String>,
    pub error: Option<Value>,
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

#[derive(Serialize)]
pub struct WsAction<T> {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub id: String,
    #[serde(flatten)]
    pub payload: T,
}

pub const DEFAULT_BASE_URL: &str = "https://jorik.gleckson.com";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub base_url: String,
    #[serde(default = "default_offset")]
    pub visualizer_offset: i64,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_viz")]
    pub visualizer_style: String,
    #[serde(default = "default_layout")]
    pub layout: String,
}

fn default_offset() -> i64 {
    0
}
fn default_theme() -> String {
    "Default".to_string()
}
fn default_viz() -> String {
    "Bars".to_string()
}
fn default_layout() -> String {
    "Desk".to_string()
}

pub fn settings_file_path() -> Option<PathBuf> {
    config_dir().map(|p| p.join("jorik-cli").join("settings.json"))
}

pub fn load_settings() -> Settings {
    if let Some(path) = settings_file_path()
        && let Ok(contents) = fs::read_to_string(&path)
        && let Ok(settings) = serde_json::from_str::<Settings>(&contents)
    {
        return migrate_saved_settings(settings);
    }
    Settings {
        base_url: DEFAULT_BASE_URL.to_string(),
        visualizer_offset: 0,
        theme: "Default".to_string(),
        visualizer_style: "Bars".to_string(),
        layout: "Standard".to_string(),
    }
}

fn migrate_saved_settings(mut settings: Settings) -> Settings {
    // Only migrate the retired default. Explicit CLI/environment overrides and
    // custom saved hosts are left untouched; no config file is rewritten here.
    if settings.base_url.trim_end_matches('/') == "https://jorik.xserv.pp.ua" {
        settings.base_url = DEFAULT_BASE_URL.to_string();
    }
    settings
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

pub fn validate_base_url(base: &str) -> Result<()> {
    let url = Url::parse(base).context("invalid server URL")?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "server URL must use http or https and include a host"
    );
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "server URL must not contain credentials, a query, or a fragment"
    );
    Ok(())
}

pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Bound decoded bytes even when Content-Length is absent or dishonest.
pub async fn response_text(mut response: reqwest::Response) -> Result<String> {
    if let Some(length) = response.content_length() {
        anyhow::ensure!(
            length <= MAX_RESPONSE_BYTES as u64,
            "server response exceeds the size limit"
        );
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("server response could not be read"))?
    {
        anyhow::ensure!(
            chunk.len() <= MAX_RESPONSE_BYTES.saturating_sub(body.len()),
            "server response exceeds the size limit"
        );
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| anyhow::anyhow!("server response is not valid UTF-8"))
}

pub fn build_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;

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
