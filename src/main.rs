use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use colored::Colorize;
use dirs::config_dir;
use open::that;
use reqwest::Client;
use serde::Serialize;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

/// CLI to interact with the Jorik webhook server (play/skip/stop/health).
#[derive(Parser, Debug)]
#[command(name = "jorik", author, version, about)]
struct Cli {
    /// Base URL of the webhook server (e.g., https://jorik.xserv.pp.ua)
    #[arg(
        long,
        global = true,
        env = "JORIK_BASE_URL",
        default_value = "https://jorik.xserv.pp.ua"
    )]
    base_url: String,

    /// Bearer token for authorization (if required by the server)
    #[arg(long, global = true, env = "JORIK_TOKEN")]
    token: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Check server health
    Health,
    /// Enqueue audio to play
    Play {
        /// Query/URL to play (required)
        query: String,
        /// Guild ID (optional if the server can infer from user_id)
        #[arg(long)]
        guild_id: Option<String>,
        /// Voice channel ID (optional if the server can infer from user_id)
        #[arg(long)]
        channel_id: Option<String>,
        /// User ID for context/authorization
        #[arg(long)]
        user_id: Option<String>,
        /// Override display name
        #[arg(long)]
        requested_by: Option<String>,
        /// Avatar URL to show in Discord
        #[arg(long)]
        avatar_url: Option<String>,
    },
    /// Skip the current track
    Skip {
        /// Guild ID (optional if the server can infer from user_id)
        #[arg(long)]
        guild_id: Option<String>,
        /// User ID for context/authorization
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Stop playback and clear queue
    Stop {
        /// Guild ID (optional if the server can infer from user_id)
        #[arg(long)]
        guild_id: Option<String>,
        /// User ID for context/authorization
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Obtain and store a bearer token via browser auth
    Login,
}

#[derive(Serialize)]
struct PlayPayload {
    action: &'static str,
    guild_id: Option<String>,
    channel_id: Option<String>,
    query: String,
    user_id: Option<String>,
    requested_by: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Serialize)]
struct SkipPayload {
    action: &'static str,
    guild_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Serialize)]
struct StopPayload {
    action: &'static str,
    guild_id: Option<String>,
    user_id: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("jorik-cli/0.1.0")
        .build()
        .context("building HTTP client")?;

    let token = cli.token.clone().or_else(load_token);

    match cli.command {
        Commands::Health => health(&client, &cli.base_url).await?,
        Commands::Play {
            query,
            guild_id,
            channel_id,
            user_id,
            requested_by,
            avatar_url,
        } => {
            let payload = PlayPayload {
                action: "play",
                guild_id,
                channel_id,
                query,
                user_id,
                requested_by,
                avatar_url,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Skip { guild_id, user_id } => {
            let payload = SkipPayload {
                action: "skip",
                guild_id,
                user_id,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Stop { guild_id, user_id } => {
            let payload = StopPayload {
                action: "stop",
                guild_id,
                user_id,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Login => {
            login(&cli.base_url).await?;
        }
    }

    Ok(())
}

async fn health(client: &Client, base_url: &str) -> Result<()> {
    let url = build_url(base_url, "/health");
    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    print_response(resp).await
}

async fn post_audio<T: Serialize>(
    client: &Client,
    base_url: &str,
    token: Option<&str>,
    payload: &T,
) -> Result<()> {
    let url = build_url(base_url, "/webhook/audio");
    let mut req = client.post(&url).json(payload);
    if let Some(bearer) = token {
        req = req.bearer_auth(bearer);
    }
    let resp = req.send().await.with_context(|| format!("POST {url}"))?;
    print_response(resp).await
}

async fn print_response(resp: reqwest::Response) -> Result<()> {
    let status = resp.status();
    let text = resp.text().await.context("reading response body")?;

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
        let status_colored = color_status(status);
        if let Some(summary) = summarize(&json) {
            println!("[{}]\n{}", status_colored, summary);
        } else {
            println!("[{}]\n{}", status_colored, json.to_string());
        }
    } else {
        let status_colored = color_status(status);
        println!("[{}] {}", status_colored, text);
    }

    Ok(())
}

fn summarize(json: &serde_json::Value) -> Option<String> {
    let obj = json.as_object()?;
    if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
        let msg = obj.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let hint = if err == "unauthorized" {
            format!(
                "\n  {}",
                "hint: run `jorik login` or provide --guild-id and --channel-id if permitted"
                    .blue()
            )
        } else {
            String::new()
        };
        let header = "error".red().bold();
        return Some(format!("{header}:\n  code: {err}\n  message: {msg}{hint}"));
    }
    let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("");
    let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("");
    match action {
        "play" => {
            let header = "play".cyan().bold();
            let started = obj
                .get("started")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let position = obj.get("position").and_then(|v| v.as_u64()).unwrap_or(0);
            let dropped = obj
                .get("dropped")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let tracks = obj.get("tracks").and_then(|v| v.as_array());
            let tracks_len = tracks.map(|t| t.len()).unwrap_or(0);
            let first = tracks.and_then(|t| t.get(0)).and_then(|v| v.as_object());
            let title = first
                .and_then(|o| o.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let url = first
                .and_then(|o| o.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "{header}:\n  status: {status}\n  started: {started}\n  dropped: {dropped}\n  position: {position}\n  tracks: {tracks_len}\n  first: \"{title}\"\n  url: {url}"
            ))
        }
        "skip" => {
            let header = "skip".magenta().bold();
            if let Some(skipped) = obj.get("skipped").and_then(|v| v.as_object()) {
                let title = skipped.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let url = skipped.get("url").and_then(|v| v.as_str()).unwrap_or("");
                Some(format!(
                    "{header}:\n  status: {status}\n  skipped: \"{title}\"\n  url: {url}"
                ))
            } else if let Some(msg) = obj.get("message").and_then(|v| v.as_str()) {
                Some(format!("{header}:\n  status: {status}\n  message: {msg}"))
            } else {
                Some(format!("{header}:\n  status: {status}"))
            }
        }
        "stop" => {
            let header = "stop".yellow().bold();
            let stopped = obj
                .get("stopped")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(format!(
                "{header}:\n  status: {status}\n  stopped: {stopped}"
            ))
        }
        _ => {
            if !action.is_empty() || !status.is_empty() {
                let header = "status".white().bold();
                Some(format!("{header}:\n  status: {status}\n  action: {action}"))
            } else {
                None
            }
        }
    }
}

fn color_status(status: reqwest::StatusCode) -> colored::ColoredString {
    let code = status.as_u16();
    let text = status.to_string();
    if (200..300).contains(&code) {
        text.green().bold()
    } else if (400..500).contains(&code) {
        text.yellow().bold()
    } else if code >= 500 {
        text.red().bold()
    } else {
        text.normal()
    }
}

fn config_file_path() -> Option<PathBuf> {
    config_dir().map(|p| p.join("jorik-cli").join("token"))
}

fn save_token(token: &str) -> Result<()> {
    let path = config_file_path().context("cannot determine config path")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating config directory")?;
    }
    fs::write(&path, token.trim()).context("writing token file")?;
    Ok(())
}

fn load_token() -> Option<String> {
    let path = config_file_path()?;
    let contents = fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

async fn login(base_url: &str) -> Result<()> {
    let auth_url = build_url(base_url, "/authorize");
    println!("Opening browser for authorization: {auth_url}");
    let _ = that(&auth_url);

    print!("Paste bearer token from the page: ");
    io::stdout().flush().ok();
    let mut token = String::new();
    io::stdin().read_line(&mut token).context("reading token")?;
    let token = token.trim();
    if token.is_empty() {
        bail!("No token provided");
    }
    save_token(token)?;
    if let Some(path) = config_file_path() {
        println!("Token saved to {}", path.display());
    }
    Ok(())
}

fn build_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}
