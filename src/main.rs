use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use colored::Colorize;
use dirs::config_dir;
use open::that;
use reqwest::{Client, Url};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

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
    /// Login to get a token
    Login,
    /// Get lyrics for current track
    Lyrics {
        #[arg(long)]
        guild_id: Option<String>,
        #[arg(long)]
        user_id: Option<String>,
    },
}

#[derive(Deserialize, Clone)]
struct GiteaAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GiteaRelease {
    tag_name: String,
    assets: Vec<GiteaAsset>,
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

#[derive(Serialize)]
struct FilterPayload {
    action: &'static str,
    guild_id: Option<String>,
    user_id: Option<String>,
    filters: AudioFilters,
}

#[derive(Serialize)]
struct LyricsPayload {
    action: String,
    guild_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Serialize, Default)]
struct AudioFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    volume: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    equalizer: Option<Vec<EqualizerBand>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    karaoke: Option<KaraokeOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timescale: Option<TimescaleOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tremolo: Option<TremoloOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vibrato: Option<VibratoOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rotation: Option<RotationOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    distortion: Option<DistortionOptions>,
    #[serde(rename = "channelMix", skip_serializing_if = "Option::is_none")]
    channel_mix: Option<ChannelMixOptions>,
    #[serde(rename = "lowPass", skip_serializing_if = "Option::is_none")]
    low_pass: Option<LowPassOptions>,
}

#[derive(Serialize, Clone)]
struct EqualizerBand {
    band: i32,
    gain: f32,
}

#[derive(Serialize, Clone)]
struct KaraokeOptions {
    level: Option<f32>,
    #[serde(rename = "monoLevel")]
    mono_level: Option<f32>,
    #[serde(rename = "filterBand")]
    filter_band: Option<f32>,
    #[serde(rename = "filterWidth")]
    filter_width: Option<f32>,
}

#[derive(Serialize, Clone)]
struct TimescaleOptions {
    speed: Option<f32>,
    pitch: Option<f32>,
    rate: Option<f32>,
}

#[derive(Serialize, Clone)]
struct TremoloOptions {
    frequency: Option<f32>,
    depth: Option<f32>,
}

#[derive(Serialize, Clone)]
struct VibratoOptions {
    frequency: Option<f32>,
    depth: Option<f32>,
}

#[derive(Serialize, Clone)]
struct RotationOptions {
    #[serde(rename = "rotationHz")]
    rotation_hz: Option<f32>,
}

#[derive(Serialize, Clone)]
struct DistortionOptions {
    #[serde(rename = "sinOffset")]
    sin_offset: Option<f32>,
    #[serde(rename = "sinScale")]
    sin_scale: Option<f32>,
    #[serde(rename = "cosOffset")]
    cos_offset: Option<f32>,
    #[serde(rename = "cosScale")]
    cos_scale: Option<f32>,
    #[serde(rename = "tanOffset")]
    tan_offset: Option<f32>,
    #[serde(rename = "tanScale")]
    tan_scale: Option<f32>,
    offset: Option<f32>,
    scale: Option<f32>,
}

#[derive(Serialize, Clone)]
struct ChannelMixOptions {
    #[serde(rename = "leftToLeft")]
    left_to_left: Option<f32>,
    #[serde(rename = "leftToRight")]
    left_to_right: Option<f32>,
    #[serde(rename = "rightToLeft")]
    right_to_left: Option<f32>,
    #[serde(rename = "rightToRight")]
    right_to_right: Option<f32>,
}

#[derive(Serialize, Clone)]
struct LowPassOptions {
    smoothing: Option<f32>,
}

async fn check_for_updates(client: &Client) -> Option<(String, Vec<GiteaAsset>)> {
    let url = "https://git.xserv.pp.ua/api/v1/repos/xxanqw/jorik-cli/releases";
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
    let cli = Cli::parse();
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
            let payload = PlayPayload {
                action: "play",
                guild_id,
                channel_id,
                query: clean_query(&query.join(" ")),
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

fn clean_query(input: &str) -> String {
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
