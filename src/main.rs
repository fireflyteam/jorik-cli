use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use colored::Colorize;
use dirs::config_dir;
use open::that;
use reqwest::{Client, Url};
use semver::Version;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{self, Write};
use std::process::Command;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;

mod api;
mod ascii;
mod image;
mod tui;

use api::*;

/// CLI to interact with the Jorik webhook server.
#[derive(Parser, Debug)]
#[command(name = "jorik CLI", author, version, about)]
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
}

#[derive(serde::Deserialize)]
struct GiteaAsset {
    name: String,
    browser_download_url: String,
}

#[derive(serde::Deserialize)]
struct GiteaRelease {
    tag_name: String,
    assets: Vec<GiteaAsset>,
}

async fn check_for_updates(client: &Client) -> Option<(String, Vec<GiteaAsset>)> {
    let url = "https://api.github.com/repos/FENTTEAM/jorik-cli/releases";
    let res = client
        .get(url)
        .header("User-Agent", "jorik-cli")
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        return None;
    }

    let releases: Vec<GiteaRelease> = res.json().await.ok()?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).ok()?;

    let mut latest_version = current.clone();
    let mut update_found = false;
    let mut latest_release_info = None;

    // Filter to find the absolute latest version (including prereleases if they are newer)
    for release in releases {
        let clean_name = release.tag_name.trim_start_matches('v');
        if let Ok(version) = Version::parse(clean_name) {
            if version > latest_version {
                latest_version = version;
                latest_release_info = Some((release.tag_name, release.assets));
                update_found = true;
            }
        }
    }

    if update_found {
        latest_release_info
    } else {
        None
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // If user requested --version/-V, print enhanced version info and exit early.
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

    let cli = Cli::parse();
    
    // Check if we are running TUI first, to avoid printing update checks to stdout
    if let Commands::Tui { guild_id, user_id } = cli.command {
        return tui::run(
            cli.base_url,
            cli.token.or_else(load_token),
            guild_id,
            user_id
        ).await;
    }

    let client = Client::builder()
        .user_agent("jorik-cli")
        .timeout(Duration::from_secs(10))
        .build()
        .context("building HTTP client")?;

    let update_client = client.clone();
    let update_check = tokio::spawn(async move { check_for_updates(&update_client).await });

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
            let saved = load_auth();
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
            let saved = load_auth();
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
        Commands::Auth { command } => match command {
            AuthSubcommand::Login => {
                login(&cli.base_url).await?;
            }
            AuthSubcommand::Signout => {
                signout(&client, &cli.base_url, token.as_deref()).await?;
            }
            AuthSubcommand::Info => {
                auth_info()?;
            }
        },
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

    if let Ok(Some((latest, assets))) = update_check.await {
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
            if cfg!(target_os = "linux") {
                println!("Running update script...");
                let status = Command::new("sh")
                    .arg("-c")
                    .arg("curl -sL https://shorty.pp.ua/jorikcli | bash")
                    .status()
                    .context("Failed to execute update script")?;

                if status.success() {
                    println!(
                        "\n{}",
                        "Update successful! You can now use the latest version."
                            .green()
                            .bold()
                    );
                } else {
                    println!("\n{}", "Update failed.".red().bold());
                }
            } else if cfg!(target_os = "windows") {
                if let Some(asset) = assets.iter().find(|a| a.name.ends_with("setup.exe")) {
                    println!("Downloading installer...");
                    let temp_dir = std::env::temp_dir();
                    let installer_path = temp_dir.join(&asset.name);

                    {
                        let mut file = File::create(&installer_path)?;
                        let mut response = client.get(&asset.browser_download_url).send().await?;

                        if !response.status().is_success() {
                            bail!("Failed to download installer: {}", response.status());
                        }

                        while let Some(chunk) = response.chunk().await? {
                            file.write_all(&chunk)?;
                        }
                    }

                    println!("Running installer...");
                    Command::new(&installer_path)
                        .arg("/SILENT")
                        .spawn()
                        .context("Failed to start installer")?;

                    println!(
                        "\n{}",
                        "Update started! The application will now exit to complete the installation."
                            .green()
                            .bold()
                    );
                    std::process::exit(0);
                } else {
                    println!("{}", "No Windows installer found for this release.".red());
                    println!(
                        "Download it manually at: https://git.xserv.pp.ua/xxanqw/jorik-cli/releases"
                    );
                }
            } else {
                println!("Automatic updates are not supported on this platform.");
                println!("Download it at: https://git.xserv.pp.ua/xxanqw/jorik-cli/releases");
            }
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
    print_response(resp).await
}

async fn print_response(resp: reqwest::Response) -> Result<()> {
    let status = resp.status();
    let text = resp.text().await.context("reading response body")?;

    if let Ok(json) = serde_json::from_str::<Value>(&text) {
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
                    let pct = (elapsed as f64 / duration as f64 * 20.0).round() as usize;
                    let bar = "━".repeat(pct) + "⚪" + &"━".repeat(20usize.saturating_sub(pct));
                    format!("[{}]
", bar)
                } else {
                    "\n".to_string()
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

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

async fn login(base_url: &str) -> Result<()> {
    // Start a local listener so we can receive the issued bearer token
    // via a callback redirect from the webhook server. If no callback is
    // received within the timeout, fall back to the manual paste flow.
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("binding local listener; the legacy manual token-paste flow is deprecated. Please run `jorik auth login` on a device where your browser can redirect to http://127.0.0.1 so the CLI can automatically capture token, avatar and username")?;
    let local_addr = listener
        .local_addr()?;
    let callback_url = format!("http://{}/oauth-callback", local_addr);
    println!(
        "{} Local callback URL: {}",
        "📬".yellow(),
        callback_url.as_str().underline()
    );

    // Build authorize URL with callback parameter (the webhook server will
    // embed this callback into the OAuth `state` so it can redirect back).
    let mut auth_url =
        Url::parse(&build_url(base_url, "/authorize")).context("parsing authorize URL")?;
    auth_url
        .query_pairs_mut()
        .append_pair("callback", &callback_url);

    println!("{} Opening browser for authorization...", "🔑".yellow());
    println!("Link: {}", auth_url.as_str().underline());
    let _ = that(auth_url.as_str());

    // Wait for a single incoming connection (with timeout).
    match timeout(Duration::from_secs(120), listener.accept()).await {
        Ok(Ok((mut stream, _addr))) => {
            // Read the request (headers should fit into this buffer for our simple case).
            let mut buf = vec![0u8; 8192];
            let n = stream
                .read(&mut buf)
                .await?;
            let req = String::from_utf8_lossy(&buf[..n]);
            let first_line = req.lines().next().unwrap_or("");
            let path = first_line.split_whitespace().nth(1).unwrap_or("");
            // Prepend a scheme+host so `Url::parse` can parse query params.
            if let Ok(parsed) = Url::parse(&format!("http://localhost{}", path)) {
                let token_pair = parsed.query_pairs().find(|(k, _)| k == "token");
                let avatar_pair = parsed.query_pairs().find(|(k, _)| k == "avatar");
                let username_pair = parsed.query_pairs().find(|(k, _)| k == "username");
                if let Some((_k, v)) = token_pair {
                    let token = v.into_owned();
                    let token_trim = token.trim();
                    if token_trim.is_empty() {
                        let body = "Missing token";
                        let resp = format!(
                            "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        stream.write_all(resp.as_bytes()).await.ok();
                        bail!("No token provided");
                    }

                    let avatar_val = avatar_pair.map(|(_, val)| val.into_owned());
                    let username_val = username_pair.map(|(_, val)| val.into_owned());
                    save_token(token_trim, avatar_val.as_deref(), username_val.as_deref())?;

                    // Build a small, readable success page and kick off confetti animation.
                    let escaped_username = username_val
                        .as_deref()
                        .map(|s| escape_html(s))
                        .unwrap_or_else(|| "User".to_string());
                    let escaped_avatar = avatar_val.as_deref().map(|s| escape_html(s));
                    let saved_path_html = if let Some(path) = config_file_path() {
                        format!(
                            "<p>Saved to <code>{}</code></p>",
                            escape_html(&path.display().to_string())
                        )
                    } else {
                        "".to_string()
                    };

                    let mut body = String::new();
                    body.push_str(
                        r##"<!doctype html><html><head><meta charset="utf-8"/><meta name="viewport" content="width=device-width,initial-scale=1"/><title>Authorization complete</title><style>"##,
                    );
                    body.push_str(r##"body{font-family:-apple-system,BlinkMacSystemFont,\"Segoe UI\",Roboto,\"Helvetica Neue\",Arial, sans-serif;background:#2f3136;color:#dcddde;margin:0;padding:0;display:flex;align-items:center;justify-content:center;height:100vh}"##);
                    body.push_str(r##".container{max-width:560px;width:100%;padding:28px;background:#36393f;border-radius:12px;box-shadow:0 6px 20px rgba(0,0,0,0.6)}"##);
                    body.push_str(
                        r##".header{display:flex;align-items:center;gap:16px;margin-bottom:18px}"##,
                    );
                    body.push_str(r##".badge{width:56px;height:56px;display:flex;align-items:center;justify-content:center;border-radius:50%;background:#2f3136}"##);
                    body.push_str(r##".check{width:34px;height:34px;border-radius:50%;background:#43b581;color:#fff;display:flex;align-items:center;justify-content:center;font-weight:700;font-size:16px}"##);
                    body.push_str(r##".avatar{width:56px;height:56px;border-radius:50%;object-fit:cover;border:2px solid rgba(0,0,0,0.4)}"##);
                    body.push_str(r##".user{font-size:16px;font-weight:600;margin:0;color:#fff}"##);
                    body.push_str(r##".sp{color:#b9bbbe;font-size:13px;margin-top:4px}"##);
                    body.push_str(r##".path{display:inline-block;background:#2f3136;padding:6px 8px;border-radius:6px;color:#b9bbbe;font-family:monospace;margin-top:8px}"##);
                    body.push_str(
                        r##"</style></head><body><div class=\"container\"><div class=\"header\">"##,
                    );
                    if let Some(avatar) = &escaped_avatar {
                        body.push_str(&format!(
                            r##"<img class=\"avatar\" src=\"{}\" alt=\"avatar"##,
                            avatar
                        ));
                    } else {
                        body.push_str(r##"<div class=\"badge\"><div class=\"check\">✓</div></div>"##);
                    }
                    body.push_str(&format!(
                        r##"<div><div class=\"user\">{}</div><div class=\"sp\">Authorization complete</div>{}"##,
                        escaped_username,
                        saved_path_html
                    ));
                    body.push_str(r##"</div><div><p class=\"sp\">Token saved to your config. You may close this window.</p></div>"##);

                    // confetti
                    body.push_str(r##"<script src=\"https://cdn.jsdelivr.net/npm/canvas-confetti@1.6.0/dist/confetti.browser.min.js\"></script>"##);
                    body.push_str(
                        r##"<script>
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
</script>"##,
                    );
                    body.push_str("</div></body></html>");

                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n{}",
                        body
                    );
                    stream.write_all(resp.as_bytes()).await.ok();
                    stream.shutdown().await.ok();

                    if let Some(path) = config_file_path() {
                        println!("{} Token saved to {}", "✔".green(), path.display());
                    }
                    return Ok(())
                }
            }

            // If we reached here, callback didn't include a token. Respond with 400 and return OK.
            let body = "No token in callback";
            let resp = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.ok();
            Ok(())
        }
        _ => {
            bail!(
                "No callback received within timeout (120s). The legacy manual token-paste flow is deprecated. Please run `jorik auth login` and complete the authorization in your browser so the CLI can automatically capture token, avatar and username."
            );
        }
    }
}

fn auth_info() -> Result<()> {
    if let Some(auth) = load_auth() {
        if let Some(path) = config_file_path() {
            println!("{} Auth file: {}", "ℹ️".blue(), path.display());
        }
        println!(
            "{} User: {}",
            "👤".cyan(),
            auth.username
                .clone()
                .unwrap_or_else(|| "Unknown".to_string())
        );
        if let Some(avatar) = auth.avatar_url {
            println!("{} Avatar: {}", "🖼️".cyan(), avatar);
        } else {
            println!("{} Avatar: (none)", "🖼️".cyan());
        }

        let token = auth.token;
        let masked = if token.len() > 8 {
            format!("{}...{}", &token[0..4], &token[token.len() - 4..])
        } else {
            token
        };
        println!("{} Token: {}", "🔑".cyan(), masked);
        Ok(())
    } else {
        println!(
            "{} Not authenticated. Run `jorik auth login` to authenticate.",
            "ℹ️".blue()
        );
        Ok(())
    }
}

async fn signout(client: &Client, base_url: &str, token: Option<&str>) -> Result<()> {
    // If token present, attempt to revoke it on the server first.
    if let Some(tok) = token {
        println!("{} Revoking token on server...", "🔒".yellow());
        let url = build_url(base_url, "/webhook/auth/revoke");
        match client.post(&url).bearer_auth(tok).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    match resp.json::<serde_json::Value>().await {
                        Ok(json) => {
                            let revoked = json
                                .get("revoked")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if revoked {
                                println!("{} Server revoked token", "✔".green());
                            } else {
                                println!("{} Server did not revoke token", "ℹ️".blue());
                            }
                        }
                        Err(e) => {
                            println!("{} Failed to parse server response: {}", "✘".red(), e);
                        }
                    }
                } else {
                    println!("{} Server returned status {}", "✘".red(), resp.status());
                }
            }
            Err(e) => {
                println!(
                    "{} Failed to contact server to revoke token: {}",
                    "✘".red(),
                    e
                );
            }
        }
    } else {
        println!("{} No token present; skipping server revoke", "ℹ️".blue());
    }

    // Remove local auth file regardless of remote result
    let path = config_file_path().context("cannot determine config path")?;
    if path.exists() {
        fs::remove_file(&path).context("removing auth file")?;
        println!("{} Signed out and removed {}", "✔".green(), path.display());
    } else {
        println!("{} No auth found", "ℹ️".blue());
    }
    Ok(())
}
