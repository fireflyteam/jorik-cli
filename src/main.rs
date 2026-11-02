use anyhow::{Context, Result, bail};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, parser::ValueSource};
use colored::Colorize;
use dirs::config_dir;
use reqwest::Client;
use serde_json::Value;
use std::io::{self, Write};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

mod api;
mod ascii;
mod auth;
mod credentials;
mod image;
mod output;
mod tui;
mod updater;

pub use updater::ReleaseAsset as GiteaAsset;

use api::*;

/// CLI to interact with the Jorik webhook server.
#[derive(Parser)]
#[command(name = "jorik CLI", author, version, about)]
struct Cli {
    /// Base URL of the webhook server
    #[arg(
        long,
        global = true,
        env = "JORIK_BASE_URL",
        default_value = api::DEFAULT_BASE_URL
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
        #[arg(num_args = 1..)]
        query: Vec<String>,
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
    /// Enqueue the "turip" track (Spotify link)
    Turip {
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
    /// Apply audio filters (clear, bassboost, nightcore, vaporwave, 8d, soft, tremolo, vibrato, karaoke)
    Filter {
        /// Filter style
        style: String,
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Account-related commands (login, signout, info)
    Auth {
        #[command(subcommand)]
        command: AuthSubcommand,
    },
    /// Get lyrics for current track
    Lyrics {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
    /// Launch the TUI interface
    Tui {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum AuthSubcommand {
    /// Login via browser and capture token, username and avatar
    Login,
    /// Sign out and remove the saved auth data from device
    Signout,
    /// Show current saved auth info
    Info,
    /// Bind old unscoped auth.json credentials to an explicitly confirmed server
    Migrate {
        #[arg(long)]
        confirm_server: String,
    },
    /// Permanently remove old unscoped plaintext auth files
    ClearLegacy {
        #[arg(long, required = true)]
        confirm: bool,
    },
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;

fn resolve_base_url(saved: &str, requested: &str, source: Option<ValueSource>) -> String {
    if source == Some(ValueSource::DefaultValue) {
        saved.to_string()
    } else {
        requested.to_string()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    {
        let args: Vec<_> = std::env::args_os().collect();
        let mut want_version = false;
        let mut want_protocols = false;
        for a in &args {
            if let Some(s) = a.to_str() {
                if s == "-V" || s == "--version" {
                    want_version = true;
                }
                if s == "-p" || s == "--protocols" {
                    want_protocols = true;
                }
                if s.starts_with('-') && !s.starts_with("--") {
                    let short = &s[1..];
                    if short.contains('V') {
                        want_version = true;
                    }
                    if short.contains('p') {
                        want_protocols = true;
                    }
                }
            }
        }
        if want_version {
            image::print_version_info(want_protocols);
            std::process::exit(0);
        }
    }

    let matches = Cli::command().get_matches();
    let mut cli = Cli::from_arg_matches(&matches)?;
    // Recovery commands must work even with invalid saved server settings.
    if let Commands::Auth { command } = &cli.command {
        match command {
            AuthSubcommand::Migrate { confirm_server } => {
                return migrate_auth(confirm_server).await;
            }
            AuthSubcommand::ClearLegacy { confirm } => {
                anyhow::ensure!(*confirm, "explicit confirmation is required");
                credentials::clear_legacy().await?;
                println!("Legacy plaintext credential files removed.");
                return Ok(());
            }
            _ => {}
        }
    }
    let mut settings = api::load_settings();
    cli.base_url = resolve_base_url(
        &settings.base_url,
        &cli.base_url,
        matches.value_source("base_url"),
    );
    // Canonicalize once; the same secure server identity is used by both interfaces.
    cli.base_url = credentials::server_identity(&cli.base_url)?;
    settings.base_url = cli.base_url.clone();

    let client = Client::builder()
        .user_agent("jorik-cli")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .context("building HTTP client")?;

    if credentials::legacy_present() {
        eprintln!(
            "Unscoped plaintext credentials remain from 0.4.x. Use `auth migrate --confirm-server URL` or `auth clear-legacy --confirm`; they are never reused automatically."
        );
    }
    // Authentication is handled before background update checks and credential
    // resolution, so login/signout remain usable when migration or lookup fails.
    if let Commands::Auth { command } = &cli.command {
        return match command {
            AuthSubcommand::Login => login(&cli.base_url).await,
            AuthSubcommand::Signout => signout(&client, &cli.base_url, cli.token.as_deref()).await,
            AuthSubcommand::Info => auth_info(&cli.base_url).await,
            AuthSubcommand::Migrate { .. } | AuthSubcommand::ClearLegacy { .. } => {
                unreachable!("recovery commands handled before settings resolution")
            }
        };
    }
    let saved = if cli.token.is_some() || matches!(cli.command, Commands::Health) {
        None
    } else {
        credentials::load(&cli.base_url).await?
    };
    let token = cli
        .token
        .clone()
        .or_else(|| saved.as_ref().map(|auth| auth.token.clone()));
    if let Some(token) = &token {
        credentials::validate_token(token)?;
    }

    if token.is_none() && !matches!(cli.command, Commands::Health | Commands::Tui { .. }) {
        bail!(
            "not authenticated for {}; run `auth login`, explicitly migrate legacy credentials, or supply JORIK_TOKEN",
            cli.base_url
        );
    }

    if let Commands::Tui { guild_id, user_id } = cli.command {
        if let Some((latest, _assets)) = tui::run(settings, token, guild_id, user_id).await? {
            return updater::install(&latest).await;
        }
        return Ok(());
    }

    let update_check = tokio::spawn(updater::check_for_updates());

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
            let avatar = avatar_url.or_else(|| saved.as_ref().and_then(|a| a.avatar_url.clone()));
            let requested_by =
                requested_by.or_else(|| saved.as_ref().and_then(|a| a.username.clone()));
            let payload = PlayPayload {
                action: "play",
                guild_id,
                channel_id,
                query: clean_query(&query.join(" ")),
                user_id,
                requested_by,
                avatar_url: avatar,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Turip {
            guild_id,
            channel_id,
            user_id,
            requested_by,
            avatar_url,
        } => {
            let avatar = avatar_url.or_else(|| saved.as_ref().and_then(|a| a.avatar_url.clone()));
            let requested_by =
                requested_by.or_else(|| saved.as_ref().and_then(|a| a.username.clone()));
            let payload = PlayPayload {
                action: "play",
                guild_id,
                channel_id,
                query: clean_query("https://open.spotify.com/track/2RQWB4Asy1rjZL4IUcJ7kn"),
                user_id,
                requested_by,
                avatar_url: avatar,
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
        Commands::Auth { .. } => unreachable!("auth commands handled before credential resolution"),
        Commands::Lyrics { guild_id, user_id } => {
            let payload = LyricsPayload {
                action: "lyrics".to_string(),
                guild_id,
                user_id,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Filter {
            style,
            guild_id,
            user_id,
        } => {
            let filters = match style.to_lowercase().as_str() {
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
                _ => {
                    eprintln!("Unknown filter style: {}", style);
                    return Ok(());
                }
            };

            let payload = FilterPayload {
                action: "filter",
                guild_id,
                user_id,
                filters,
            };
            post_audio(&client, &cli.base_url, token.as_deref(), &payload).await?;
        }
        Commands::Tui { .. } => unreachable!(), // Handled early
    }

    if let Ok(Some((latest, _assets))) = update_check.await {
        println!(
            "\n{} {} -> {}",
            "A new version of jorik-cli is available:".yellow().bold(),
            env!("CARGO_PKG_VERSION").red(),
            latest.green().bold()
        );

        print!("Do you want to update and install the latest version? [y/N]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if input.trim().eq_ignore_ascii_case("y") {
            updater::install(&latest).await?;
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

async fn post_audio<T: serde::Serialize>(
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
    print_response(resp, token).await
}

async fn print_response(resp: reqwest::Response, token: Option<&str>) -> Result<()> {
    let status = resp.status();
    let text = api::response_text(resp).await?;

    if let Ok(mut json) = serde_json::from_str::<Value>(&text) {
        output::scrub_json(&mut json, token);
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
        println!("{}", output::safe_text(&text, token));
    } else {
        println!("{} Success", "✔".green());
        println!("{}", output::safe_text(&text, token));
    }

    Ok(())
}

fn summarize(json: &Value) -> Option<String> {
    let obj = json.as_object()?;

    // Handle Errors
    if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
        let msg = obj
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        let hint = if err == "unauthorized" {
            // If a legacy token exists locally, show a specific hint asking the user to re-login.
            if config_dir()
                .map(|p| p.join("jorik-cli").join("token"))
                .map(|p| p.exists())
                .unwrap_or(false)
            {
                format!(
                    "\n{}",
                    "💡 Hint: Found a legacy token file — run `jorik auth login` to re-authenticate and save username/avatar.".yellow()
                )
            } else {
                format!(
                    "\n{}",
                    "💡 Hint: Run `jorik auth login` or check your token.".yellow()
                )
            }
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
            let artist = first.and_then(|o| o.get("author")).and_then(|v| v.as_str());

            let display_title = if let Some(a) = artist {
                format!("{} by {}", title, a)
            } else {
                title.to_string()
            };

            if count > 1 {
                Some(format!(
                    "{} Added {} tracks to queue (starting with {})",
                    "🎶".cyan(),
                    count,
                    display_title.bold()
                ))
            } else {
                Some(format!(
                    "{} Added {} to queue",
                    "🎶".cyan(),
                    display_title.bold()
                ))
            }
        }
        "skip" => {
            if let Some(skipped) = obj.get("skipped").and_then(|v| v.as_object()) {
                let title = skipped
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown Track");
                let artist = skipped.get("author").and_then(|v| v.as_str());
                let display_title = if let Some(a) = artist {
                    format!("{} by {}", title, a)
                } else {
                    title.to_string()
                };
                Some(format!(
                    "{} Skipped {}",
                    "⏭️".magenta(),
                    display_title.bold()
                ))
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
                let artist = curr.get("author").and_then(|v| v.as_str());
                let display_title = if let Some(a) = artist {
                    format!("{} by {}", title, a)
                } else {
                    title.to_string()
                };
                output.push_str(&format!("{} {}\n", "▶️".green(), display_title.bold()));
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
                        let artist = item.get("author").and_then(|v| v.as_str());
                        let display_title = if let Some(a) = artist {
                            format!("{} by {}", title, a)
                        } else {
                            title.to_string()
                        };
                        output.push_str(&format!("{}. {}\n", i + 1, display_title));
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
                let artist = track.and_then(|t| t.get("author")).and_then(|v| v.as_str());

                let display_title = if let Some(a) = artist {
                    format!("{} by {}", title, a)
                } else {
                    title.to_string()
                };

                let elapsed = np.get("elapsedMs").and_then(|v| v.as_u64()).unwrap_or(0);
                let duration = np.get("durationMs").and_then(|v| v.as_u64()).unwrap_or(0);

                let progress = if duration > 0 {
                    let pct = ((elapsed as f64 / duration as f64).clamp(0.0, 1.0) * 20.0).round()
                        as usize;
                    let bar = "━".repeat(pct) + "⚪" + &"━".repeat(20usize.saturating_sub(pct));
                    format!(
                        "[{}]
",
                        bar
                    )
                } else {
                    "\n".to_string()
                };

                let time_str = format!(
                    "{:02}:{:02} / {:02}:{:02}",
                    elapsed / 60000,
                    (elapsed % 60000) / 1000,
                    duration / 60000,
                    (duration % 60000) / 1000
                );

                Some(format!(
                    "{} {}\n{} {}",
                    "▶️".green(),
                    display_title.bold(),
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
        "filter" => {
            let msg = obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Filters updated");
            Some(format!("{} {}", "🎚️".cyan(), msg))
        }
        "lyrics" => {
            if let Some(data) = obj.get("data").and_then(|v| v.as_object()) {
                let mut output = String::new();
                output.push_str(&format!("{}\n\n", "🎤 Lyrics".magenta().bold()));

                if let Some(text) = data.get("text").and_then(|v| v.as_str()) {
                    output.push_str(text);
                } else if let Some(lines) = data.get("lines").and_then(|v| v.as_array()) {
                    for line in lines {
                        let timestamp = line.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
                        let text = line.get("line").and_then(|v| v.as_str()).unwrap_or("");
                        let ts_str = format!(
                            "[{:02}:{:02}]",
                            timestamp / 60000,
                            (timestamp % 60000) / 1000
                        );
                        output.push_str(&format!("{} {}\n", ts_str.dimmed(), text));
                    }
                }

                if let Some(source) = data.get("sourceName").and_then(|v| v.as_str()) {
                    output.push_str(&format!("\n\nSource: {}", source.dimmed()));
                }
                Some(output)
            } else {
                Some(format!("{} No lyrics data found", "ℹ️".blue()))
            }
        }
        _ => None,
    }
}

async fn login(base_url: &str) -> Result<()> {
    credentials::check_available(base_url).await?;
    let session = auth::LoginSession::bind(base_url).await?;
    println!("Open this link to log in:\n{}", session.authorization_url());
    let link = session.authorization_url().to_string();
    tokio::task::spawn_blocking(move || {
        if open::that(link).is_err() {
            eprintln!("Could not open a browser automatically; use the link above.");
        }
    });
    let cancel = CancellationToken::new();
    let work_cancel = cancel.clone();
    let work = async {
        let pending = session.wait(&work_cancel).await?;
        if let Err(error) =
            credentials::save(base_url, pending.auth.clone(), work_cancel.clone()).await
        {
            let _ = pending.respond(false).await;
            return Err(error);
        }
        let _ = pending.respond(true).await;
        println!("Logged in. Credentials saved in the OS credential store for {base_url}.");
        Ok(())
    };
    tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => {
            cancel.cancel();
            bail!("login cancelled");
        }
        result = work => result,
    }
}

async fn migrate_auth(confirmed_server: &str) -> Result<()> {
    let server = credentials::server_identity(confirmed_server)?;
    let cancel = CancellationToken::new();
    let work = credentials::migrate(&server, cancel.clone());
    let migrated = tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => { cancel.cancel(); bail!("migration cancelled"); }
        result = work => result?,
    };
    if migrated.is_some() {
        println!(
            "Legacy credentials migrated to the OS store for {server}; plaintext files removed."
        );
    } else {
        println!("No supported legacy auth.json found. Use `auth login` to authenticate.");
    }
    Ok(())
}

async fn auth_info(base_url: &str) -> Result<()> {
    if let Some(auth) = credentials::load(base_url).await? {
        println!(
            "Server: {base_url}\nStorage: OS credential store\nUser: {}\nToken: [redacted]",
            auth.username.as_deref().unwrap_or("Unknown")
        );
    } else {
        println!("Not authenticated for {base_url}. Run `jorik auth login`.");
    }
    Ok(())
}

async fn signout(client: &Client, base_url: &str, override_token: Option<&str>) -> Result<()> {
    let saved = if override_token.is_none() {
        match credentials::load(base_url).await {
            Ok(saved) => saved,
            Err(_) => {
                eprintln!(
                    "Could not read saved credentials; local removal will still be attempted."
                );
                None
            }
        }
    } else {
        None
    };
    let token = override_token.or_else(|| saved.as_ref().map(|auth| auth.token.as_str()));
    let removed = credentials::delete(base_url).await;
    match auth::revoke(client, base_url, token).await {
        Ok(true) => println!("Remote token revoked."),
        Ok(false) => eprintln!("Remote revocation was not confirmed."),
        Err(error) => eprintln!("{error}"),
    }
    removed?;
    println!("Signed out locally for {base_url}.");
    Ok(())
}
