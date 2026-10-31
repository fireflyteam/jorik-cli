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
    "Standard".to_string()
}

pub fn config_file_path() -> Option<PathBuf> {
    config_dir().map(|p| p.join("jorik-cli").join("auth.json"))
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
    write_auth_file(&path, &json)?;
    Ok(())
}

fn write_auth_file(path: &std::path::Path, json: &str) -> Result<()> {
    use std::io::Write;

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).context("opening auth file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .context("restricting auth file permissions")?;
    }
    file.set_len(0).context("truncating auth file")?;
    file.write_all(json.as_bytes())
        .context("writing auth file")?;
    Ok(())
}

pub fn load_auth() -> Option<Auth> {
    if let Some(path) = config_file_path()
        && let Ok(contents) = fs::read_to_string(&path)
        && let Ok(auth) = serde_json::from_str::<Auth>(&contents)
    {
        return Some(auth);
    }
    None
}

pub fn load_token() -> Option<String> {
    load_auth().map(|a| a.token)
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
