//! Offline design study. Every track, state and action is simulated.
//! cargo run --locked --example tui_study
use std::{io, time::Duration};

use clap::{Parser, ValueEnum};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, LineGauge, List, ListItem, ListState, Paragraph, Wrap},
};
use ratatui_image::{
    Resize, StatefulImage,
    picker::Picker,
    protocol::{StatefulProtocol, StatefulProtocolType},
};

const BG: Color = Color::Rgb(44, 50, 72);
const CHROME: Color = Color::Rgb(22, 27, 40);
const SIDEBAR: Color = Color::Rgb(28, 34, 50);
const SESSION: Color = Color::Rgb(35, 43, 61);
const TRACKLIST: Color = Color::Rgb(21, 27, 41);
const ROW_ALT: Color = Color::Rgb(32, 41, 59);
const TABLE_HEADER: Color = Color::Rgb(48, 60, 86);
const FG: Color = Color::Rgb(236, 238, 245);
const MUTED: Color = Color::Rgb(183, 193, 212);
const ACCENT: Color = Color::Rgb(200, 181, 255);
const ARTIST: Color = Color::Rgb(168, 218, 204);
const ALBUM_TEXT: Color = Color::Rgb(201, 194, 226);
const LENGTH_TEXT: Color = Color::Rgb(228, 203, 161);
const HEADER_TEXT: Color = Color::Rgb(219, 228, 250);
const SELECT: Color = Color::Rgb(70, 57, 102);
const RULE: Color = Color::Rgb(80, 94, 125);
const WARN: Color = Color::Rgb(240, 197, 125);
const ALBUM: &str = "Hurry Up, We're Dreaming";
const TRACK_LENGTH: &str = "4:03";
const COVER_BYTES: &[u8] =
    include_bytes!("../docs/assets/tui-study/m83-hurry-up-were-dreaming.jpg");

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum Design {
    Queue,
    #[default]
    Desk,
    Monitor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum Scenario {
    #[default]
    Playing,
    Paused,
    Stale,
    Empty,
    Error,
    Pending,
    Add,
    Details,
    Clear,
    Help,
}

#[derive(Parser)]
#[command(about = "Offline Jorik design study; no account, audio or network access")]
struct Args {
    #[arg(long, value_enum, default_value = "desk")]
    design: Design,
    #[arg(long, value_enum, default_value = "playing")]
    scenario: Scenario,
    /// Render a deterministic frame and exit instead of starting the TUI.
    #[arg(long)]
    render: bool,
    #[arg(long, default_value_t = 80, value_parser = clap::value_parser!(u16).range(24..=240))]
    width: u16,
    #[arg(long, default_value_t = 24, value_parser = clap::value_parser!(u16).range(8..=80))]
    height: u16,
    /// Export styled cells for the preview renderer.
    #[arg(long, requires = "render")]
    json: bool,
}

struct Study {
    design: Design,
    scenario: Scenario,
    selected: usize,
    offset: usize,
    page: usize,
    draft: String,
    message: String,
    tracks: Vec<Track>,
    cover: StatefulProtocol,
    cover_font_size: (u16, u16),
    cover_area: Option<Rect>,
}

struct Track {
    title: String,
    artist: String,
    album: String,
    length: String,
}

impl Study {
    fn new(args: &Args) -> Self {
        let samples = [
            ("Intro", "The xx", "xx", "2:07"),
            ("Nightcall", "Kavinsky", "OutRun", "4:18"),
            ("A Real Hero", "College", "Drive", "4:28"),
            ("Teardrop", "Massive Attack", "Mezzanine", "5:30"),
            ("Roads", "Portishead", "Dummy", "5:02"),
            ("All I Need", "Air", "Moon Safari", "4:28"),
            ("Open Eye Signal", "Jon Hopkins", "Immunity", "7:51"),
            ("Archangel", "Burial", "Untrue", "4:00"),
            (
                "Two Thousand and Seventeen",
                "Four Tet",
                "New Energy",
                "4:12",
            ),
            ("Kerala", "Bonobo", "Migration", "3:57"),
            ("Gosh", "Jamie xx", "In Colour", "4:51"),
            ("Retrograde", "James Blake", "Overgrown", "3:43"),
            (
                "Someone Great",
                "LCD Soundsystem",
                "Sound of Silver",
                "6:25",
            ),
            (
                "Everything In Its Right Place",
                "Radiohead",
                "Kid A",
                "4:11",
            ),
            ("Says", "Nils Frahm", "Spaces", "8:18"),
            ("A Walk", "Tycho", "Dive", "5:16"),
            ("Odessa", "Caribou", "Swim", "5:15"),
            (
                "Your Hand In Mine",
                "Explosions In The Sky",
                "The Earth Is Not a Cold Dead Place",
                "8:17",
            ),
            ("Hoppipolla", "Sigur Ros", "Takk...", "4:28"),
            ("Outro", "M83", ALBUM, "4:07"),
        ];
        let mut picker = Picker::halfblocks();
        // Opaque padding covers every image cell, including the final partial row.
        picker.set_background_color(cover_background());
        Self {
            design: args.design,
            scenario: args.scenario,
            selected: 1,
            offset: 0,
            page: 1,
            draft: String::new(),
            message: String::new(),
            tracks: samples
                .into_iter()
                .map(|(title, artist, album, length)| Track {
                    title: title.into(),
                    artist: artist.into(),
                    album: album.into(),
                    length: length.into(),
                })
                .collect(),
            cover: picker.new_resize_protocol(cover_image()),
            cover_font_size: (10, 20),
            cover_area: None,
        }
    }

    fn move_by(&mut self, amount: isize) {
        if !self.tracks.is_empty() {
            self.selected = self
                .selected
                .saturating_add_signed(amount)
                .min(self.tracks.len() - 1);
        }
    }
}

fn cover_image() -> image::DynamicImage {
    image::load_from_memory(COVER_BYTES).expect("the bundled album cover must decode")
}

fn cover_background() -> image::Rgba<u8> {
    let [r, g, b] = rgb(SIDEBAR);
    image::Rgba([r, g, b, 255])
}

fn desk_visible(area: Rect) -> bool {
    area.width >= 78 && area.height >= 18
}

fn text(f: &mut Frame, area: Rect, value: impl Into<String>, color: Color) {
    f.render_widget(Paragraph::new(value.into()).fg(color), area);
}

fn row(area: Rect, y: u16) -> Rect {
    Rect::new(
        area.x,
        area.y + y.min(area.height),
        area.width,
        u16::from(y < area.height),
    )
}

fn inset(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    )
}

fn padded(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    area.inner(ratatui::layout::Margin::new(horizontal, vertical))
}

fn heading(f: &mut Frame, area: Rect, design: Design) {
    let selected = match design {
        Design::Queue => 0,
        Design::Desk => 1,
        Design::Monitor => 2,
    };
    let mut tabs = ["1 Queue", "2 Desk", "3 Monitor"]
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            Span::styled(
                format!(" {label} "),
                if i == selected {
                    Style::default().fg(ACCENT).bg(SELECT).bold()
                } else {
                    Style::default().fg(MUTED)
                },
            )
        })
        .collect::<Vec<_>>();
    tabs.push(Span::styled("  OFFLINE STUDY", Style::default().fg(MUTED)));
    f.render_widget(Paragraph::new(Line::from(tabs)), area);
}

fn playback(f: &mut Frame, area: Rect, study: &Study) {
    f.render_widget(Block::default().bg(SIDEBAR), area);
    let area = padded(
        area,
        if area.width >= 36 { 2 } else { 1 },
        u16::from(area.height >= 8),
    );
    let empty = study.scenario == Scenario::Empty;
    let stale = study.scenario == Scenario::Stale;
    let state = if empty {
        "IDLE"
    } else if stale {
        "LAST KNOWN"
    } else if study.scenario == Scenario::Paused {
        "PAUSED"
    } else {
        "PLAYING"
    };
    let desk = matches!(study.design, Design::Desk) && area.width <= 34 && area.height >= 6;
    if desk {
        text(f, row(area, 0), state, if stale { WARN } else { ACCENT });
        f.render_widget(
            Paragraph::new(if empty {
                "No track playing"
            } else {
                "Midnight City"
            })
            .bold(),
            row(area, 1),
        );
        text(
            f,
            row(area, 2),
            if empty { "Press a to add music" } else { "M83" },
            MUTED,
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Album  ", Style::default().fg(MUTED)),
                Span::styled(
                    clip(
                        if empty { "Unavailable" } else { ALBUM },
                        area.width.saturating_sub(7),
                    ),
                    Style::default().fg(FG),
                ),
            ])),
            row(area, 4),
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{state}  "),
                    Style::default().fg(if stale { WARN } else { ACCENT }),
                ),
                Span::styled(
                    if empty {
                        "No track playing"
                    } else {
                        "Midnight City"
                    },
                    Style::default().fg(FG).bold(),
                ),
            ])),
            row(area, 0),
        );
        text(
            f,
            row(area, 1),
            if empty {
                "Join a Discord voice channel, then press a to add music."
            } else {
                "M83"
            },
            MUTED,
        );
    }
    if area.height > 2 {
        let timing = Layout::horizontal([
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Length(6),
        ])
        .split(row(area, if desk { 5 } else { 2 }));
        text(
            f,
            timing[0],
            if empty || stale { "--:--" } else { "2:04" },
            MUTED,
        );
        f.render_widget(
            Paragraph::new(if empty { "--:--" } else { TRACK_LENGTH })
                .alignment(ratatui::layout::Alignment::Right)
                .fg(MUTED),
            timing[2],
        );
        f.render_widget(
            LineGauge::default()
                .ratio(if empty || stale { 0.0 } else { 124.0 / 243.0 })
                .label("")
                .filled_style(Style::default().fg(ACCENT))
                .unfilled_style(Style::default().fg(RULE)),
            timing[1],
        );
    }
    if !desk && area.height > 3 {
        f.render_widget(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(RULE)),
            row(area, 3),
        );
    }
}

fn album_cover(f: &mut Frame, area: Rect, study: &mut Study) {
    if area.height < 3 || study.scenario == Scenario::Empty {
        return;
    }
    // Image size follows terminal cell geometry, keeping the artwork proportional.
    let (font_width, font_height) = study.cover_font_size;
    let width = (u32::from(area.height) * u32::from(font_height) * 700
        / (u32::from(font_width.max(1)) * 718))
        .min(u32::from(area.width)) as u16;
    let bounds = Rect::new(area.x, area.y, width.max(1), area.height);
    let resize = Resize::Fit(Some(image::imageops::FilterType::Lanczos3));
    let fitted = study.cover.size_for(resize.clone(), bounds);
    let artwork = Rect::new(
        area.x,
        area.bottom().saturating_sub(fitted.height),
        fitted.width,
        fitted.height,
    );
    // Graphics protocols can paint above later text; hide artwork under modals.
    if matches!(
        study.scenario,
        Scenario::Add | Scenario::Clear | Scenario::Details | Scenario::Help
    ) {
        return;
    }
    f.render_stateful_widget(
        StatefulImage::default().resize(resize),
        artwork,
        &mut study.cover,
    );
    if matches!(study.cover.protocol_type(), StatefulProtocolType::Kitty(_)) {
        // ratatui-image 10 leaves Kitty's virtual grid dimensions implicit.
        // Match it to the placeholder grid, including the final padded cell row.
        let cell = &mut f.buffer_mut()[(artwork.x, artwork.y)];
        let symbol = cell.symbol().replacen(
            "a=T,U=1,",
            &format!("a=T,U=1,c={},r={},", artwork.width, artwork.height),
            1,
        );
        cell.set_symbol(&symbol);
    }
    study.cover_area = Some(artwork);
}

fn queue(f: &mut Frame, area: Rect, study: &mut Study) {
    f.render_widget(Block::default().bg(TRACKLIST), area);
    let area = padded(
        area,
        if area.width >= 64 { 2 } else { 1 },
        u16::from(area.height >= 8),
    );
    if area.height < 3 {
        return;
    }
    let count = if study.scenario == Scenario::Empty {
        0
    } else {
        study.tracks.len()
    };
    let title = if matches!(study.scenario, Scenario::Stale | Scenario::Error) {
        format!("Up next   {count} loaded   Cached")
    } else {
        format!("Up next   {count} loaded")
    };
    text(f, row(area, 0), title, ACCENT);
    let compact = area.width < 64;
    f.render_widget(
        Block::default().bg(TABLE_HEADER),
        Rect::new(area.x, area.y + 1, area.width, if compact { 2 } else { 1 }),
    );
    let columns = if compact {
        Layout::horizontal([
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Length(6),
        ])
        .split(row(area, 1))
    } else {
        Layout::horizontal([
            Constraint::Length(6),
            Constraint::Fill(4),
            Constraint::Fill(3),
            Constraint::Fill(3),
            Constraint::Length(6),
        ])
        .split(row(area, 1))
    };
    let secondary = Layout::horizontal([
        Constraint::Length(6),
        Constraint::Fill(1),
        Constraint::Fill(1),
    ])
    .split(row(area, 2));
    text(f, columns[0], "   #", HEADER_TEXT);
    text(f, columns[1], "TITLE", HEADER_TEXT);
    if compact {
        text(f, columns[2], "LENGTH", HEADER_TEXT);
        text(f, secondary[1], "ARTIST", HEADER_TEXT);
        text(f, secondary[2], "ALBUM", HEADER_TEXT);
    } else {
        text(f, columns[2], "ARTIST", HEADER_TEXT);
        text(f, columns[3], "ALBUM", HEADER_TEXT);
        text(f, columns[4], "LENGTH", HEADER_TEXT);
    }
    let header_rows = if compact { 3 } else { 2 };
    let item_height = if compact { 2 } else { 1 };
    let height = area.height.saturating_sub(header_rows) / item_height * item_height;
    let body = Rect::new(area.x, area.y + header_rows, area.width, height);
    study.page = usize::from(body.height / item_height).max(1);
    if count == 0 {
        text(
            f,
            row(body, 0),
            "Queue empty. Press a to add a song name or URL.",
            FG,
        );
        return;
    }
    // Widths include the two-cell selection marker; all metadata clips by cells.
    let items: Vec<_> = study
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let mut first = vec![
                Span::styled(format!("{:>2}  ", i + 1), Style::default().fg(MUTED)),
                queue_column(&track.title, columns[1].width, FG),
            ];
            let style = Style::default().bg(if i % 2 == 0 { TRACKLIST } else { ROW_ALT });
            if compact {
                first.push(queue_column(&track.length, columns[2].width, LENGTH_TEXT));
                ListItem::new(vec![
                    Line::from(first),
                    Line::from(vec![
                        Span::raw("    "),
                        queue_column(&track.artist, secondary[1].width, ARTIST),
                        queue_column(&track.album, secondary[2].width, ALBUM_TEXT),
                    ]),
                ])
                .style(style)
            } else {
                first.extend([
                    queue_column(&track.artist, columns[2].width, ARTIST),
                    queue_column(&track.album, columns[3].width, ALBUM_TEXT),
                    queue_column(&track.length, columns[4].width, LENGTH_TEXT),
                ]);
                ListItem::new(Line::from(first)).style(style)
            }
        })
        .collect();
    let mut state = ListState::default()
        .with_selected(Some(study.selected))
        .with_offset(study.offset);
    f.render_stateful_widget(
        List::new(items).highlight_symbol("> ").highlight_style(
            Style::default()
                .fg(ACCENT)
                .bg(SELECT)
                .add_modifier(Modifier::BOLD),
        ),
        body,
        &mut state,
    );
    study.offset = state.offset();
}

fn queue_column(value: &str, width: u16, color: Color) -> Span<'static> {
    let value = clip(value, width.saturating_sub(2));
    let padding = usize::from(width).saturating_sub(Line::from(value.as_str()).width());
    Span::styled(
        format!("{value}{}", " ".repeat(padding)),
        Style::default().fg(color),
    )
}

fn clip(value: &str, width: u16) -> String {
    let mut out = String::new();
    for c in value.chars() {
        out.push(c);
        if Line::from(out.as_str()).width() > usize::from(width) {
            out.pop();
            while !out.is_empty() && Line::from(out.as_str()).width() >= usize::from(width) {
                out.pop();
            }
            if width > 0 {
                out.push('…');
            }
            break;
        }
    }
    out
}

fn inspector(f: &mut Frame, area: Rect, study: &Study) {
    f.render_widget(Block::default().bg(SESSION), area);
    let area = padded(area, 2, 1);
    let Some(track) = study.tracks.get(study.selected) else {
        return;
    };
    let lines = vec![
        Line::from("Selected track").fg(ACCENT),
        Line::from(""),
        Line::from(track.title.as_str()).bold(),
        Line::from(track.artist.as_str()).fg(MUTED),
        Line::from(track.album.as_str()).fg(MUTED),
        Line::from(track.length.as_str()).fg(MUTED),
        Line::from(""),
        Line::from(format!("Position {} in loaded queue", study.selected + 1)).fg(MUTED),
        Line::from(""),
        Line::from("Esc Back to queue").fg(MUTED),
    ];
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn content(f: &mut Frame, area: Rect, study: &mut Study) {
    match study.design {
        Design::Queue => {
            let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(area);
            playback(f, rows[0], study);
            if study.scenario == Scenario::Details && area.width >= 112 {
                let cols = Layout::horizontal([Constraint::Min(70), Constraint::Length(36)])
                    .split(rows[1]);
                queue(f, cols[0], study);
                inspector(f, inset(cols[1]), study);
            } else {
                queue(f, rows[1], study);
            }
        }
        Design::Desk if desk_visible(area) => {
            let spacious = area.height >= 24;
            let cols = Layout::horizontal([
                Constraint::Length(if area.width >= 110 { 38 } else { 34 }),
                Constraint::Length(4),
                Constraint::Min(0),
            ])
            .split(area);
            f.render_widget(Block::default().bg(SIDEBAR), cols[0]);
            let playback_height = if spacious { 9 } else { 6 };
            let session_height = if spacious { 9 } else { 4 };
            let (font_width, font_height) = study.cover_font_size;
            let full_width_cover_height = (u32::from(cols[0].width) * u32::from(font_width) * 718)
                .div_ceil(u32::from(font_height.max(1)) * 700)
                as u16;
            let cover_height = cols[0]
                .height
                .saturating_sub(playback_height + session_height + 2)
                .min(full_width_cover_height);
            let left = Layout::vertical([
                Constraint::Length(playback_height),
                Constraint::Length(1),
                Constraint::Length(session_height),
                Constraint::Min(1),
                Constraint::Length(cover_height),
            ])
            .split(cols[0]);
            f.render_widget(Block::default().bg(SESSION), left[2]);
            playback(f, left[0], study);
            let fields = [
                ("Server", "example.com"),
                ("Guild", "123456789012345678"),
                (
                    "Transport",
                    if study.scenario == Scenario::Stale {
                        "HTTP"
                    } else {
                        "WebSocket"
                    },
                ),
                (
                    "Updated",
                    if study.scenario == Scenario::Stale {
                        "12s ago"
                    } else {
                        "Just now"
                    },
                ),
                ("Repeat", "Off"),
            ];
            let mut lines = vec![Line::from("Session").fg(ACCENT)];
            if spacious {
                lines.push(Line::from(""));
            }
            lines.extend(
                fields
                    .into_iter()
                    .filter(|(label, _)| spacious || !matches!(*label, "Transport" | "Updated"))
                    .map(|(label, value)| {
                        Line::from(vec![
                            Span::styled(format!("{label:<11}"), Style::default().fg(MUTED)),
                            Span::styled(value, Style::default().fg(FG)),
                        ])
                    }),
            );
            let session_inner = padded(
                left[2],
                if left[2].width >= 36 { 2 } else { 1 },
                u16::from(spacious),
            );
            f.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: true }),
                session_inner,
            );
            album_cover(f, left[4], study);
            queue(f, cols[2], study);
        }
        Design::Desk => {
            let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(area);
            playback(f, rows[0], study);
            queue(f, rows[1], study);
        }
        Design::Monitor => {
            let rows = Layout::vertical([
                Constraint::Length(5),
                Constraint::Min(0),
                Constraint::Length(6.min(area.height.saturating_sub(8))),
            ])
            .split(area);
            playback(f, rows[0], study);
            let mut lines = vec![Line::from("Spectrum preview (simulated)").fg(MUTED)];
            if !matches!(study.scenario, Scenario::Empty | Scenario::Stale) && rows[1].height >= 4 {
                let levels: [u16; 24] = [
                    18, 35, 60, 83, 46, 64, 95, 73, 40, 55, 78, 100, 65, 36, 52, 82, 71, 42, 68,
                    91, 57, 38, 23, 12,
                ];
                let glyphs = ["  ", "▁▁", "▂▂", "▃▃", "▄▄", "▅▅", "▆▆", "▇▇", "██"];
                let height = rows[1].height.saturating_sub(2);
                let count = usize::from(rows[1].width / 3).min(levels.len());
                lines.push(Line::from(""));
                for y in (0..height).rev() {
                    let bars = levels
                        .iter()
                        .take(count)
                        .map(|level| {
                            let eighths = (level * height * 8 / 100).saturating_sub(y * 8).min(8);
                            format!("{} ", glyphs[usize::from(eighths)])
                        })
                        .collect::<String>();
                    lines.push(Line::from(bars).fg(ACCENT));
                }
            } else {
                lines.push(Line::from("Spectrum unavailable.").fg(MUTED));
            }
            f.render_widget(Paragraph::new(lines), rows[1]);
            queue(f, rows[2], study);
        }
    }
}

fn popup(f: &mut Frame, title: &str, lines: Vec<Line<'_>>, height: u16) {
    let bounds = f.area();
    let width = 68.min(bounds.width.saturating_sub(2));
    let height = height
        .saturating_add(2)
        .min(bounds.height.saturating_sub(2));
    let area = Rect::new(
        (bounds.width - width) / 2,
        (bounds.height - height) / 2,
        width,
        height,
    );
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines)
            .fg(FG)
            .bg(SESSION)
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .title(title)
                    .border_style(Style::default().fg(ACCENT))
                    .padding(ratatui::widgets::Padding::new(2, 2, 1, 1)),
            ),
        area,
    );
}

fn render(f: &mut Frame, study: &mut Study) {
    let bounds = f.area();
    study.cover_area = None;
    f.render_widget(Block::default().fg(FG).bg(BG), bounds);
    if bounds.width < 48 || bounds.height < 16 {
        let rows = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inset(bounds));
        f.render_widget(Block::default().bg(CHROME), rows[0]);
        f.render_widget(Block::default().bg(CHROME), rows[2]);
        text(f, rows[0], "JORIK\nOffline preview", ACCENT);
        playback(f, rows[1], study);
        text(
            f,
            rows[2],
            "a Add  Space Pause  q Quit\n48x16 for queue",
            MUTED,
        );
        return;
    }
    let desk = matches!(study.design, Design::Desk) && bounds.width >= 80 && bounds.height >= 24;
    let regions = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(u16::from(desk)),
        Constraint::Min(0),
        Constraint::Length(u16::from(desk)),
        Constraint::Length(2),
    ])
    .split(inset(bounds));
    f.render_widget(Block::default().bg(CHROME), regions[0]);
    f.render_widget(Block::default().bg(CHROME), regions[4]);
    let rows = [
        padded(regions[0], 2, 0),
        regions[2],
        padded(regions[4], 2, 0),
    ];
    let transport = if study.scenario == Scenario::Stale {
        "HTTP mode"
    } else {
        "Connected"
    };
    let top =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(15)]).split(row(rows[0], 0));
    text(
        f,
        top[0],
        if desk {
            "JORIK"
        } else {
            "JORIK   example.com   Guild 123456789012345678"
        },
        FG,
    );
    text(
        f,
        top[1],
        transport,
        if study.scenario == Scenario::Stale {
            WARN
        } else {
            MUTED
        },
    );
    heading(f, row(rows[0], 1), study.design);
    content(f, rows[1], study);
    let status = if !study.message.is_empty() {
        study.message.clone()
    } else {
        match study.scenario {
            Scenario::Stale => "! State 12s old. Last known data; r simulates refresh.".into(),
            Scenario::Error => "! Queue refresh failed. Cached rows retained; r retries.".into(),
            Scenario::Pending => "... Pause requested. Waiting for server confirmation.".into(),
            Scenario::Empty => "No music queued. Playback controls affect the shared bot.".into(),
            _ => format!(
                "Showing {}–{} of {} loaded tracks. Total unavailable.",
                study.offset + 1,
                (study.offset + study.page).min(study.tracks.len()),
                study.tracks.len()
            ),
        }
    };
    text(
        f,
        row(rows[2], 0),
        status,
        if matches!(study.scenario, Scenario::Stale | Scenario::Error) {
            WARN
        } else {
            MUTED
        },
    );
    text(
        f,
        row(rows[2], 1),
        if bounds.width < 64 {
            "a Add  Space Pause  ? Help  q Quit"
        } else {
            "a Add  Enter Details  Space Pause  : Actions  ? Help  q Quit"
        },
        FG,
    );
    match study.scenario {
        Scenario::Add => popup(
            f,
            " Add music ",
            vec![
                Line::from("Song name or URL. Server chooses the match.").fg(MUTED),
                Line::from(""),
                Line::from(format!("> {}_", study.draft)),
                Line::from(""),
                Line::from("Enter Add   Esc Cancel").fg(ACCENT),
                Line::from("Offline preview. Nothing will be sent.").fg(MUTED),
            ],
            9,
        ),
        Scenario::Clear => popup(
            f,
            " Clear the shared queue? ",
            vec![
                Line::from("Removes ALL upcoming tracks, including unloaded rows."),
                Line::from("Total count unknown. This cannot be undone.").fg(WARN),
                Line::from(""),
                Line::from("Esc Keep queue    y Clear").fg(ACCENT),
                Line::from("Offline preview. No music will be affected.").fg(MUTED),
            ],
            8,
        ),
        Scenario::Details if !matches!(study.design, Design::Queue) || bounds.width < 114 => {
            let (title, artist, album, length) = study
                .tracks
                .get(study.selected)
                .map(|track| {
                    (
                        track.title.as_str(),
                        track.artist.as_str(),
                        track.album.as_str(),
                        track.length.as_str(),
                    )
                })
                .unwrap_or(("No selection", "", "", ""));
            popup(
                f,
                " Selected track ",
                vec![
                    Line::from(title).bold(),
                    Line::from(artist).fg(MUTED),
                    Line::from(album).fg(MUTED),
                    Line::from(length).fg(MUTED),
                    Line::from(""),
                    Line::from("This track can currently be inspected only.").fg(MUTED),
                    Line::from("Esc Back to queue").fg(ACCENT),
                ],
                10,
            );
        }
        Scenario::Help => popup(
            f,
            " Design study controls ",
            vec![
                Line::from("1 Queue   2 Desk (selected direction)   3 Monitor"),
                Line::from("Arrow keys or j and k browse. PgUp, PgDn, Home, End."),
                Line::from("a adds music. Enter opens details. Space pauses."),
                Line::from("d Stale scenario; e Error; n Empty; r Reset"),
                Line::from("c Clear confirmation; : Actions; Esc Close; q Quit"),
                Line::from(""),
                Line::from("All actions are local simulations.").fg(WARN),
                Line::from("Production navigation 1-4 is unchanged.").fg(MUTED),
            ],
            12,
        ),
        _ => {}
    }
}

fn handle(study: &mut Study, key: event::KeyEvent) -> bool {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }
    if study.scenario == Scenario::Add {
        match key.code {
            KeyCode::Esc => {
                study.scenario = Scenario::Playing;
                study.draft.clear();
            }
            KeyCode::Enter if !study.draft.trim().is_empty() => {
                study.message =
                    format!("DEMO: submitted {}. No network request sent.", study.draft);
                study.draft.clear();
                study.scenario = Scenario::Playing;
            }
            KeyCode::Backspace => {
                study.draft.pop();
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !c.is_control()
                    && study.draft.chars().count() < 256 =>
            {
                study.draft.push(c);
            }
            _ => {}
        }
        return false;
    }
    if matches!(
        study.scenario,
        Scenario::Clear | Scenario::Details | Scenario::Help
    ) {
        if key.code == KeyCode::Esc {
            study.scenario = Scenario::Playing;
        } else if study.scenario == Scenario::Clear && key.code == KeyCode::Char('y') {
            study.scenario = Scenario::Empty;
            study.message = "DEMO: upcoming queue cleared. No network request sent.".into();
        }
        return false;
    }
    match key.code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('1') => study.design = Design::Queue,
        KeyCode::Char('2') => study.design = Design::Desk,
        KeyCode::Char('3') => study.design = Design::Monitor,
        KeyCode::Down | KeyCode::Char('j') => study.move_by(1),
        KeyCode::Up | KeyCode::Char('k') => study.move_by(-1),
        KeyCode::PageDown => study.move_by(study.page as isize),
        KeyCode::PageUp => study.move_by(-(study.page as isize)),
        KeyCode::Home => study.selected = 0,
        KeyCode::End => study.selected = study.tracks.len().saturating_sub(1),
        KeyCode::Char(' ' | 'p') => {
            study.scenario = if study.scenario == Scenario::Paused {
                Scenario::Playing
            } else {
                Scenario::Paused
            }
        }
        KeyCode::Char('a' | '/') => {
            study.scenario = Scenario::Add;
            study.message.clear();
        }
        KeyCode::Char('d') => {
            study.scenario = Scenario::Stale;
            study.message.clear();
        }
        KeyCode::Char('e') => {
            study.scenario = Scenario::Error;
            study.message.clear();
        }
        KeyCode::Char('n') => {
            study.scenario = Scenario::Empty;
            study.message.clear();
        }
        KeyCode::Char('r') => {
            study.scenario = Scenario::Playing;
            study.message.clear();
        }
        KeyCode::Char('c') => study.scenario = Scenario::Clear,
        KeyCode::Char('?') => study.scenario = Scenario::Help,
        KeyCode::Char(':') => {
            study.message =
                "DEMO: production palette has Shuffle, Repeat, Filters, 24/7, Account.".into()
        }
        KeyCode::Enter if study.scenario != Scenario::Empty => study.scenario = Scenario::Details,
        KeyCode::Esc => study.message.clear(),
        _ => {}
    }
    false
}

fn rgb(color: Color) -> [u8; 3] {
    if let Color::Rgb(r, g, b) = color {
        [r, g, b]
    } else {
        [232, 234, 237]
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut study = Study::new(&args);
    if args.render {
        let mut terminal = Terminal::new(TestBackend::new(args.width, args.height))?;
        terminal.draw(|frame| render(frame, &mut study))?;
        let buffer = terminal.backend().buffer();
        if args.json {
            let cells: Vec<_> = buffer
                .content
                .iter()
                .map(|cell| {
                    serde_json::json!({
                        "text": cell.symbol(), "fg": rgb(cell.fg), "bg": rgb(cell.bg),
                        "bold": cell.modifier.contains(Modifier::BOLD),
                    })
                })
                .collect();
            let image_layers = study
                .cover_area
                .map(|area| {
                    serde_json::json!({
                        "file": "m83-hurry-up-were-dreaming.jpg", "x": area.x, "y": area.y,
                        "width": area.width, "height": area.height,
                        "background": rgb(SIDEBAR),
                    })
                })
                .into_iter()
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::json!({"width": args.width, "height": args.height, "cells": cells, "image_layers": image_layers})
            );
        } else {
            for row in buffer.content.chunks(usize::from(args.width)) {
                println!(
                    "{}",
                    row.iter()
                        .map(|cell| cell.symbol())
                        .collect::<String>()
                        .trim_end()
                );
            }
        }
        return Ok(());
    }
    let mut terminal = ratatui::init();
    // Query only after entering the alternate screen, before reading key events.
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    picker.set_background_color(cover_background());
    study.cover_font_size = picker.font_size();
    study.cover = picker.new_resize_protocol(cover_image());
    // init installs Ratatui's panic hook; restore on normal exit and I/O errors.
    let result = (|| -> io::Result<()> {
        loop {
            terminal.draw(|frame| render(frame, &mut study))?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && handle(&mut study, key)
            {
                break;
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result?;
    Ok(())
}
