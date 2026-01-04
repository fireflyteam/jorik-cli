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

/// CLI to interact with the Jorik webhook server.
#[derive(Parser, Debug)]
#[command(name = "jorik", author, version, about)]
struct Cli {
    /// Base URL of the webhook server
    #[arg(
        long,
        global = true,
        env = "JORIK_BASE_URL",
        default_value = "https://jorik.xserv.pp.ua"
    )]
    base_url: String,

    /// Bearer token for authorization
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
        /// Query/URL to play
        query: String,
        /// Guild ID (optional)
        #[arg(long)]
        guild_id: Option<String>,
        /// Voice channel ID (optional)
        #[arg(long)]
        channel_id: Option<String>,
        /// User ID (optional)
        #[arg(long)]
        user_id: Option<String>,
        /// Override display name
        #[arg(long)]
        requested_by: Option<String>,
        /// Avatar URL
        #[arg(long)]
        avatar_url: Option<String>,
    },
    /// Skip the current track
    Skip {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Stop playback and clear queue
    Stop {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Pause or resume playback
    Pause {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Show the current queue
    Queue {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
        #[arg(long, default_value = "10")]
        limit: usize,
        #[arg(long, default_value = "0")]
        offset: usize,
    },
    /// Clear the queue
    Clear {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Show currently playing track
    NowPlaying {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Set loop mode (off, track, queue)
    Loop {
        mode: String,
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Toggle 24/7 mode
    #[command(name = "247")]
    TwentyFourSeven {
        /// "on" or "off". If omitted, toggles.
        state: Option<String>,
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Shuffle the queue
    Shuffle {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Login to get a token
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
struct SimplePayload {
    action: &'static str,
    guild_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Serialize)]
struct QueuePayload {
    action: &'static str,
    guild_id: Option<String>,
    user_id: Option<String>,
    limit: usize,
    offset: usize,
}

#[derive(Serialize)]
struct LoopPayload {
    action: &'static str,
    guild_id: Option<String>,
    user_id: Option<String>,
    loop_mode: String,
}

#[derive(Serialize)]
struct TwentyFourSevenPayload {
    action: &'static str,
    guild_id: Option<String>,
    user_id: Option<String>,
    enabled: Option<bool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("jorik-cli/0.2.0")
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
            let payload = SimplePayload {
                action: "skip",
                guild_id,
                user_id,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Stop { guild_id, user_id } => {
            let payload = SimplePayload {
                action: "stop",
                guild_id,
                user_id,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Pause { guild_id, user_id } => {
            let payload = SimplePayload {
                action: "pause",
                guild_id,
                user_id,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Queue {
            guild_id,
            user_id,
            limit,
            offset,
        } => {
            let payload = QueuePayload {
                action: "queue",
                guild_id,
                user_id,
                limit,
                offset,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Clear { guild_id, user_id } => {
            let payload = SimplePayload {
                action: "clear",
                guild_id,
                user_id,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::NowPlaying { guild_id, user_id } => {
            let payload = SimplePayload {
                action: "nowplaying",
                guild_id,
                user_id,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Loop {
            mode,
            guild_id,
            user_id,
        } => {
            let payload = LoopPayload {
                action: "loop",
                guild_id,
                user_id,
                loop_mode: mode,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::TwentyFourSeven {
            state,
            guild_id,
            user_id,
        } => {
            let enabled = match state.as_deref() {
                Some("on") | Some("true") => Some(true),
                Some("off") | Some("false") => Some(false),
                _ => None,
            };
            let payload = TwentyFourSevenPayload {
                action: "247",
                guild_id,
                user_id,
                enabled,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Shuffle { guild_id, user_id } => {
            let payload = SimplePayload {
                action: "shuffle",
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

    if resp.status().is_success() {
        println!("{} Server is healthy", "✔".green());
    } else {
        println!("{} Server returned status {}", "✘".red(), resp.status());
    }
    Ok(())
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
        if let Some(summary) = summarize(&json) {
            println!("{}", summary);
        } else if !status.is_success() {
            // Fallback for errors that summarize didn't catch
            println!("{} Request failed ({})", "✘".red(), status);
            println!("{}", json);
        } else {
            // Fallback for success
            println!("{} Success", "✔".green());
            println!("{}", json);
        }
    } else if !status.is_success() {
        println!("{} Request failed ({})", "✘".red(), status);
        println!("{}", text);
    } else {
        println!("{} Success", "✔".green());
        println!("{}", text);
    }

    Ok(())
}

fn summarize(json: &serde_json::Value) -> Option<String> {
    let obj = json.as_object()?;

    // Handle Errors
    if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
        let msg = obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        let hint = if err == "unauthorized" {
            format!(
                "\n{}",
                "💡 Hint: Run `jorik login` or check your token.".yellow()
            )
        } else {
            String::new()
        };
        return Some(format!("{} {}{}", "✘".red(), msg, hint));
    }

    let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("");

    match action {
        "play" => {
            let tracks = obj.get("tracks").and_then(|v| v.as_array());
            let count = tracks.map(|t| t.len()).unwrap_or(0);
            let first = tracks.and_then(|t| t.first()).and_then(|v| v.as_object());
            let title = first
                .and_then(|o| o.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown Track");

            if count > 1 {
                Some(format!(
                    "{} Added {} tracks to queue (starting with {})",
                    "🎶".cyan(),
                    count,
                    title.bold()
                ))
            } else {
                Some(format!("{} Added {} to queue", "🎶".cyan(), title.bold()))
            }
        }
        "skip" => {
            if let Some(skipped) = obj.get("skipped").and_then(|v| v.as_object()) {
                let title = skipped
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Track");
                Some(format!("{} Skipped {}", "⏭️".magenta(), title.bold()))
            } else {
                Some(format!("{} Nothing to skip", "ℹ️".blue()))
            }
        }
        "stop" => Some(format!("{} Playback stopped and queue cleared", "⏹️".red())),
        "pause" => {
            let state = obj.get("state").and_then(|v| v.as_str()).unwrap_or("");
            match state {
                "paused" => Some(format!("{} Playback paused", "⏸️".yellow())),
                "resumed" => Some(format!("{} Playback resumed", "▶️".green())),
                _ => Some(format!("{} Toggled pause", "⏯️".yellow())),
            }
        }
        "queue" => {
            let current = obj.get("current").and_then(|v| v.as_object());
            let upcoming = obj.get("upcoming").and_then(|v| v.as_array());
            let total = obj
                .get("total_upcoming")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let mut output = String::new();
            output.push_str(&format!("{}\n", "Current Queue".bold().underline()));

            if let Some(curr) = current {
                let title = curr
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");
                output.push_str(&format!("{} {}\n", "▶️".green(), title.bold()));
            } else {
                output.push_str("Nothing playing currently.\n");
            }

            if let Some(list) = upcoming {
                if !list.is_empty() {
                    output.push_str("\nUp Next:\n");
                    for (i, item) in list.iter().enumerate() {
                        let title = item
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown");
                        output.push_str(&format!("{}. {}\n", i + 1, title));
                    }
                    if total > list.len() as u64 {
                        output.push_str(&format!("... and {} more\n", total - list.len() as u64));
                    }
                } else {
                    output.push_str("\nQueue is empty.\n");
                }
            }
            Some(output)
        }
        "clear" => {
            let removed = obj.get("removed").and_then(|v| v.as_u64()).unwrap_or(0);
            Some(format!(
                "{} Cleared {} tracks from queue",
                "🗑️".red(),
                removed
            ))
        }
        "nowplaying" => {
            if let Some(np) = obj.get("now_playing").and_then(|v| v.as_object()) {
                let track = np.get("track").and_then(|v| v.as_object());
                let title = track
                    .and_then(|t| t.get("title"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");
                let elapsed = np.get("elapsedMs").and_then(|v| v.as_u64()).unwrap_or(0);
                let duration = np.get("durationMs").and_then(|v| v.as_u64()).unwrap_or(0);

                let progress = if duration > 0 {
                    let pct = (elapsed as f64 / duration as f64 * 20.0).round() as usize;
                    let bar = "━".repeat(pct) + "⚪" + &"━".repeat(20usize.saturating_sub(pct));
                    format!("[{}]", bar)
                } else {
                    "".to_string()
                };

                let time_str = format!(
                    "{}:{:02} / {}:{:02}",
                    elapsed / 60000,
                    (elapsed % 60000) / 1000,
                    duration / 60000,
                    (duration % 60000) / 1000
                );

                Some(format!(
                    "{} {}\n{} {}",
                    "▶️".green(),
                    title.bold(),
                    progress,
                    time_str
                ))
            } else {
                Some(format!("{} Nothing is playing right now", "zzz".blue()))
            }
        }
        "loop" => {
            let mode = obj.get("mode").and_then(|v| v.as_str()).unwrap_or("off");
            Some(format!("{} Loop mode set to: {}", "🔁".cyan(), mode.bold()))
        }
        "247" => {
            let enabled = obj
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if enabled {
                Some(format!("{} 24/7 mode enabled", "🌙".yellow()))
            } else {
                Some(format!("{} 24/7 mode disabled", "☀️".yellow()))
            }
        }
        "shuffle" => Some(format!("{} Queue shuffled", "🔀".magenta())),
        _ => None,
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
    println!("{} Opening browser for authorization...", "🔑".yellow());
    println!("Link: {}", auth_url.underline());
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
        println!("{} Token saved to {}", "✔".green(), path.display());
    }
    Ok(())
}

fn build_url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}
